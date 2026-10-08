//! HTTP and websocket transport for an [`EngineRuntime`].
//!
//! Serves the active engine's static files, and on `/socket` upgrades to a
//! websocket that carries client events in and state updates out, both as
//! binary frames of postcard ([`crate::protocol`]).

use core::fmt::{Debug, Display};

use embassy_futures::select::{Either3, select3};
use embassy_time::{Duration, Instant, Timer};

use edge_http::{
    Method,
    io::{
        Error,
        server::{Connection, Handler},
    },
    ws::MAX_BASE64_KEY_RESPONSE_LEN,
};
use edge_nal::{Readable, TcpSplit};
use edge_ws::{Error as WsError, FrameHeader, FrameType};
use embedded_io_async::{Read, Write};

use extreme_traits::RawEngine;

use crate::{
    ble_settings::BleStatusView,
    config::{WS_CLIENT_TIMEOUT_MS, WS_HEARTBEAT_MS, WS_MAX_INCOMING},
    fmt::{Dbg, Disp},
    protocol::{MAX_WS_STATE_MESSAGE, WsEventKind, encode_ws_state, split_ws_event},
    runtime::{EngineRuntime, StateMessage},
};

/// An [`edge_http`] request handler backed by an [`EngineRuntime`].
pub struct HttpHandler<'r, E: RawEngine> {
    runtime: &'r EngineRuntime<E>,
}

// derived impls would needlessly require `E: Clone`
impl<E: RawEngine> Clone for HttpHandler<'_, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E: RawEngine> Copy for HttpHandler<'_, E> {}

impl<'r, E: RawEngine> HttpHandler<'r, E> {
    pub fn new(runtime: &'r EngineRuntime<E>) -> Self {
        Self { runtime }
    }

    /// Relays between a websocket and the runtime until either side fails or
    /// the client closes.
    async fn run_websocket<S>(&self, socket: &mut S) -> Result<(), WsError<S::Error>>
    where
        S: TcpSplit,
    {
        // subscribe before reading the current state, so no change is missed
        let mut updates = match self.runtime.subscribe() {
            Ok(updates) => updates,
            Err(e) => {
                error!(
                    "websocket: cannot subscribe to state updates: {:?}",
                    Dbg(&e)
                );
                return Ok(());
            }
        };

        let (mut rx, mut tx) = socket.split();

        if let Ok(state) = self.runtime.current_state().await {
            send_state(&mut tx, self.runtime.now(), &state, self.runtime.ble_status_view().as_ref()).await?;
        }

        // A client that vanishes without closing (a Kindle going to sleep)
        // leaves no trace on the read side, so it is pinged every heartbeat
        // and dropped once it has been silent for too long; a browser
        // answers pings by itself. Each heartbeat also carries the state,
        // so the client can tell a dead connection from a quiet engine.
        let heartbeat = Duration::from_millis(WS_HEARTBEAT_MS);
        let mut last_heard = Instant::now();
        let mut next_heartbeat = last_heard + heartbeat;

        let mut buf = [0_u8; WS_MAX_INCOMING];
        loop {
            // wait for readability rather than for a frame header: a frame
            // read cancelled half way would desynchronise the stream
            match select3(
                rx.readable(),
                updates.next_message_pure(),
                Timer::at(next_heartbeat),
            )
            .await
            {
                Either3::First(readable) => {
                    readable.map_err(WsError::Io)?;
                    let header = FrameHeader::recv(&mut rx).await?;
                    let payload = header.recv_payload(&mut rx, &mut buf).await?;
                    last_heard = Instant::now();

                    match header.frame_type {
                        FrameType::Binary(_) => self.client_event(payload).await,
                        FrameType::Text(_) => {
                            warn!("websocket: ignoring a text frame (the protocol is binary)");
                        }
                        FrameType::Ping => {
                            let pong = FrameHeader {
                                mask_key: None,
                                frame_type: FrameType::Pong,
                                payload_len: payload.len() as u64,
                            };
                            pong.send(&mut tx).await?;
                            pong.send_payload(&mut tx, payload).await?;
                        }
                        FrameType::Close => {
                            info!("websocket: closed by client");
                            return Ok(());
                        }
                        // a pong has done its job by arriving
                        FrameType::Pong | FrameType::Continue(_) => {
                            debug!("websocket: ignoring {}", Disp(&header));
                        }
                    }
                }
                Either3::Second(state) => {
                    send_state(&mut tx, self.runtime.now(), &state, self.runtime.ble_status_view().as_ref()).await?;
                }
                Either3::Third(()) => {
                    if last_heard.elapsed() >= Duration::from_millis(WS_CLIENT_TIMEOUT_MS) {
                        info!(
                            "websocket: client silent for {} ms, disconnecting",
                            last_heard.elapsed().as_millis()
                        );
                        return Ok(());
                    }
                    next_heartbeat = Instant::now() + heartbeat;
                    send_ping(&mut tx).await?;

                    // the client can't see pings, so it is also sent the
                    // state, which it takes as proof the connection lives.
                    // A pending update is sent instead, rather than left to
                    // follow (and possibly undo) a newer current state.
                    let state = match updates.try_next_message_pure() {
                        Some(state) => Ok(state),
                        None => self.runtime.current_state().await,
                    };
                    if let Ok(state) = state {
                        send_state(&mut tx, self.runtime.now(), &state, self.runtime.ble_status_view().as_ref()).await?;
                    }
                }
            }
        }
    }
}

impl<E: RawEngine> HttpHandler<'_, E> {
    /// A message from the client: a `WsEvent` (an engine event, or a BLE
    /// command).
    async fn client_event(&self, payload: &[u8]) {
        match split_ws_event(payload) {
            Some((WsEventKind::Engine, event)) => {
                if self.runtime.external_event(event).await.is_err() {
                    warn!(
                        "websocket: engine event refused (undecodable, or not for the active engine): {:?}",
                        Dbg(&event)
                    );
                }
            }
            Some((WsEventKind::Ble, command)) => {
                match postcard::from_bytes(command) {
                    Ok(command) => match self.runtime.ble_command(command) {
                        Ok(()) => info!("websocket: BLE command {:?}", Dbg(&command)),
                        Err(()) => warn!(
                            "websocket: BLE command {:?} refused (no BLE on this target, or busy)",
                            Dbg(&command)
                        ),
                    },
                    Err(_) => warn!("websocket: undecodable BLE command: {:?}", Dbg(&command)),
                }
            }
            None => warn!("websocket: undecodable message: {:?}", Dbg(&payload)),
        }
    }
}

impl<E: RawEngine> Handler for HttpHandler<'_, E> {
    type Error<T>
        = Error<T>
    where
        T: Debug;

    async fn handle<T, const N: usize>(
        &self,
        _task_id: impl Display + Copy,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write + TcpSplit,
    {
        let headers = conn.headers()?;

        if headers.method != Method::Get {
            conn.initiate_response(405, Some("Method Not Allowed"), &[])
                .await?;
        } else if headers.path != "/socket" {
            let path = match headers.path.trim_start_matches('/') {
                "" => "index.html",
                path => path,
            };

            debug!("http: GET {}", path);
            // the lock is released before the (slow) write
            if let Some(file) = self.runtime.static_file(path).await {
                conn.initiate_response(200, Some("OK"), &[]).await?;
                conn.write_all(file).await?;
            } else {
                conn.initiate_response(404, Some("Not Found"), &[]).await?;
            }
        } else if !conn.is_ws_upgrade_request()? {
            conn.initiate_response(200, Some("OK"), &[("Content-Type", "text/plain")])
                .await?;
            conn.write_all(b"Initiate WS Upgrade request to switch this connection to WS")
                .await?;
        } else {
            let mut buf = [0_u8; MAX_BASE64_KEY_RESPONSE_LEN];
            conn.initiate_ws_upgrade_response(&mut buf).await?;
            conn.complete().await?;
            info!("websocket: connected");

            let socket = conn.unbind()?;
            if let Err(e) = self.run_websocket(socket).await {
                info!("websocket: disconnected: {:?}", Dbg(&e));
            }
        }

        Ok(())
    }
}

/// Sends an empty websocket ping.
async fn send_ping<W>(socket: &mut W) -> Result<(), WsError<W::Error>>
where
    W: Write,
{
    let header = FrameHeader {
        mask_key: None,
        frame_type: FrameType::Ping,
        payload_len: 0,
    };
    header.send(&mut *socket).await?;
    socket.flush().await.map_err(WsError::Io)
}

/// Sends a state message, a `WsState` stamped `timestamp` with the BLE
/// status `ble`, as one binary websocket frame.
async fn send_state<W>(
    socket: &mut W,
    timestamp: u64,
    state: &StateMessage,
    ble: Option<&BleStatusView>,
) -> Result<(), WsError<W::Error>>
where
    W: Write,
{
    let mut frame = [0u8; MAX_WS_STATE_MESSAGE];
    // sized for the largest engine state and BLE status, so this can't fail
    let Ok(len) = encode_ws_state(timestamp, &state.engine, ble, &mut frame) else {
        error!("websocket: {} state does not fit a frame", state.kind);
        return Ok(());
    };

    let header = FrameHeader {
        mask_key: None,
        // `false`: not fragmented, this frame is the whole message
        frame_type: FrameType::Binary(false),
        payload_len: len as u64,
    };
    header.send(&mut *socket).await?;
    socket.write_all(&frame[..len]).await.map_err(WsError::Io)?;
    socket.flush().await.map_err(WsError::Io)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::{
        ble_settings::{BleNote, BleStatus},
        protocol::WsState,
    };
    use embassy_futures::block_on;
    use embassy_time::Instant;
    use extreme_traits::{MAX_STATE_SIZE, StateBytes};

    /// Stands in for an engine set's `EngineState`: a tag and some bytes.
    #[derive(Debug, PartialEq, serde::Deserialize)]
    enum Engine {
        Selector(()),
        Small(u8, u8),
    }

    fn sent(state: &StateMessage, timestamp: u64, ble: Option<&BleStatusView>) -> std::vec::Vec<u8> {
        let mut out = [0u8; MAX_WS_STATE_MESSAGE + 16];
        let capacity = out.len();
        let written = {
            let mut writer: &mut [u8] = &mut out;
            block_on(send_state(&mut writer, timestamp, state, ble)).unwrap();
            capacity - writer.len()
        };
        let mut reader: &[u8] = &out[..written];
        let header = block_on(FrameHeader::recv(&mut reader)).unwrap();
        assert_eq!(header.frame_type, FrameType::Binary(false));
        assert_eq!(header.payload_len as usize, reader.len());
        reader.to_vec()
    }

    #[test]
    fn send_state_handles_max_size_message() {
        let mut engine = StateBytes::new();
        engine.push(1).unwrap();
        engine.resize(MAX_STATE_SIZE, 0xab).unwrap();
        let state = StateMessage { kind: "Big", engine };
        let mut status = BleStatus {
            pair_until: Some(Instant::MAX),
            note: BleNote::PairedNotSaved([0xff; 6]),
            ..Default::default()
        };
        for _ in 0..crate::config::MAX_BLE_BONDS {
            status.bonds.push([0xff; 6]).unwrap();
        }
        let ble = status.view(Instant::from_secs(0));

        let frame = sent(&state, u64::MAX, Some(&ble));
        assert_eq!(frame.len(), MAX_WS_STATE_MESSAGE);
        // the timestamp, the engine bytes as they were, the status
        assert_eq!(&frame[..10], &postcard::to_stdvec(&u64::MAX).unwrap()[..]);
        assert_eq!(&frame[10..10 + MAX_STATE_SIZE], &state.engine[..]);
        let ble_bytes = postcard::to_stdvec(&Some(&ble)).unwrap();
        assert_eq!(&frame[10 + MAX_STATE_SIZE..], &ble_bytes[..]);
    }

    /// A small engine state decodes as the typed `WsState`.
    #[test]
    fn send_state_is_a_ws_state() {
        let state = StateMessage {
            kind: "Small",
            engine: StateBytes::from_slice(&[1, 0xab, 0xcd]).unwrap(),
        };
        let ble = BleStatus::default().view(Instant::from_secs(0));
        let frame = sent(&state, 1_790_000_000_000, Some(&ble));
        let decoded: WsState<Engine> = postcard::from_bytes(&frame).unwrap();
        assert_eq!(decoded.timestamp, 1_790_000_000_000);
        assert_eq!(decoded.engine, Engine::Small(0xab, 0xcd));
        assert_eq!(decoded.ble, Some(ble));
    }

    /// Without BLE the status is `None`, one byte.
    #[test]
    fn send_state_without_ble() {
        let state = StateMessage {
            kind: "Selector",
            engine: StateBytes::from_slice(&[0]).unwrap(),
        };
        assert_eq!(sent(&state, 7, None), [7, 0, 0]);
    }
}
