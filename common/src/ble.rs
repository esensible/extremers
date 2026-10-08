//! BLE GATT transport for an [`EngineRuntime`] (BLE.md).
//!
//! One service with two characteristics: `state` (read, notify) carries the
//! compact engine state, `event` (write, write-without-response) takes
//! compact client events. Up to [`CONNECTIONS_MAX`] centrals at once, each
//! served by its own worker; a worker with no connection advertises, and
//! only one advertises at a time, so the device is connectable exactly
//! while a slot is free. No pairing or bonding.
//!
//! Platform-free: the target hands in a bt-hci controller.

use embassy_futures::{
    join::{join, join_array},
    select::{Either, select},
};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer};
use trouble_host::{att::AttClient, prelude::*};

use extreme_traits::RawEngine;

use crate::{
    ble_settings::AddrText,
    config::{BLE_NAME, MAX_BLE_CONNECTIONS},
    fmt::{Dbg, Disp},
    runtime::{EngineRuntime, StateMessage},
};

/// Concurrent BLE connections: the watch plus a phone or a debugging tool.
/// The target sizes its controller with it too.
pub const CONNECTIONS_MAX: usize = MAX_BLE_CONNECTIONS;

/// L2CAP channels: signal + ATT, as in the esp-hal coex example.
const L2CAP_CHANNELS_MAX: usize = 2;

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
#[gatt_service(uuid = SERVICE_UUID)]
struct RaceService {
    #[characteristic(uuid = STATE_UUID, read, notify, value = heapless09::Vec::new())]
    state: StateValue,
    #[characteristic(uuid = EVENT_UUID, write, write_without_response, value = heapless09::Vec::new())]
    event: EventValue,
}

/// Serves the BLE GATT transport on `controller`, forever, as the static
/// random `address` (BLE byte order; [`static_random_address`] makes one
/// from the chip's MAC, so every unit has its own).
///
/// [`static_random_address`]: crate::ble_settings::static_random_address
pub async fn serve_ble<C, E>(controller: C, runtime: &EngineRuntime<E>, address: [u8; 6]) -> !
where
    C: Controller,
    E: RawEngine,
{
    info!("ble: address {} (static random)", Disp(&AddrText(address)));
    let address = Address::random(address);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
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
    let workers: [_; CONNECTIONS_MAX] =
        core::array::from_fn(|me| worker(me, &stack, &advertising, &server, runtime, &unit));

    join(host, join_array(workers)).await;
    // both halves loop forever
    unreachable!()
}

/// Worker `me`: advertise (while holding the advertising lock), accept,
/// serve the connection until it drops, repeat.
async fn worker<'v, C, E>(
    me: usize,
    stack: &Stack<'_, C, DefaultPacketPool>,
    advertising: &Mutex<NoopRawMutex, &mut Peripheral<'v, C, DefaultPacketPool>>,
    server: &Server<'_>,
    runtime: &EngineRuntime<E>,
    unit: &[u8; 6],
) where
    C: Controller,
    E: RawEngine,
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

        info!(
            "ble: connection {} from {:?}",
            me,
            Dbg(&conn.peer_address())
        );
        let raw = conn.clone();
        match conn.with_attribute_server(server) {
            Ok(conn) => serve(me, stack, server, &conn, runtime).await,
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
/// events in as writes.
async fn serve<C, E>(
    me: usize,
    stack: &Stack<'_, C, DefaultPacketPool>,
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    runtime: &EngineRuntime<E>,
) where
    C: Controller,
    E: RawEngine,
{
    let state = &server.race.state;
    let event = &server.race.event;

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

    if let Ok(message) = runtime.current_state().await {
        // a central that subscribes later reads it from the table
        if let Some(value) = state_value(&message) {
            if let Err(e) = conn.set(state, &value) {
                warn!("ble: cannot set state: {:?}", Dbg(&e));
            }
            let _ = state.notify(conn, &value, false).await;
        }
    }

    loop {
        let gatt = match select(conn.next(), updates.next_message_pure()).await {
            Either::First(gatt) => gatt,
            Either::Second(message) => {
                if let Some(value) = state_value(&message) {
                    // stored too, so reads stay current
                    if let Err(e) = state.notify(conn, &value, true).await {
                        debug!("ble: connection {}: notify failed: {:?}", me, Dbg(&e));
                    }
                }
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
            GattConnectionEvent::Gatt { event: request } => {
                let reply = match request {
                    GattEvent::Write(write) if write.handle() == event.handle => {
                        // a write command (write without response) gets no
                        // reply, not even an error
                        let command = matches!(write.payload().incoming(), AttClient::Command(_));
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
                    // `accept` builds the reply from the table, so refresh
                    // it first: its times are relative to when it was
                    // captured, and the state may not have changed for a
                    // while (no GPS)
                    GattEvent::Read(read) if read.handle() == state.handle => {
                        if let Ok(message) = runtime.current_state().await
                            && let Some(value) = state_value(&message)
                            && let Err(e) = conn.set(state, &value)
                        {
                            warn!("ble: cannot set state: {:?}", Dbg(&e));
                        }
                        read.accept()
                    }
                    // CCCD writes are handled by trouble
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
}
