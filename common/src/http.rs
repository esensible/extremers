//! HTTP and websocket transport for an [`EngineRuntime`].
//!
//! Serves the active engine's static files, and on `/socket` upgrades to a
//! websocket that carries client events in and state updates out.

use core::fmt::{Debug, Display, Write as _};

use embassy_futures::select::{Either, select};

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

use extreme_traits::{MAX_MESSAGE_SIZE, RawEngine};

use crate::runtime::{EngineRuntime, StateMessage};

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
                log::error!("websocket: cannot subscribe to state updates: {:?}", e);
                return Ok(());
            }
        };

        let (mut rx, mut tx) = socket.split();

        if let Ok(state) = self.runtime.current_state().await {
            send_state(&mut tx, self.runtime.now(), &state).await?;
        }

        let mut buf = [0_u8; MAX_MESSAGE_SIZE];
        loop {
            // wait for readability rather than for a frame header: a frame
            // read cancelled half way would desynchronise the stream
            match select(rx.readable(), updates.next_message_pure()).await {
                Either::First(readable) => {
                    readable.map_err(WsError::Io)?;
                    let header = FrameHeader::recv(&mut rx).await?;
                    let payload = header.recv_payload(&mut rx, &mut buf).await?;

                    match header.frame_type {
                        FrameType::Text(_) | FrameType::Binary(_) => {
                            if self.runtime.external_event(payload).await.is_err() {
                                log::warn!(
                                    "websocket: undecodable event: {}",
                                    core::str::from_utf8(payload).unwrap_or("<not utf-8>")
                                );
                            }
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
                            log::info!("websocket: closed by client");
                            return Ok(());
                        }
                        FrameType::Pong | FrameType::Continue(_) => {
                            log::debug!("websocket: ignoring {}", header);
                        }
                    }
                }
                Either::Second(state) => {
                    send_state(&mut tx, self.runtime.now(), &state).await?;
                }
            }
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

            log::debug!("http: GET {}", path);
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
            log::info!("websocket: connected");

            let socket = conn.unbind()?;
            if let Err(e) = self.run_websocket(socket).await {
                log::info!("websocket: disconnected: {:?}", e);
            }
        }

        Ok(())
    }
}

/// Sends `{"timestamp":<timestamp>,"kind":"<kind>","engine":<state>}` as a
/// single websocket text frame. The pieces are written straight to the
/// socket, so there is no intermediate buffer to overflow.
async fn send_state<W>(
    socket: &mut W,
    timestamp: u64,
    state: &StateMessage,
) -> Result<(), WsError<W::Error>>
where
    W: Write,
{
    const PREFIX: &[u8] = b"{\"timestamp\":";
    const KIND: &[u8] = b",\"kind\":\"";
    const ENGINE: &[u8] = b"\",\"engine\":";
    const SUFFIX: &[u8] = b"}";

    let mut digits: heapless::String<20> = heapless::String::new();
    // a u64 has at most 20 digits, so this can't fail
    let _ = write!(digits, "{}", timestamp);

    // engine names are Rust identifiers, so need no JSON escaping
    let pieces = [
        PREFIX,
        digits.as_bytes(),
        KIND,
        state.kind.as_bytes(),
        ENGINE,
        &state.json,
        SUFFIX,
    ];

    let header = FrameHeader {
        mask_key: None,
        // `false`: not fragmented, this frame is the whole message
        frame_type: FrameType::Text(false),
        payload_len: pieces.iter().map(|piece| piece.len() as u64).sum(),
    };
    header.send(&mut *socket).await?;

    // server frames are not masked, so the payload can be written in pieces
    for piece in pieces {
        socket.write_all(piece).await.map_err(WsError::Io)?;
    }
    socket.flush().await.map_err(WsError::Io)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use embassy_futures::block_on;
    use extreme_traits::StateJson;

    #[test]
    fn send_state_handles_max_size_message() {
        let mut json = StateJson::new();
        json.resize(MAX_MESSAGE_SIZE, b'x').unwrap();
        json[0] = b'"';
        json[MAX_MESSAGE_SIZE - 1] = b'"';
        let state = StateMessage {
            kind: "TuneSpeed",
            json,
        };

        let mut out = [0u8; MAX_MESSAGE_SIZE + 128];
        let capacity = out.len();
        let written = {
            let mut writer: &mut [u8] = &mut out;
            block_on(send_state(&mut writer, u64::MAX, &state)).unwrap();
            capacity - writer.len()
        };

        let mut reader: &[u8] = &out[..written];
        let header = block_on(FrameHeader::recv(&mut reader)).unwrap();
        assert_eq!(header.frame_type, FrameType::Text(false));
        assert_eq!(header.payload_len as usize, reader.len());

        let expected = std::format!(
            "{{\"timestamp\":{},\"kind\":\"TuneSpeed\",\"engine\":{}}}",
            u64::MAX,
            core::str::from_utf8(&state.json).unwrap()
        );
        assert_eq!(reader, expected.as_bytes());
    }
}
