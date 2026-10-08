//! BLE GATT transport for an [`EngineRuntime`] (BLE.md).
//!
//! One service with two characteristics: `state` (read, notify) carries the
//! compact engine state, `event` (write, write-without-response) takes
//! compact client events. Up to [`CONNECTIONS_MAX`] centrals at once, each
//! served by its own worker; a worker with no connection advertises, and
//! only one advertises at a time, so the device is connectable exactly
//! while a slot is free.
//!
//! Security (BLE.md "Security"): both characteristics (and `state`'s CCCD)
//! need a link encrypted with a bond this box holds. LE Secure Connections,
//! Just Works (the box has no display or keys). New bonds are made only
//! while the pairing window is open, which the Kindle's settings panel opens
//! ([`BleCommand::Pair`], [`BLE_PAIRING_WINDOW_SECS`]); bonded devices
//! reconnect and re-encrypt at any time. Bonds are kept in flash through the
//! target's [`BondStore`] and handed back to the host at boot.
//!
//! Platform-free: the target hands in a bt-hci controller, the address and
//! the bond store.

use core::cell::{Cell, RefCell};

use embassy_futures::{
    join::{join3, join_array},
    select::{Either, Either4, select, select4},
};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer};
use trouble_host::{att::AttClient, prelude::*};

use extreme_traits::RawEngine;

use crate::{
    ble_settings::{AddrText, BleCommand, BleNote, BleStatus, BondRecord},
    config::{BLE_NAME, BLE_PAIRING_WINDOW_SECS, MAX_BLE_BONDS, MAX_BLE_CONNECTIONS},
    fmt::{Dbg, Disp},
    runtime::{EngineRuntime, StateMessage},
};

/// Concurrent BLE connections: the watch plus a phone or a debugging tool.
/// The target sizes its controller with it too.
pub const CONNECTIONS_MAX: usize = MAX_BLE_CONNECTIONS;

/// L2CAP channels: signal + ATT, as in the esp-hal coex example.
const L2CAP_CHANNELS_MAX: usize = 2;

/// The host's bond table: every stored bond, plus room for a fresh pairing
/// on each connection before it is stored (or refused and removed).
const HOST_BONDS: usize = MAX_BLE_BONDS + CONNECTIONS_MAX;

/// A peer that tried to resume a bond this box does not hold is
/// disconnected after this unless it pairs afresh, so it does not hold a
/// connection slot for nothing (Apple devices usually leave at once).
const STALE_GRACE: Duration = Duration::from_secs(10);

/// The service and its characteristics (BLE.md).
const SERVICE_UUID: u128 = 0xe4a1c000_6b2f_4f77_9a0d_3c5e7b9d1f20;
const STATE_UUID: u128 = 0xe4a1c001_6b2f_4f77_9a0d_3c5e7b9d1f20;
const EVENT_UUID: u128 = 0xe4a1c002_6b2f_4f77_9a0d_3c5e7b9d1f20;

/// Largest `state` value: one notification at the default ATT MTU.
const STATE_MAX: usize = 20;
/// Largest `event` value.
const EVENT_MAX: usize = 8;

/// Company identifier of the scan response's manufacturer data: 0xFFFF is
/// the Bluetooth SIG's value for internal use and testing, i.e. no company.
const COMPANY_NONE: u16 = 0xffff;

type StateValue = heapless09::Vec<u8, STATE_MAX>;
type EventValue = heapless09::Vec<u8, EVENT_MAX>;

/// `connections_max` sizes the per-client tables (CCCDs); the default of 1
/// would refuse the second central its subscription.
#[gatt_server(connections_max = CONNECTIONS_MAX)]
struct Server {
    race: RaceService,
}

/// Characteristic values live in one table shared by every connection
/// (only CCCDs are per client). `state` is stored on every notification
/// and captured afresh for every read, so a read is the latest state with
/// its times relative to the read, not to the last change.
///
/// `permissions(encrypted)` makes the host itself refuse any access on an
/// unencrypted link (reads, writes and the CCCD) before it reaches
/// [`serve`], which then picks the error that makes a central encrypt or
/// pair; [`serve`] also refuses an encrypted link that is not a stored bond.
#[gatt_service(uuid = SERVICE_UUID)]
struct RaceService {
    #[characteristic(uuid = STATE_UUID, read, notify, value = heapless09::Vec::new(), permissions(encrypted))]
    state: StateValue,
    #[characteristic(uuid = EVENT_UUID, write, write_without_response, value = heapless09::Vec::new(), permissions(encrypted))]
    event: EventValue,
}

/// Where the target keeps bonds across power cycles (the C6: its settings
/// partition). Slots are `0..MAX_BLE_BONDS`.
#[allow(async_fn_in_trait)] // only ever used on the target's own executor
pub trait BondStore {
    /// Every stored bond by slot.
    async fn load(&self) -> [Option<BondRecord>; MAX_BLE_BONDS];
    /// Stores `bond` in `slot`, replacing what was there; `false` on failure.
    async fn save(&self, slot: usize, bond: &BondRecord) -> bool;
    /// Deletes every bond; `false` on failure.
    async fn clear(&self) -> bool;
}

/// What the workers and the control loop share. One executor and no borrow
/// is held across an await, so `Cell`/`RefCell` do.
struct Shared {
    address: [u8; 6],
    /// The stored bonds by slot: the source of truth for who may connect.
    records: RefCell<[Option<BondRecord>; MAX_BLE_BONDS]>,
    /// When the pairing window closes; `None` while it is closed.
    pair_until: Cell<Option<Instant>>,
    note: Cell<BleNote>,
    /// Per worker: the window opened or closed, or bonds were deleted; the
    /// worker re-checks its connection.
    kick: [Signal<NoopRawMutex, ()>; CONNECTIONS_MAX],
}

impl Shared {
    fn pairing_open(&self) -> bool {
        self.pair_until.get().is_some_and(|t| Instant::now() < t)
    }

    /// The stored bond `identity` belongs to, if any.
    fn slot_of(&self, identity: &Identity) -> Option<usize> {
        self.records
            .borrow()
            .iter()
            .position(|r| r.is_some_and(|r| record_identity(&r).match_identity(identity)))
    }

    /// Whether `bond` (as the host used it to encrypt) is one of the stored
    /// bonds, key included.
    fn holds(&self, bond: &BondInformation) -> bool {
        self.slot_of(&bond.identity)
            .and_then(|slot| self.records.borrow()[slot])
            .is_some_and(|r| r.ltk == bond.ltk.to_le_bytes())
    }

    fn kick_all(&self) {
        for kick in &self.kick {
            kick.signal(());
        }
    }

    fn status(&self) -> BleStatus {
        let mut records: heapless::Vec<BondRecord, MAX_BLE_BONDS> =
            self.records.borrow().iter().flatten().copied().collect();
        records.sort_unstable_by_key(|r| r.seq);
        BleStatus {
            addr: self.address,
            pair_until: self.pair_until.get(),
            bonds: records.iter().map(|r| r.addr).collect(),
            note: self.note.get(),
        }
    }

    async fn publish<E: RawEngine>(&self, runtime: &EngineRuntime<E>) {
        runtime.set_ble_status(self.status()).await;
    }
}

/// Serves the BLE GATT transport on `controller`, forever, as the static
/// random `address` (BLE byte order; [`static_random_address`] makes one
/// from the chip's MAC, so every unit has its own), with bonds kept in
/// `store`.
///
/// [`static_random_address`]: crate::ble_settings::static_random_address
pub async fn serve_ble<C, E, S>(
    controller: C,
    runtime: &EngineRuntime<E>,
    address: [u8; 6],
    store: &S,
) -> !
where
    C: Controller,
    E: RawEngine,
    S: BondStore,
{
    info!("ble: address {} (static random)", Disp(&AddrText(address)));
    let shared = Shared {
        address,
        records: RefCell::new([None; MAX_BLE_BONDS]),
        pair_until: Cell::new(None),
        note: Cell::new(BleNote::None),
        kick: [const { Signal::new() }; CONNECTIONS_MAX],
    };
    let address = Address::random(address);

    let mut resources: HostResources<
        DefaultPacketPool,
        CONNECTIONS_MAX,
        L2CAP_CHANNELS_MAX,
        1,
        HOST_BONDS,
    > = HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        .set_io_capabilities(IoCapabilities::NoInputNoOutput)
        .build();
    let mut peripheral = stack.peripheral();
    let mut runner = stack.runner();

    let server = match Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: BLE_NAME,
        appearance: &appearance::UNKNOWN,
    })) {
        Ok(server) => server,
        Err(e) => {
            // only a GATT table too small for the service gets here
            error!("ble: cannot build the GATT server: {:?}", Dbg(&e));
            core::future::pending().await
        }
    };

    // Stored bonds back into the host, so a bonded device re-encrypts.
    let stored = store.load().await;
    for (slot, record) in stored.iter().enumerate() {
        let Some(record) = record else { continue };
        info!(
            "ble: bond slot {}: {} ({}), restored",
            slot,
            Disp(&AddrText(record.addr)),
            if record.irk.is_some() { "IRK" } else { "no IRK" }
        );
        if let Err(e) = stack.add_bond_information(record_bond(record)) {
            warn!("ble: bond slot {} not restored: {:?}", slot, Dbg(&e));
            continue;
        }
        shared.records.borrow_mut()[slot] = Some(*record);
    }
    info!(
        "ble: {} of {} bond(s) restored; pairing window closed (open it from the Kindle's settings)",
        shared.records.borrow().iter().flatten().count(),
        MAX_BLE_BONDS
    );
    shared.publish(runtime).await;

    let host = async {
        loop {
            if let Err(e) = runner.run().await {
                error!("ble: host stopped: {:?}; restarting", Dbg(&e));
            }
            Timer::after(Duration::from_secs(1)).await;
        }
    };

    let advertising = Mutex::<NoopRawMutex, _>::new(&mut peripheral);
    let unit = unit_id(address.addr.raw());
    let workers: [_; CONNECTIONS_MAX] = core::array::from_fn(|me| {
        worker(me, &shared, &stack, &advertising, &server, runtime, store, &unit)
    });

    join3(host, control(&shared, &stack, runtime, store), join_array(workers)).await;
    // all three loop forever
    unreachable!()
}

/// The pairing window and the Kindle's commands.
async fn control<C, E, S>(
    shared: &Shared,
    stack: &Stack<'_, C, DefaultPacketPool>,
    runtime: &EngineRuntime<E>,
    store: &S,
) where
    C: Controller,
    E: RawEngine,
    S: BondStore,
{
    let window = Duration::from_secs(BLE_PAIRING_WINDOW_SECS);
    loop {
        let closes = async {
            match shared.pair_until.get() {
                Some(t) => Timer::at(t).await,
                None => core::future::pending().await,
            }
        };
        match select(runtime.next_ble_command(), closes).await {
            Either::First(BleCommand::Pair) => {
                info!(
                    "ble: pairing window open for {} s (from the Kindle)",
                    BLE_PAIRING_WINDOW_SECS
                );
                shared.pair_until.set(Some(Instant::now() + window));
                shared.note.set(BleNote::None);
            }
            Either::First(BleCommand::Cancel) => {
                if shared.pair_until.take().is_none() {
                    continue;
                }
                info!("ble: pairing window closed (cancelled from the Kindle)");
                shared.note.set(BleNote::None);
            }
            Either::First(BleCommand::ForgetAll) => forget_all(shared, stack, store).await,
            Either::Second(()) => {
                info!(
                    "ble: pairing window closed after {} s, nobody paired",
                    BLE_PAIRING_WINDOW_SECS
                );
                shared.pair_until.set(None);
                shared.note.set(BleNote::Expired);
            }
        }
        shared.kick_all();
        shared.publish(runtime).await;
    }
}

/// Deletes every bond, from flash and from the host; the workers then drop
/// any connection that was trusted through one.
async fn forget_all<C: Controller, S: BondStore>(
    shared: &Shared,
    stack: &Stack<'_, C, DefaultPacketPool>,
    store: &S,
) {
    // The host's table may also hold a fresh pairing not yet stored.
    let mut identities: heapless::Vec<Identity, HOST_BONDS> = heapless::Vec::new();
    stack.with_bond_information(|bonds| {
        for bond in bonds {
            let _ = identities.push(bond.identity);
        }
    });
    for identity in identities {
        let _ = stack.remove_bond_information(identity);
    }
    let count = shared.records.borrow().iter().flatten().count();
    *shared.records.borrow_mut() = [None; MAX_BLE_BONDS];
    if store.clear().await {
        info!("ble: all {} bond(s) forgotten (from the Kindle)", count);
    } else {
        warn!("ble: forgetting bonds: flash erase failed; they may come back at the next boot");
    }
    shared.note.set(BleNote::Forgotten);
}

/// Worker `me`: advertise (while holding the advertising lock), accept,
/// serve the connection until it drops, repeat.
#[allow(clippy::too_many_arguments)]
async fn worker<'v, C, E, S>(
    me: usize,
    shared: &Shared,
    stack: &Stack<'_, C, DefaultPacketPool>,
    advertising: &Mutex<NoopRawMutex, &mut Peripheral<'v, C, DefaultPacketPool>>,
    server: &Server<'_>,
    runtime: &EngineRuntime<E>,
    store: &S,
    unit: &[u8; 6],
) where
    C: Controller,
    E: RawEngine,
    S: BondStore,
{
    loop {
        let conn = {
            let mut peripheral = advertising.lock().await;
            let Some(advertiser) = advertise(&mut peripheral, unit).await else {
                Timer::after(Duration::from_secs(1)).await;
                continue;
            };
            info!("ble: advertising as {} (worker {})", BLE_NAME, me);
            match advertiser.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    warn!("ble: accept failed: {:?}", Dbg(&e));
                    continue;
                }
            }
        };

        // Bonding only inside the window; outside it a pairing still
        // encrypts the link but makes no bond, and is then refused
        // (`serve`).
        let pairing = shared.pairing_open();
        let _ = conn.set_bondable(pairing);
        let slot = shared.slot_of(&conn.peer_identity());
        let peer = addr_of(&conn.peer_address());
        match slot {
            Some(slot) => info!(
                "ble: connection {} from {} (bond slot {} {}; pairing window {})",
                me,
                Disp(&AddrText(peer)),
                slot,
                Disp(&AddrText(addr_of(&conn.peer_identity().addr))),
                if pairing { "open" } else { "closed" }
            ),
            None => info!(
                "ble: connection {} from {} (not bonded; pairing window {})",
                me,
                Disp(&AddrText(peer)),
                if pairing { "open" } else { "closed" }
            ),
        }
        // a kick from before this connection is not about it
        shared.kick[me].reset();
        let raw = conn.clone();
        match conn.with_attribute_server(server) {
            Ok(conn) => serve(me, shared, stack, server, &conn, runtime, store).await,
            Err(e) => {
                warn!("ble: attribute server: {:?}; disconnecting", Dbg(&e));
                raw.disconnect();
            }
        }
    }
}

/// The unit's address as it is printed (most significant byte first): the
/// scan response's manufacturer data, so a central that cannot see BLE
/// addresses (CoreBluetooth hides them) can still tell one box from another
/// and match it to the address the Kindle shows.
fn unit_id(address: &[u8]) -> [u8; 6] {
    let mut id = [0u8; 6];
    id.copy_from_slice(address);
    id.reverse();
    id
}

/// Starts connectable advertising as [`BLE_NAME`] with the service UUID,
/// and `unit` in the scan response; `None` (logged) if the controller
/// refused.
async fn advertise<'v, C: Controller>(
    peripheral: &mut Peripheral<'v, C, DefaultPacketPool>,
    unit: &[u8; 6],
) -> Option<Advertiser<'v, C, DefaultPacketPool>> {
    let mut adv_data = [0; 31];
    let mut scan_data = [0; 31];
    let (Some(len), Some(scan_len)) = (
        adv_data_encode(&mut adv_data),
        scan_data_encode(unit, &mut scan_data),
    ) else {
        // the unit tests check both fit
        error!("ble: advertising data does not fit");
        return None;
    };
    match peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &adv_data[..len],
                scan_data: &scan_data[..scan_len],
            },
        )
        .await
    {
        Ok(advertiser) => Some(advertiser),
        Err(e) => {
            warn!("ble: advertise failed: {:?}", Dbg(&e));
            None
        }
    }
}

/// The advertising data: flags 3 + 128-bit UUID 18 + name 2 + 5 = 28 of the
/// 31 bytes.
fn adv_data_encode(out: &mut [u8; 31]) -> Option<usize> {
    AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteServiceUuids128(&[SERVICE_UUID.to_le_bytes()]),
            AdStructure::CompleteLocalName(BLE_NAME.as_bytes()),
        ],
        &mut out[..],
    )
    .ok()
}

/// The scan response: manufacturer data, company [`COMPANY_NONE`] then the
/// unit's address, 2 + 2 + 6 = 10 bytes.
fn scan_data_encode(unit: &[u8; 6], out: &mut [u8; 31]) -> Option<usize> {
    AdStructure::encode_slice(
        &[AdStructure::ManufacturerSpecificData {
            company_identifier: COMPANY_NONE,
            payload: unit,
        }],
        &mut out[..],
    )
    .ok()
}

/// One connection, until it drops: state changes out as notifications,
/// events in as writes, both only once the link is trusted.
///
/// Trusted = encrypted with a bond this box holds: either the central
/// resumed encryption with a stored bond's key (`Encrypted` with that
/// bond), or it just bonded inside the pairing window (`PairingComplete`).
/// An encrypted link is not enough on its own: outside the window a
/// central can still pair (without bonding) and encrypt, and may even
/// claim a bonded device's identity address while doing so.
async fn serve<C, E, S>(
    me: usize,
    shared: &Shared,
    stack: &Stack<'_, C, DefaultPacketPool>,
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    runtime: &EngineRuntime<E>,
    store: &S,
) where
    C: Controller,
    E: RawEngine,
    S: BondStore,
{
    let state = &server.race.state;
    let event = &server.race.event;
    let mut trusted = false;
    // set when the peer turned out to hold a bond this box does not
    let mut stale_deadline: Option<Instant> = None;

    // subscribe before reading the current state, so no change is missed
    let mut updates = match runtime.subscribe() {
        Ok(updates) => updates,
        Err(e) => {
            error!(
                "ble: connection {}: cannot subscribe to state updates: {:?}; disconnecting",
                me,
                Dbg(&e)
            );
            conn.raw().disconnect();
            wait_disconnect(conn).await;
            return;
        }
    };

    if let Ok(message) = runtime.current_state().await
        && let Some(value) = state_value(&message)
        && let Err(e) = conn.set(state, &value)
    {
        // a central that subscribes later reads it from the table
        warn!("ble: cannot set state: {:?}", Dbg(&e));
    }

    loop {
        let stale = async {
            match stale_deadline {
                Some(t) => Timer::at(t).await,
                None => core::future::pending().await,
            }
        };
        let gatt = match select4(
            conn.next(),
            updates.next_message_pure(),
            shared.kick[me].wait(),
            stale,
        )
        .await
        {
            Either4::First(gatt) => gatt,
            Either4::Second(message) => {
                if trusted && let Some(value) = state_value(&message) {
                    // stored too, so reads stay current
                    if let Err(e) = state.notify(conn, &value, true).await {
                        debug!("ble: connection {}: notify failed: {:?}", me, Dbg(&e));
                    }
                }
                continue;
            }
            Either4::Third(()) => {
                // the window opened or closed, or bonds were deleted
                let _ = conn.raw().set_bondable(shared.pairing_open());
                if trusted && shared.slot_of(&conn.raw().peer_identity()).is_none() {
                    info!("ble: connection {}: its bond was deleted; disconnecting", me);
                    trusted = false;
                    conn.raw().disconnect();
                }
                continue;
            }
            Either4::Fourth(()) => {
                warn!(
                    "ble: connection {}: stale peer did not pair afresh within {} s; disconnecting",
                    me,
                    STALE_GRACE.as_secs()
                );
                stale_deadline = None;
                conn.raw().disconnect();
                continue;
            }
        };

        match gatt {
            GattConnectionEvent::Disconnected { reason } => {
                info!("ble: connection {} closed: {:?}", me, Dbg(&reason));
                return;
            }
            GattConnectionEvent::RequestConnectionParams(request) => {
                if let Err(e) = request.accept(None, stack).await {
                    debug!("ble: connection parameters: {:?}", Dbg(&e));
                }
            }
            // A bonded central resumed encryption with its key.
            GattConnectionEvent::Encrypted { bond: Some(bond), .. } => {
                trusted = shared.holds(&bond);
                if trusted {
                    info!(
                        "ble: connection {}: encrypted with the bond of {}",
                        me,
                        Disp(&AddrText(addr_of(&bond.identity.addr)))
                    );
                } else {
                    warn!(
                        "ble: connection {}: encrypted with a key that is not a stored bond; untrusted",
                        me
                    );
                }
            }
            // Fresh pairings report `PairingComplete` as well (below).
            GattConnectionEvent::Encrypted { bond: None, .. } => {}
            GattConnectionEvent::PairingComplete { bond, .. } => {
                let peer = conn.raw().peer_identity();
                match bond {
                    Some(bond) if shared.pairing_open() => {
                        enrol(shared, stack, store, &bond).await;
                        trusted = true;
                        stale_deadline = None;
                        shared.publish(runtime).await;
                    }
                    bond => {
                        let why = if bond.is_none() {
                            "the central did not bond"
                        } else {
                            "pairing window closed"
                        };
                        warn!(
                            "ble: connection {}: pairing from {} rejected ({}); disconnecting",
                            me,
                            Disp(&AddrText(addr_of(&peer.addr))),
                            why
                        );
                        // The host took the new key into its table, over a
                        // stored bond of the same identity if there was
                        // one: put that back, or drop the new entry.
                        let stored = shared
                            .slot_of(&peer)
                            .and_then(|slot| shared.records.borrow()[slot]);
                        match stored {
                            Some(record) => {
                                let _ = stack.add_bond_information(record_bond(&record));
                            }
                            None => {
                                let _ = stack.remove_bond_information(peer);
                            }
                        }
                        trusted = false;
                        conn.raw().disconnect();
                        shared.note.set(if shared.pairing_open() {
                            BleNote::Failed
                        } else {
                            BleNote::Rejected
                        });
                        shared.publish(runtime).await;
                    }
                }
            }
            GattConnectionEvent::PairingFailed(e) => {
                warn!("ble: connection {}: pairing failed: {:?}", me, Dbg(&e));
                if shared.pairing_open() {
                    shared.note.set(BleNote::Failed);
                    shared.publish(runtime).await;
                }
            }
            // The central tried to resume a bond this box does not hold
            // (forgotten, or the settings were erased): the host refused it
            // and kept the link, so it could pair afresh over it.
            GattConnectionEvent::LongTermKeyMissing => {
                warn!(
                    "ble: connection {}: {} has an old pairing with this box (forget {} on that device, then pair again); disconnecting in {} s unless it pairs afresh",
                    me,
                    Disp(&AddrText(addr_of(&conn.raw().peer_address()))),
                    BLE_NAME,
                    STALE_GRACE.as_secs()
                );
                stale_deadline = Some(Instant::now() + STALE_GRACE);
                shared.note.set(BleNote::StaleBond);
                shared.publish(runtime).await;
            }
            GattConnectionEvent::Gatt { event: request } => {
                let reply = match request {
                    // The host refused an access on the unencrypted link.
                    // Insufficient Authentication makes a central with a
                    // bond encrypt, and one without pair, then retry (what
                    // CoreBluetooth does by itself) -- but only a bonded
                    // device or the open window can get anywhere with
                    // that. Anyone else gets Insufficient Authorization,
                    // so it does not pair in vain.
                    GattEvent::NotAllowed(refused) => {
                        let bonded = shared.slot_of(&conn.raw().peer_identity()).is_some();
                        let code = if bonded || shared.pairing_open() {
                            AttErrorCode::INSUFFICIENT_AUTHENTICATION
                        } else {
                            AttErrorCode::INSUFFICIENT_AUTHORISATION
                        };
                        info!(
                            "ble: connection {}: unencrypted access refused ({})",
                            me,
                            if bonded {
                                "bonded: encrypt"
                            } else if shared.pairing_open() {
                                "pairing window open: pair"
                            } else {
                                "not bonded, pairing window closed"
                            }
                        );
                        if matches!(refused.payload().incoming(), AttClient::Command(_)) {
                            // a write command gets no reply, not even an
                            // error: dropped
                            let _ = refused.into_payload();
                            continue;
                        }
                        refused.reject(code)
                    }
                    GattEvent::Write(write) if write.handle() == event.handle => {
                        // a write command (write without response) gets no
                        // reply, not even an error
                        let command = matches!(write.payload().incoming(), AttClient::Command(_));
                        if !trusted {
                            warn!("ble: connection {}: event refused: link not bonded", me);
                            if command {
                                write.accept_unprocessed()
                            } else {
                                write.reject(AttErrorCode::INSUFFICIENT_AUTHORISATION)
                            }
                        } else {
                            let payload = write.with_data(|offset, data| {
                                if offset == 0 {
                                    EventValue::from_slice(data).ok()
                                } else {
                                    None
                                }
                            });
                            let accepted = match &payload {
                                Some(payload) => runtime.compact_event(payload).await.is_ok(),
                                None => false,
                            };
                            if accepted {
                                debug!("ble: connection {}: event {:?}", me, Dbg(&payload));
                                write.accept()
                            } else {
                                warn!("ble: connection {}: event refused: {:?}", me, Dbg(&payload));
                                if command {
                                    write.accept_unprocessed()
                                } else {
                                    write.reject(AttErrorCode::VALUE_NOT_ALLOWED)
                                }
                            }
                        }
                    }
                    // Subscribing is reading too.
                    GattEvent::Write(write) if Some(write.handle()) == state.cccd_handle => {
                        if trusted {
                            write.accept()
                        } else {
                            warn!("ble: connection {}: subscription refused: link not bonded", me);
                            write.reject(AttErrorCode::INSUFFICIENT_AUTHORISATION)
                        }
                    }
                    // `accept` builds the reply from the table, so refresh
                    // it first: its times are relative to when it was
                    // captured, and the state may not have changed for a
                    // while (no GPS)
                    GattEvent::Read(read) if read.handle() == state.handle => {
                        if !trusted {
                            warn!("ble: connection {}: state read refused: link not bonded", me);
                            read.reject(AttErrorCode::INSUFFICIENT_AUTHORISATION)
                        } else {
                            if let Ok(message) = runtime.current_state().await
                                && let Some(value) = state_value(&message)
                                && let Err(e) = conn.set(state, &value)
                            {
                                warn!("ble: cannot set state: {:?}", Dbg(&e));
                            }
                            read.accept()
                        }
                    }
                    other => other.accept(),
                };
                match reply {
                    Ok(reply) => reply.send().await,
                    Err(e) => warn!("ble: connection {}: reply failed: {:?}", me, Dbg(&e)),
                }
            }
            _ => {}
        }
    }
}

/// A fresh bond inside the pairing window: into a slot (the device's own if
/// it was bonded before, else a free one, else the oldest bond's), into
/// flash, and the window closes (one device per "Pair watch").
async fn enrol<C: Controller, S: BondStore>(
    shared: &Shared,
    stack: &Stack<'_, C, DefaultPacketPool>,
    store: &S,
    bond: &BondInformation,
) {
    let identity = bond.identity;
    let existing = shared.slot_of(&identity);
    let (slot, evicted) = {
        let records = shared.records.borrow();
        match existing.or_else(|| records.iter().position(Option::is_none)) {
            Some(slot) => (slot, None),
            // full: the oldest bond goes (every slot is taken here)
            None => records
                .iter()
                .enumerate()
                .filter_map(|(slot, r)| r.map(|r| (slot, Some(r))))
                .min_by_key(|(_, r)| r.map(|r| r.seq))
                .unwrap_or((0, None)),
        }
    };
    if let Some(oldest) = evicted {
        info!(
            "ble: all {} bond slots taken: replacing the oldest, {}",
            MAX_BLE_BONDS,
            Disp(&AddrText(oldest.addr))
        );
        let _ = stack.remove_bond_information(record_identity(&oldest));
        // a connection trusted through it is dropped
        shared.records.borrow_mut()[slot] = None;
        shared.kick_all();
    }
    let seq = shared
        .records
        .borrow()
        .iter()
        .flatten()
        .map(|r| r.seq.wrapping_add(1))
        .max()
        .unwrap_or(1)
        .max(evicted.map_or(0, |r| r.seq.wrapping_add(1)));
    let record = record_of(bond, seq);
    let saved = store.save(slot, &record).await;
    shared.records.borrow_mut()[slot] = Some(record);
    shared.pair_until.set(None);
    if saved {
        info!(
            "ble: pairing accepted: bonded with {} ({}), bond slot {}; pairing window closed",
            Disp(&AddrText(record.addr)),
            if record.irk.is_some() { "IRK" } else { "no IRK" },
            slot
        );
        shared.note.set(BleNote::Paired(record.addr));
    } else {
        warn!(
            "ble: pairing accepted: bonded with {}, but the bond was NOT saved to flash (lost at power off); pairing window closed",
            Disp(&AddrText(record.addr))
        );
        shared.note.set(BleNote::PairedNotSaved(record.addr));
    }
    // the other connection learns the window closed
    shared.kick_all();
}

/// The stored form of a bond made by the host.
fn record_of(bond: &BondInformation, seq: u32) -> BondRecord {
    BondRecord {
        // 2 and 3 are the resolved forms of public and random
        addr_kind: bond.identity.addr.kind.as_raw() & 1,
        addr: addr_of(&bond.identity.addr),
        irk: bond.identity.irk.map(|irk| irk.to_le_bytes()),
        ltk: bond.ltk.to_le_bytes(),
        authenticated: bond.security_level == SecurityLevel::EncryptedAuthenticated,
        seq,
    }
}

fn record_identity(record: &BondRecord) -> Identity {
    Identity {
        addr: Address {
            kind: AddrKind::new(record.addr_kind),
            addr: BdAddr::new(record.addr),
        },
        irk: record.irk.and_then(IdentityResolvingKey::from_le_bytes),
    }
}

/// The host's form of a stored bond.
fn record_bond(record: &BondRecord) -> BondInformation {
    let level = if record.authenticated {
        SecurityLevel::EncryptedAuthenticated
    } else {
        SecurityLevel::Encrypted
    };
    BondInformation::new(
        record_identity(record),
        LongTermKey::from_le_bytes(record.ltk),
        level,
        true,
    )
}

fn addr_of(address: &Address) -> [u8; 6] {
    let mut out = [0u8; 6];
    out.copy_from_slice(address.addr.raw());
    out
}

/// Waits for a connection we asked to close to report the disconnect.
async fn wait_disconnect(conn: &GattConnection<'_, '_, DefaultPacketPool>) {
    loop {
        if let GattConnectionEvent::Disconnected { .. } = conn.next().await {
            return;
        }
    }
}

/// The `state` characteristic value of a state message.
fn state_value(message: &StateMessage) -> Option<StateValue> {
    let value = StateValue::from_slice(&message.compact).ok();
    if value.is_none() {
        error!(
            "ble: {} compact state is {} bytes, more than {}",
            message.kind,
            message.compact.len(),
            STATE_MAX
        );
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BLE.md's UUIDs, byte for byte as written there.
    #[test]
    fn uuids_match_the_protocol() {
        let tail = [
            0x6b, 0x2f, 0x4f, 0x77, 0x9a, 0x0d, 0x3c, 0x5e, 0x7b, 0x9d, 0x1f, 0x20,
        ];
        for (uuid, n) in [(SERVICE_UUID, 0x00), (STATE_UUID, 0x01), (EVENT_UUID, 0x02)] {
            let bytes = uuid.to_be_bytes();
            assert_eq!(bytes[..4], [0xe4, 0xa1, 0xc0, n]);
            assert_eq!(bytes[4..], tail);
        }
    }

    /// The advertising data fits one legacy advertisement.
    #[test]
    fn advertising_data_fits() {
        let mut adv_data = [0; 31];
        let len = adv_data_encode(&mut adv_data).unwrap();
        assert_eq!(len, 3 + 18 + 2 + BLE_NAME.len());
        // the UUID goes over the air little-endian
        assert_eq!(&adv_data[5..21], &SERVICE_UUID.to_le_bytes());
        assert_eq!(adv_data[20], 0xe4);
    }

    /// The scan response carries the unit's address as printed, after the
    /// "no company" identifier (little-endian on air).
    #[test]
    fn scan_response_carries_the_unit() {
        let unit = unit_id(&[0x03, 0x02, 0x01, 0x04, 0x32, 0xd4]);
        assert_eq!(unit, [0xd4, 0x32, 0x04, 0x01, 0x02, 0x03]);
        let mut scan_data = [0; 31];
        let len = scan_data_encode(&unit, &mut scan_data).unwrap();
        assert_eq!(
            &scan_data[..len],
            &[9, 0xff, 0xff, 0xff, 0xd4, 0x32, 0x04, 0x01, 0x02, 0x03]
        );
    }

    /// A bond goes to flash and back into the host unchanged, and is
    /// recognised by its identity, key included. (Recognising a resolvable
    /// private address through the IRK is trouble-host's `match_identity`.)
    #[test]
    fn bond_round_trip() {
        let irk = IdentityResolvingKey::from_le_bytes([0x5a; 16]).unwrap();
        let identity = Identity {
            addr: Address::random([1, 2, 3, 4, 5, 0xc6]),
            irk: Some(irk),
        };
        let bond = BondInformation::new(
            identity,
            LongTermKey::from_le_bytes([9; 16]),
            SecurityLevel::Encrypted,
            true,
        );
        let record = record_of(&bond, 7);
        assert_eq!(record.seq, 7);
        assert_eq!(record_bond(&record), bond);

        let shared = Shared {
            address: [0; 6],
            records: RefCell::new([None; MAX_BLE_BONDS]),
            pair_until: Cell::new(None),
            note: Cell::new(BleNote::None),
            kick: [const { Signal::new() }; CONNECTIONS_MAX],
        };
        shared.records.borrow_mut()[2] = Some(record);
        assert_eq!(shared.slot_of(&identity), Some(2));
        assert!(shared.holds(&bond));
        // the same identity with another key is not the stored bond
        let mut other = bond.clone();
        other.ltk = LongTermKey::from_le_bytes([8; 16]);
        assert!(!shared.holds(&other));
        // an unrelated device is not
        let stranger = Identity {
            addr: Address::random([7, 7, 7, 7, 7, 0xc7]),
            irk: None,
        };
        assert_eq!(shared.slot_of(&stranger), None);
    }
}
