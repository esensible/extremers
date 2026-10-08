//! The messages on the wire, around the engine's own state and events
//! (README "Wire protocols", BLE.md "State" and "Events"). Everything is
//! [postcard](https://docs.rs/postcard).
//!
//! A state message is the device's clock when it was sent, then the engine
//! state ([`RawEngine::serialize_state`](extreme_traits::RawEngine)):
//! [`State`] over BLE, and [`WsState`], the same plus the BLE status, on
//! the websocket. The engine state is serialized once per change (the
//! runtime's `StateMessage`) and each transport writes the time in front
//! of it when it sends, with [`encode_state`] / [`encode_ws_state`], so the
//! time is the moment of sending: clients set their clock offset from it.
//!
//! The types here are what `tools/bindgen` generates the clients' JS from,
//! with the engine set's `EngineState` and `EngineEvent` for `S` and `E`;
//! the tests below check that the hand-assembled bytes are exactly those
//! types' postcard.

use postcard_schema::Schema;
use serde::{Deserialize, Serialize};

use crate::ble_settings::{BLE_STATUS_VIEW_MAX, BleCommand, BleStatusView};

/// A state message on BLE: the BLE `state` characteristic's value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Schema)]
pub struct State<S> {
    /// The device's clock when this was sent, epoch milliseconds (uptime
    /// until the GPS has given it the time).
    pub timestamp: u64,
    pub engine: S,
}

/// A state message on the websocket (a binary frame): [`State`]'s fields,
/// then the BLE transport's status for the Kindle's settings panel (`None`
/// on a target without BLE).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Schema)]
pub struct WsState<S> {
    pub timestamp: u64,
    pub engine: S,
    pub ble: Option<BleStatusView>,
}

/// A message from a websocket client (a binary frame): an engine event, or
/// a command for the BLE transport. Over BLE a client sends the engine
/// event alone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Schema)]
pub enum WsEvent<E> {
    Engine(E),
    Ble(BleCommand),
}

/// Longest postcard `u64` (a varint).
pub const MAX_TIMESTAMP_SIZE: usize = 10;

/// Largest [`State`].
pub const MAX_STATE_MESSAGE: usize = MAX_TIMESTAMP_SIZE + extreme_traits::MAX_STATE_SIZE;

/// Largest [`WsState`].
pub const MAX_WS_STATE_MESSAGE: usize = MAX_STATE_MESSAGE + BLE_STATUS_VIEW_MAX;

/// Writes a [`State`] into `out`, from the engine state already serialized:
/// the timestamp, then `engine` as is. Returns the length, or `Err` if
/// `out` is too small.
pub fn encode_state(timestamp: u64, engine: &[u8], out: &mut [u8]) -> Result<usize, ()> {
    let n = postcard::to_slice(&timestamp, out).map_err(|_| ())?.len();
    let end = n + engine.len();
    out.get_mut(n..end).ok_or(())?.copy_from_slice(engine);
    Ok(end)
}

/// Writes a [`WsState`] into `out`: [`encode_state`], then `ble`.
pub fn encode_ws_state(
    timestamp: u64,
    engine: &[u8],
    ble: Option<&BleStatusView>,
    out: &mut [u8],
) -> Result<usize, ()> {
    let n = encode_state(timestamp, engine, out)?;
    let rest = out.get_mut(n..).ok_or(())?;
    Ok(n + postcard::to_slice(&ble, rest).map_err(|_| ())?.len())
}

/// The variants of [`WsEvent`], without their data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum WsEventKind {
    Engine,
    Ble,
}

/// Splits a websocket client message into its [`WsEvent`] variant and that
/// variant's data, which is the engine event's postcard for `Engine`.
/// `None` if it is not a `WsEvent`.
pub fn split_ws_event(message: &[u8]) -> Option<(WsEventKind, &[u8])> {
    postcard::take_from_bytes::<WsEventKind>(message).ok()
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::ble_settings::BleNote;
    use std::vec::Vec;

    /// Stands in for an engine set's state and events.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Schema)]
    enum Engine {
        Selector(()),
        Counter { count: u32, speed: f32 },
    }

    fn engine_bytes(engine: &Engine) -> Vec<u8> {
        postcard::to_stdvec(engine).unwrap()
    }

    #[test]
    fn state_is_timestamp_then_engine() {
        let engine = Engine::Counter { count: 300, speed: 6.5 };
        let mut out = [0u8; MAX_STATE_MESSAGE];
        let n = encode_state(1_790_000_000_123, &engine_bytes(&engine), &mut out).unwrap();
        let typed = State { timestamp: 1_790_000_000_123, engine: engine.clone() };
        assert_eq!(&out[..n], &postcard::to_stdvec(&typed).unwrap()[..]);
        assert_eq!(postcard::from_bytes::<State<Engine>>(&out[..n]).unwrap(), typed);
        // a timestamp from 2026 is a 6-byte varint
        assert_eq!(n, 6 + engine_bytes(&engine).len());

        // too small a buffer
        assert_eq!(encode_state(0, &[1, 2, 3], &mut out[..3]), Err(()));
        assert_eq!(encode_state(u64::MAX, &[], &mut out[..9]), Err(()));
    }

    #[test]
    fn ws_state_adds_ble_status() {
        let engine = Engine::Selector(());
        let ble = BleStatusView {
            addr: [1, 2, 3, 4, 5, 6],
            pair_ms: 117_000,
            bonds: heapless::Vec::from_slice(&[[9; 6]]).unwrap(),
            max_bonds: 4,
            note: BleNote::Paired([9; 6]),
        };
        let mut out = [0u8; MAX_WS_STATE_MESSAGE];
        for ble in [None, Some(ble)] {
            let n = encode_ws_state(7, &engine_bytes(&engine), ble.as_ref(), &mut out).unwrap();
            let typed = WsState { timestamp: 7, engine: engine.clone(), ble };
            assert_eq!(&out[..n], &postcard::to_stdvec(&typed).unwrap()[..]);
        }
        // without BLE: timestamp, selector tag, None
        let n = encode_ws_state(7, &[0], None, &mut out).unwrap();
        assert_eq!(&out[..n], &[7, 0, 0]);
    }

    #[test]
    fn ws_events_split() {
        let event = WsEvent::<Engine>::Engine(Engine::Counter { count: 1, speed: 0.0 });
        let bytes = postcard::to_stdvec(&event).unwrap();
        let (kind, rest) = split_ws_event(&bytes).unwrap();
        assert_eq!(kind, WsEventKind::Engine);
        assert_eq!(postcard::from_bytes::<Engine>(rest).unwrap(), Engine::Counter { count: 1, speed: 0.0 });

        let bytes = postcard::to_stdvec(&WsEvent::<Engine>::Ble(BleCommand::ForgetAll)).unwrap();
        assert_eq!(bytes, [1, 2]);
        assert_eq!(split_ws_event(&bytes), Some((WsEventKind::Ble, &[2u8][..])));

        assert_eq!(split_ws_event(&[]), None);
        assert_eq!(split_ws_event(&[2]), None);
    }
}
