//! BLE GATT transport for a [`Host`] (BLE.md).
//!
//! One service with three characteristics: `state` (read, notify) carries
//! the host's envelope, `event` (write, write-without-response) takes host
//! events, both postcard (README "Host protocol"), and `ui-psm` (read) names
//! the L2CAP channel that serves watch bundles ([`UI_PSM`], BLE.md "Watch
//! bundles"). Up to [`CONNECTIONS_MAX`] centrals at once, each
//! served by its own worker; a worker with no connection advertises, and
//! only one advertises at a time, so the device is connectable exactly
//! while a slot is free.
//!
//! GATT caching (BLE.md "GATT caching"): the GATT service has Service
//! Changed, Client Supported Features and Database Hash. A bonded client's
//! subscriptions and features are kept with its bond, and when the database
//! is not the one it last saw (a firmware update) it is told: Service Changed
//! if it subscribed, Database Out Of Sync errors if it uses robust caching.
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
    select::{Either, Either3, Either4, select, select3, select4},
};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer};
use trouble_host::{
    att::{AttClient, AttReq},
    prelude::*,
};

use extreme_protocol::ENVELOPE_MAX;

use crate::{
    ble_settings::{AddrText, BleCommand, BleNote, BleStatus, BondRecord, GattClientRecord},
    bulk::{self, Transfer},
    config::{BLE_NAME, BLE_PAIRING_WINDOW_SECS, MAX_BLE_BONDS, MAX_BLE_CONNECTIONS},
    fmt::{Dbg, Disp, Hex},
    host::{Host, Refused, StateMessage, Transport},
};

/// Concurrent BLE connections: the watch plus a phone or a debugging tool.
/// The target sizes its controller with it too.
pub const CONNECTIONS_MAX: usize = MAX_BLE_CONNECTIONS;

/// L2CAP connection-oriented channels (signalling and ATT are fixed
/// channels and take none): the bulk channel on each connection, plus one
/// each so a second request on a connection can be refused rather than sit
/// in the only slot.
const L2CAP_CHANNELS_MAX: usize = 2 * CONNECTIONS_MAX;

/// The bulk channel's LE PSM (BLE.md "Watch bundles"), from the dynamic
/// range 0x0080..=0x00FF; published in `ui-psm`.
pub const UI_PSM: u16 = 0x0081;

/// Largest SDU this side takes on the bulk channel: a `GetAsset` is at most
/// 13 bytes, and an L2CAP MTU is at least 23.
const BULK_RX_MTU: u16 = 23;
/// Credits this side grants on the bulk channel: requests come one at a
/// time, each a single PDU; each is given back as it is read.
const BULK_RX_CREDITS: u16 = 2;
/// A bulk channel with no request for this long is closed; the client opens
/// another when it needs one.
const BULK_IDLE: Duration = Duration::from_secs(30);

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
const UI_PSM_UUID: u128 = 0xe4a1c003_6b2f_4f77_9a0d_3c5e7b9d1f20;

/// Largest `state` value: an `Envelope` at its largest. A notification
/// carries at most ATT MTU - 3 bytes, so a central must exchange a larger
/// ATT MTU than the default 23 (BLE.md "State"); CoreBluetooth always does,
/// and the host accepts up to its packet pool's MTU less 4, 247. A value
/// that does not fit the link's MTU is not notified (logged); a read gets
/// it whole, in parts (Read Blob).
const STATE_MAX: usize = ENVELOPE_MAX;
/// Largest `event` value: a `HostEvent`; a write longer than this is
/// refused. Today's largest is 24 bytes (a Race bump with `seconds` and
/// `timestamp` at their longest, plus the package id), more than the
/// default MTU's 20: a central must exchange a larger MTU (BLE.md).
const EVENT_MAX: usize = 64;

/// Company identifier of the scan response's manufacturer data: 0xFFFF is
/// the Bluetooth SIG's value for internal use and testing, i.e. no company.
const COMPANY_NONE: u16 = 0xffff;

type StateValue = heapless09::Vec<u8, STATE_MAX>;
type EventValue = heapless09::Vec<u8, EVENT_MAX>;

/// The attribute table: GAP, the GATT service with its caching
/// characteristics, and ours.
const ATT_TABLE_SIZE: usize =
    GAP_SERVICE_ATTRIBUTE_COUNT + GATT_CACHING_ATTRIBUTE_COUNT + RaceService::ATTRIBUTE_COUNT;

/// `connections_max` sizes the per-client tables (CCCDs); the default of 1
/// would refuse the second central its subscription. Built with
/// [`GapConfig::build_with_caching`] and `new` (not `new_with_config`), so
/// the table is sized here.
#[gatt_server(connections_max = CONNECTIONS_MAX, attribute_table_size = ATT_TABLE_SIZE)]
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
    /// The bulk channel's LE PSM, u16 little-endian.
    #[characteristic(uuid = UI_PSM_UUID, read, value = UI_PSM, permissions(encrypted))]
    ui_psm: u16,
}

/// Where the target keeps bonds across power cycles (the C6: its settings
/// partition). Slots are `0..MAX_BLE_BONDS`.
#[allow(async_fn_in_trait)] // only ever used on the target's own executor
pub trait BondStore {
    /// Every stored bond by slot.
    async fn load(&self) -> [Option<BondRecord>; MAX_BLE_BONDS];
    /// Stores `bond` in `slot`, replacing what was there; `false` on failure.
    async fn save(&self, slot: usize, bond: &BondRecord) -> bool;
    /// Deletes every bond and its GATT client record; `false` on failure.
    async fn clear(&self) -> bool;
    /// Every stored GATT client record by bond slot, and the database hash
    /// the bonded clients were last served (BLE.md "GATT caching").
    async fn load_gatt(&self) -> ([Option<GattClientRecord>; MAX_BLE_BONDS], Option<[u8; 16]>);
    /// Stores the GATT client record of bond `slot`; `false` on failure.
    async fn save_gatt(&self, slot: usize, record: &GattClientRecord) -> bool;
    /// Stores the database hash now served; `false` on failure.
    async fn save_gatt_hash(&self, hash: &[u8; 16]) -> bool;
}

/// What the target can say about its memory, for the bulk channel's log.
#[derive(Clone, Copy)]
pub struct Diagnostics {
    /// Free heap now, bytes.
    pub heap_free: fn() -> usize,
    /// Logs the heap and stack, labelled.
    pub log_memory: fn(&str),
}

/// What the workers and the control loop share. One executor and no borrow
/// is held across an await, so `Cell`/`RefCell` do.
struct Shared {
    address: [u8; 6],
    /// The stored bonds by slot: the source of truth for who may connect.
    records: RefCell<[Option<BondRecord>; MAX_BLE_BONDS]>,
    /// Each bond's GATT client state, as stored (`None`: nothing stored,
    /// which reads as nothing configured and change-unaware).
    gatt: RefCell<[Option<GattClientRecord>; MAX_BLE_BONDS]>,
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

    async fn publish(&self, runtime: &Host) {
        runtime.set_ble_status(self.status()).await;
    }
}

/// Serves the BLE GATT transport on `controller`, forever, as the static
/// random `address` (BLE byte order; [`static_random_address`] makes one
/// from the chip's MAC, so every unit has its own), with bonds kept in
/// `store`.
///
/// [`static_random_address`]: crate::ble_settings::static_random_address
pub async fn serve_ble<C, S>(
    controller: C,
    runtime: &Host,
    address: [u8; 6],
    store: &S,
    diagnostics: Diagnostics,
) -> !
where
    C: Controller,
    S: BondStore,
{
    info!("ble: address {} (static random)", Disp(&AddrText(address)));
    let shared = Shared {
        address,
        records: RefCell::new([None; MAX_BLE_BONDS]),
        gatt: RefCell::new([None; MAX_BLE_BONDS]),
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
        .register_l2cap_spsm(UI_PSM)
        .build();
    let mut peripheral = stack.peripheral();
    let mut runner = stack.runner();

    let mut table: AttributeTable<'_, NoopRawMutex, ATT_TABLE_SIZE> = AttributeTable::new();
    let caching = match GapConfig::Peripheral(PeripheralConfig {
        name: BLE_NAME,
        appearance: &appearance::UNKNOWN,
    })
    .build_with_caching(&mut table)
    {
        Ok(caching) => caching,
        Err(e) => {
            // only a GATT table too small for the services gets here
            error!("ble: cannot build the GATT server: {:?}", Dbg(&e));
            core::future::pending().await
        }
    };
    let server = Server::new(table);
    let db_hash = match caching.set_database_hash(&server) {
        Ok(hash) => hash,
        Err(e) => {
            error!("ble: cannot set the database hash: {:?}", Dbg(&e));
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
    restore_gatt(&shared, store, db_hash).await;
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
    let context = Context {
        shared: &shared,
        stack: &stack,
        server: &server,
        caching: &caching,
        runtime,
        store,
        diagnostics,
    };
    let workers: [_; CONNECTIONS_MAX] =
        core::array::from_fn(|me| worker(me, &context, &advertising, &unit));

    join3(host, control(&shared, &stack, runtime, store), join_array(workers)).await;
    // all three loop forever
    unreachable!()
}

/// What every connection's worker uses.
struct Context<'a, 'd, C: Controller, S> {
    shared: &'a Shared,
    stack: &'a Stack<'d, C, DefaultPacketPool>,
    /// Its values are all `'static` (the name, the services' storage).
    server: &'a Server<'static>,
    caching: &'a GattCaching,
    runtime: &'a Host,
    store: &'a S,
    diagnostics: Diagnostics,
}

/// The bonded clients' GATT state from flash. If the database is not the
/// one last served (a firmware update changed the table), every bonded
/// client is change-unaware until it is told (BLE.md "GATT caching"); a
/// bond with no record yet (made before GATT caching) is one too.
async fn restore_gatt<S: BondStore>(shared: &Shared, store: &S, db_hash: [u8; 16]) {
    let (records, served) = store.load_gatt().await;
    let changed = served != Some(db_hash);
    info!(
        "ble: database hash {}{}",
        Disp(&Hex(&db_hash)),
        match served {
            None => " (none stored: first boot with GATT caching)",
            Some(_) if changed => " (changed since last served: bonded clients are change-unaware)",
            Some(_) => " (unchanged)",
        }
    );
    for slot in 0..MAX_BLE_BONDS {
        if shared.records.borrow()[slot].is_none() {
            continue;
        }
        let mut record = records[slot].unwrap_or_default();
        if changed && record.aware {
            record.aware = false;
            if !store.save_gatt(slot, &record).await {
                warn!("ble: bond slot {}: GATT record not saved", slot);
            }
        }
        info!(
            "ble: bond slot {}: GATT client {}, Service Changed CCCD {:04x}, features {:02x}",
            slot,
            if record.aware { "change-aware" } else { "change-unaware" },
            record.service_changed,
            record.features
        );
        shared.gatt.borrow_mut()[slot] = Some(record);
    }
    // after the records, so a power cut in between leaves the old hash and
    // the next boot marks them again
    if changed && !store.save_gatt_hash(&db_hash).await {
        warn!("ble: database hash not saved");
    }
}

/// The pairing window and the Kindle's commands.
async fn control<C, S>(
    shared: &Shared,
    stack: &Stack<'_, C, DefaultPacketPool>,
    runtime: &Host,
    store: &S,
) where
    C: Controller,
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
    *shared.gatt.borrow_mut() = [None; MAX_BLE_BONDS];
    if store.clear().await {
        info!("ble: all {} bond(s) forgotten (from the Kindle)", count);
    } else {
        warn!("ble: forgetting bonds: flash erase failed; they may come back at the next boot");
    }
    shared.note.set(BleNote::Forgotten);
}

/// Worker `me`: advertise (while holding the advertising lock), accept,
/// serve the connection until it drops, repeat.
async fn worker<'v, C, S>(
    me: usize,
    ctx: &Context<'_, '_, C, S>,
    advertising: &Mutex<NoopRawMutex, &mut Peripheral<'v, C, DefaultPacketPool>>,
    unit: &[u8; 6],
) where
    C: Controller,
    S: BondStore,
{
    let shared = ctx.shared;
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
        match conn.with_attribute_server(ctx.server) {
            Ok(conn) => serve(me, ctx, &conn).await,
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

/// What the two halves of a connection ([`serve_gatt`], [`serve_bulk`])
/// share.
struct Link {
    /// Encrypted with a bond this box holds (see [`serve`]).
    trusted: Cell<bool>,
    /// The bond slot of a trusted link.
    slot: Cell<Option<usize>>,
    /// The client knows the database as it is (BLE.md "GATT caching").
    /// Only a trusted bonded client can be change-unaware.
    aware: Cell<bool>,
}

/// One connection, until it drops: GATT ([`serve_gatt`]) and the bulk
/// channel ([`serve_bulk`]) side by side, both only once the link is
/// trusted.
///
/// Trusted = encrypted with a bond this box holds: either the central
/// resumed encryption with a stored bond's key (`Encrypted` with that
/// bond), or it just bonded inside the pairing window (`PairingComplete`).
/// An encrypted link is not enough on its own: outside the window a
/// central can still pair (without bonding) and encrypt, and may even
/// claim a bonded device's identity address while doing so.
async fn serve<C, S>(
    me: usize,
    ctx: &Context<'_, '_, C, S>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
) where
    C: Controller,
    S: BondStore,
{
    let link = Link {
        trusted: Cell::new(false),
        slot: Cell::new(None),
        aware: Cell::new(true),
    };
    // the bulk half never ends by itself; the connection ends with GATT's
    select(serve_gatt(me, ctx, conn, &link), serve_bulk(me, ctx, conn.raw(), &link)).await;
}

/// The GATT half of a connection, until it drops: state changes out as
/// notifications, events in as writes.
async fn serve_gatt<C, S>(
    me: usize,
    ctx: &Context<'_, '_, C, S>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    link: &Link,
) where
    C: Controller,
    S: BondStore,
{
    let (shared, stack, server, runtime, store) =
        (ctx.shared, ctx.stack, ctx.server, ctx.runtime, ctx.store);
    let state = &server.race.state;
    let event = &server.race.event;
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

    if let Some(value) = state_value(runtime, &runtime.current_state().await)
        && let Err(e) = conn.set(state, &value)
    {
        // a central that subscribes later reads it from the table
        warn!("ble: cannot set state: {:?}", Dbg(&e));
    }
    // warned once per connection: a central that kept the default MTU
    let mut warned_mtu = false;

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
                if link.trusted.get() && let Some(value) = state_value(runtime, &message) {
                    // A notification carries at most ATT MTU - 3 bytes and
                    // the host would cut a longer one short, which no
                    // central could decode: not sent (stored for reads,
                    // which come in parts), logged once per connection.
                    let room = conn.raw().att_mtu() as usize - 3;
                    if value.len() > room {
                        if !warned_mtu {
                            warned_mtu = true;
                            warn!(
                                "ble: connection {}: {} state is {} bytes but the ATT MTU allows {}: not notified; the central must exchange a larger MTU",
                                me,
                                message.name,
                                value.len(),
                                room
                            );
                        }
                        let _ = conn.set(state, &value);
                    } else if let Err(e) = state.notify(conn, &value, true).await {
                        // stored too, so reads stay current
                        debug!("ble: connection {}: notify failed: {:?}", me, Dbg(&e));
                    }
                }
                continue;
            }
            Either4::Third(()) => {
                // the window opened or closed, or bonds were deleted
                let _ = conn.raw().set_bondable(shared.pairing_open());
                if link.trusted.get() && shared.slot_of(&conn.raw().peer_identity()).is_none() {
                    info!("ble: connection {}: its bond was deleted; disconnecting", me);
                    link.trusted.set(false);
                    link.slot.set(None);
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
                link.trusted.set(shared.holds(&bond));
                if link.trusted.get() {
                    info!(
                        "ble: connection {}: encrypted with the bond of {}",
                        me,
                        Disp(&AddrText(addr_of(&bond.identity.addr)))
                    );
                    link.slot.set(shared.slot_of(&bond.identity));
                    resume_gatt_client(me, ctx, conn, link).await;
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
                        link.trusted.set(true);
                        link.slot.set(shared.slot_of(&bond.identity));
                        // it discovered the database as it is
                        link.aware.set(true);
                        // what it configured before pairing goes with the bond
                        save_gatt_client(ctx, conn, link).await;
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
                        link.trusted.set(false);
                        link.slot.set(None);
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
                // GATT caching: what this request means for a client that
                // may not know the database as it is.
                let caching = ctx.caching;
                let incoming = request.payload().incoming();
                let reads_hash = match &incoming {
                    AttClient::Request(AttReq::Read { handle } | AttReq::ReadBlob { handle, .. }) => {
                        *handle == caching.database_hash.handle
                    }
                    AttClient::Request(AttReq::ReadByType { attribute_type, .. }) => {
                        *attribute_type == Uuid::from(characteristic::DATABASE_HASH)
                    }
                    _ => false,
                };
                let command = matches!(incoming, AttClient::Command(_));
                // client-specific values the bond keeps
                let configures = matches!(
                    &request,
                    GattEvent::Write(w) if Some(w.handle()) == caching.service_changed.cccd_handle
                        || w.handle() == caching.client_supported_features.handle
                );
                if !link.aware.get() {
                    if reads_hash {
                        // reading the hash makes it change-aware
                        // (Core Vol 3 Part G 2.5.2.1)
                        info!("ble: connection {}: client read the database hash; change-aware", me);
                        link.aware.set(true);
                        save_gatt_client(ctx, conn, link).await;
                    } else if robust_caching(ctx, conn) {
                        // A change-unaware client with robust caching gets
                        // Database Out Of Sync for its requests until it
                        // reads the hash; its commands are ignored.
                        if command {
                            let _ = request.into_payload();
                            continue;
                        }
                        debug!("ble: connection {}: request refused: database out of sync", me);
                        match request.reject(AttErrorCode::DATABASE_OUT_OF_SYNC) {
                            Ok(reply) => reply.send().await,
                            Err(e) => warn!("ble: connection {}: reply failed: {:?}", me, Dbg(&e)),
                        }
                        continue;
                    }
                }
                let trusted = link.trusted.get();
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
                                Some(payload) => runtime.event(payload, Transport::Ble).await,
                                None => Err(Refused::Undecodable),
                            };
                            if accepted.is_ok() {
                                debug!("ble: connection {}: event {:?}", me, Dbg(&payload));
                                write.accept()
                            } else {
                                warn!(
                                    "ble: connection {}: event refused ({:?}): {:?}",
                                    me,
                                    Dbg(&accepted),
                                    Dbg(&payload)
                                );
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
                    // it first: its timestamp is when it was captured, and
                    // the state may not have changed for a while (no GPS).
                    // A value longer than the MTU allows is read in parts
                    // (Read Blob), which the host answers from the table.
                    GattEvent::Read(read) if read.handle() == state.handle => {
                        if !trusted {
                            warn!("ble: connection {}: state read refused: link not bonded", me);
                            read.reject(AttErrorCode::INSUFFICIENT_AUTHORISATION)
                        } else {
                            if let Some(value) = state_value(runtime, &runtime.current_state().await)
                                && let Err(e) = conn.set(state, &value)
                            {
                                warn!("ble: cannot set state: {:?}", Dbg(&e));
                            }
                            read.accept()
                        }
                    }
                    // Client Supported Features: bits are set, never
                    // cleared (Core Vol 3 Part G 7.2).
                    GattEvent::Write(write) if write.handle() == caching.client_supported_features.handle => {
                        let current = client_features(ctx, conn);
                        let new = write.with_data(|offset, data| match (offset, data) {
                            (0, &[features]) => Some(features),
                            _ => None,
                        });
                        match new {
                            Some(new) if GattCaching::client_features_write_allowed(current, new) => {
                                info!("ble: connection {}: client supported features {:02x}", me, new);
                                write.accept()
                            }
                            _ => {
                                warn!(
                                    "ble: connection {}: client supported features {:?} refused (now {:02x})",
                                    me,
                                    Dbg(&new),
                                    current
                                );
                                if command {
                                    let _ = write.into_payload();
                                    continue;
                                }
                                write.reject(AttErrorCode::VALUE_NOT_ALLOWED)
                            }
                        }
                    }
                    other => other.accept(),
                };
                match reply {
                    Ok(reply) => reply.send().await,
                    Err(e) => warn!("ble: connection {}: reply failed: {:?}", me, Dbg(&e)),
                }
                if configures && trusted {
                    save_gatt_client(ctx, conn, link).await;
                }
            }
            _ => {}
        }
    }
}

/// The client's Service Changed CCCD on this link.
fn service_changed_cccd<C: Controller, S>(
    ctx: &Context<'_, '_, C, S>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
) -> u16 {
    let Some(handle) = ctx.caching.service_changed.cccd_handle else {
        return 0;
    };
    let mut value = [0u8; 2];
    match ctx.server.read(conn.raw(), handle, 0, &mut value) {
        Ok(2) => u16::from_le_bytes(value),
        _ => 0,
    }
}

/// The client's Client Supported Features on this link.
fn client_features<C: Controller, S>(
    ctx: &Context<'_, '_, C, S>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
) -> u8 {
    let mut value = [0u8; 1];
    match ctx
        .server
        .read(conn.raw(), ctx.caching.client_supported_features.handle, 0, &mut value)
    {
        Ok(1) => value[0],
        _ => 0,
    }
}

fn robust_caching<C: Controller, S>(
    ctx: &Context<'_, '_, C, S>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
) -> bool {
    client_features(ctx, conn) & GattCaching::ROBUST_CACHING != 0
}

/// A bonded client is back (encrypted with its bond): what it configured
/// in the GATT service comes back from its record (merged with anything it
/// set on this link before encrypting), and if the database is not the one
/// it last saw it is told: Service Changed if it subscribed, else Database
/// Out Of Sync errors if it uses robust caching ([`serve_gatt`]).
async fn resume_gatt_client<C: Controller, S: BondStore>(
    me: usize,
    ctx: &Context<'_, '_, C, S>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    link: &Link,
) {
    let Some(slot) = link.slot.get() else {
        return;
    };
    let caching = ctx.caching;
    let record = ctx.shared.gatt.borrow()[slot].unwrap_or_default();
    if let Some(mut table) = ctx.server.get_client_att_table(conn.raw()) {
        if let Some(cccd) = caching.service_changed.cccd_handle {
            let now = service_changed_cccd(ctx, conn);
            let value = if now != 0 { now } else { record.service_changed };
            let _ = table.write(cccd, 0, &value.to_le_bytes());
        }
        let features = client_features(ctx, conn) | record.features;
        let _ = table.write(caching.client_supported_features.handle, 0, &[features]);
        ctx.server.set_client_att_table(conn.raw(), &table.view());
    }
    link.aware.set(record.aware);
    info!(
        "ble: connection {}: GATT client {}, Service Changed CCCD {:04x}, features {:02x}",
        me,
        if record.aware { "change-aware" } else { "change-unaware" },
        service_changed_cccd(ctx, conn),
        client_features(ctx, conn)
    );
    if !record.aware {
        if caching.service_changed.should_indicate(conn) {
            info!(
                "ble: connection {}: the database changed since this client saw it: indicating Service Changed",
                me
            );
            match caching
                .service_changed
                .indicate(conn, &GattCaching::WHOLE_DATABASE, false)
                .await
            {
                Ok(()) => {
                    info!("ble: connection {}: Service Changed confirmed; change-aware", me);
                    link.aware.set(true);
                }
                Err(e) => warn!("ble: connection {}: Service Changed not confirmed: {:?}", me, Dbg(&e)),
            }
        } else if robust_caching(ctx, conn) {
            info!(
                "ble: connection {}: the database changed since this client saw it: Database Out Of Sync until it reads the hash",
                me
            );
        } else {
            warn!(
                "ble: connection {}: the database changed since this client saw it, and it neither subscribed to Service Changed nor uses robust caching: it may use a stale copy (forget {} on the device to clear it)",
                me,
                BLE_NAME
            );
        }
    }
    save_gatt_client(ctx, conn, link).await;
}

/// Stores what a trusted bonded client configured in the GATT service, and
/// whether it is change-aware, with its bond, if that changed.
async fn save_gatt_client<C: Controller, S: BondStore>(
    ctx: &Context<'_, '_, C, S>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    link: &Link,
) {
    let Some(slot) = link.slot.get() else {
        return;
    };
    let record = GattClientRecord {
        aware: link.aware.get(),
        service_changed: service_changed_cccd(ctx, conn),
        features: client_features(ctx, conn),
    };
    if ctx.shared.gatt.borrow()[slot] == Some(record) {
        return;
    }
    if ctx.store.save_gatt(slot, &record).await {
        debug!("ble: bond slot {}: GATT record {:?} saved", slot, Dbg(&record));
    } else {
        warn!("ble: bond slot {}: GATT record not saved", slot);
    }
    ctx.shared.gatt.borrow_mut()[slot] = Some(record);
}

/// The bulk half of a connection (BLE.md "Watch bundles"): an L2CAP
/// connection-oriented channel on [`UI_PSM`] that serves watch bundles,
/// accepted only on a trusted link. Never returns: the connection ends with
/// its GATT half.
async fn serve_bulk<C: Controller, S>(
    me: usize,
    ctx: &Context<'_, '_, C, S>,
    conn: &Connection<'_, DefaultPacketPool>,
    link: &Link,
) {
    let listener = L2capChannel::listen(ctx.stack, conn);
    loop {
        let Ok(pending) = listener.next().await else {
            // the connection is gone
            break;
        };
        if !link.trusted.get() {
            let encrypted = conn.security_level().is_ok_and(|level| level.encrypted());
            let code = if encrypted {
                LeCreditConnResultCode::InsufficientAuthorization
            } else {
                LeCreditConnResultCode::InsufficientEncryption
            };
            warn!(
                "ble: connection {}: bulk channel refused ({:?}): link not bonded",
                me,
                Dbg(&code)
            );
            if let Err(e) = pending.reject(ctx.stack, code).await {
                debug!("ble: connection {}: bulk refusal: {:?}", me, Dbg(&e));
            }
            continue;
        }
        let config = L2capChannelConfig {
            mtu: Some(BULK_RX_MTU),
            mps: Some(BULK_RX_MTU),
            flow_policy: CreditFlowPolicy::Every(1),
            initial_credits: Some(BULK_RX_CREDITS),
        };
        let mut channel = match pending.accept(ctx.stack, &config).await {
            Ok(channel) => channel,
            Err(e) => {
                warn!("ble: connection {}: bulk channel accept failed: {:?}", me, Dbg(&e));
                continue;
            }
        };
        info!(
            "ble: connection {}: bulk channel open: peer MTU {} MPS {} credits {}; ours MTU {} MPS {} credits {}",
            me,
            channel.peer_mtu(),
            channel.peer_mps(),
            channel.peer_credits(),
            BULK_RX_MTU,
            BULK_RX_MTU,
            BULK_RX_CREDITS
        );
        serve_channel(me, ctx, &listener, &mut channel, link).await;
        // dropping the channel closes it
    }
    core::future::pending().await
}

/// One bulk channel, until it closes: each request answered with its
/// header and the asset, streamed from flash an SDU at a time.
async fn serve_channel<'d, C: Controller, S>(
    me: usize,
    ctx: &Context<'_, '_, C, S>,
    listener: &L2capChannelListener<'d, C, DefaultPacketPool>,
    channel: &mut L2capChannel<'d, DefaultPacketPool>,
    link: &Link,
) {
    let sdu_max = bulk::sdu_max(
        channel.peer_mtu(),
        channel.peer_mps(),
        <DefaultPacketPool as PacketPool>::MTU,
    );
    if sdu_max == 0 {
        warn!(
            "ble: connection {}: bulk channel: peer MTU {} MPS {} leave no room for data; closing",
            me,
            channel.peer_mtu(),
            channel.peer_mps()
        );
        return;
    }
    loop {
        let mut request = [0u8; BULK_RX_MTU as usize];
        let n = match select3(
            channel.receive(ctx.stack, &mut request),
            listener.next(),
            Timer::after(BULK_IDLE),
        )
        .await
        {
            Either3::First(Ok(n)) => n,
            Either3::First(Err(e)) => {
                info!("ble: connection {}: bulk channel closed: {:?}", me, Dbg(&e));
                return;
            }
            Either3::Second(Ok(extra)) => {
                warn!("ble: connection {}: a second bulk channel refused (one at a time)", me);
                if let Err(e) = extra.reject(ctx.stack, LeCreditConnResultCode::NoResources).await {
                    debug!("ble: connection {}: bulk refusal: {:?}", me, Dbg(&e));
                }
                continue;
            }
            Either3::Second(Err(_)) => return,
            Either3::Third(()) => {
                info!(
                    "ble: connection {}: bulk channel idle for {} s; closing",
                    me,
                    BULK_IDLE.as_secs()
                );
                return;
            }
        };
        if !link.trusted.get() {
            warn!("ble: connection {}: bulk request on a link no longer trusted; closing", me);
            return;
        }
        let request = match bulk::parse_request(&request[..n]) {
            Ok(request) => request,
            Err(e) => {
                warn!(
                    "ble: connection {}: bulk request refused ({:?}): {:?}; closing",
                    me,
                    Dbg(&e),
                    Dbg(&&request[..n])
                );
                return;
            }
        };
        let (header, asset) = ctx.runtime.asset(&request);
        let name = ctx.runtime.package(request.package).map_or("?", |p| p.name);
        let transfer = Transfer::new(header, asset, sdu_max);
        info!(
            "ble: connection {}: bulk: {} watch bundle {} asked for: status {}, {} B in {} SDUs of up to {} B (peer MTU {} MPS {} credits {})",
            me,
            name,
            Disp(&Hex(&request.hash)),
            header.status,
            header.len,
            transfer.sdus(),
            sdu_max,
            channel.peer_mtu(),
            channel.peer_mps(),
            channel.peer_credits()
        );
        let diagnostics = ctx.diagnostics;
        let heap_before = (diagnostics.heap_free)();
        let mut heap_min = heap_before;
        let mut credit_waits = 0u32;
        let start = Instant::now();
        let mut sent = 0usize;
        for sdu in transfer {
            if channel.peer_credits() == 0 {
                credit_waits += 1;
            }
            if let Err(e) = channel.send(ctx.stack, sdu.bytes()).await {
                warn!(
                    "ble: connection {}: bulk: send failed after {} of {} B: {:?}; closing",
                    me,
                    sent,
                    header.len,
                    Dbg(&e)
                );
                return;
            }
            if let bulk::Sdu::Data(d) = sdu {
                sent += d.len();
            }
            heap_min = heap_min.min((diagnostics.heap_free)());
        }
        let ms = start.elapsed().as_millis();
        info!(
            "ble: connection {}: bulk: sent {} B of {} in {} ms ({} B/s); waited for credits {} times; heap free {} before, min {} during",
            me,
            sent,
            name,
            ms,
            bulk::throughput(sent, ms),
            credit_waits,
            heap_before,
            heap_min
        );
        (diagnostics.log_memory)("after a bulk transfer");
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
    // a new client: it discovered the database as it is, and has no stored
    // subscriptions (another device's are gone with its bond)
    let gatt = GattClientRecord {
        aware: true,
        ..Default::default()
    };
    if !store.save_gatt(slot, &gatt).await {
        warn!("ble: bond slot {}: GATT record not saved", slot);
    }
    shared.gatt.borrow_mut()[slot] = Some(gatt);
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

/// The `state` characteristic value of a state message: its envelope
/// stamped now, without the BLE status (Choose's view has `ble: None` here).
fn state_value(host: &Host, message: &StateMessage) -> Option<StateValue> {
    let mut value = StateValue::new();
    // cannot fail: resizing to the capacity
    let _ = value.resize(STATE_MAX, 0);
    match host.encode_envelope(message, host.now(), None, &mut value) {
        Ok(len) => {
            value.truncate(len);
            Some(value)
        }
        Err(()) => {
            // sized for the largest envelope, so this can't happen
            error!("ble: {} state does not fit {} bytes", message.name, STATE_MAX);
            None
        }
    }
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
        for (uuid, n) in [
            (SERVICE_UUID, 0x00),
            (STATE_UUID, 0x01),
            (EVENT_UUID, 0x02),
            (UI_PSM_UUID, 0x03),
        ] {
            let bytes = uuid.to_be_bytes();
            assert_eq!(bytes[..4], [0xe4, 0xa1, 0xc0, n]);
            assert_eq!(bytes[4..], tail);
        }
    }

    /// The bulk channel's PSM is an LE PSM from the dynamic range, which
    /// the host can register.
    #[test]
    fn ui_psm_is_dynamic() {
        assert!((0x0080..=0x00ff).contains(&UI_PSM));
        // a request fits one SDU of ours, and ours is a legal L2CAP MTU
        assert!(extreme_protocol::GET_ASSET_MAX <= BULK_RX_MTU as usize);
        assert!(BULK_RX_MTU >= 23);
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
            gatt: RefCell::new([None; MAX_BLE_BONDS]),
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
