//! HTTP and websocket transport for a [`Host`].
//!
//! The Kindle has one entry path: `/` is always the *active* package's page
//! (its index.html, not cached), so a page that sees another package
//! become active reloads `/`. Each installed package's files are at
//! `/pkg/<id>/<file>` (`<id>` in decimal; Choose is 0), where its
//! index.html points, and its watch bundle at `/pkg/<id>/watch.js`. On
//! `/socket` it upgrades to a websocket that
//! carries host events in and envelopes out, both as binary frames of
//! postcard (README "Host protocol").

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

use extreme_protocol::{ENVELOPE_MAX, PackageId};

use crate::{
    config::{WS_CLIENT_TIMEOUT_MS, WS_HEARTBEAT_MS, WS_MAX_INCOMING},
    fmt::{Dbg, Disp},
    host::{Host, StateMessage, Transport},
};

/// An [`edge_http`] request handler backed by a [`Host`].
#[derive(Clone, Copy)]
pub struct HttpHandler<'r> {
    host: &'r Host,
}

/// What a GET asks for.
#[derive(Debug, PartialEq, Eq)]
enum Route<'a> {
    /// `/`: the active package's page.
    Entry,
    /// A file of a package's web bundle (or its watch bundle).
    File(PackageId, &'a str),
    Socket,
    NotFound,
}

/// `path` (query string dropped) as a [`Route`].
fn route(path: &str) -> Route<'_> {
    let path = path.split('?').next().unwrap_or("");
    match path {
        "/" | "/index.html" => return Route::Entry,
        "/socket" => return Route::Socket,
        _ => {}
    }
    let Some(rest) = path.strip_prefix("/pkg/") else {
        return Route::NotFound;
    };
    match rest.split_once('/') {
        Some((id, file)) if !file.is_empty() && !file.contains('/') => match id.parse::<PackageId>() {
            Ok(id) => Route::File(id, file),
            Err(_) => Route::NotFound,
        },
        _ => Route::NotFound,
    }
}

impl<'r> HttpHandler<'r> {
    pub fn new(host: &'r Host) -> Self {
        Self { host }
    }

    /// The contents of `file` of package `id`.
    fn file(&self, id: PackageId, file: &str) -> Option<&'static [u8]> {
        let package = self.host.package(id)?;
        if let Some(contents) = package.web_file(file) {
            return Some(contents);
        }
        (file == "watch.js" && !package.watch().is_empty()).then(|| package.watch())
    }

    /// Relays between a websocket and the host until either side fails or
    /// the client closes.
    async fn run_websocket<S>(&self, socket: &mut S) -> Result<(), WsError<S::Error>>
    where
        S: TcpSplit,
    {
        // subscribe before reading the current state, so no change is missed
        let mut updates = match self.host.subscribe() {
            Ok(updates) => updates,
            Err(e) => {
                error!("websocket: cannot subscribe to state updates: {:?}", Dbg(&e));
                return Ok(());
            }
        };

        let (mut rx, mut tx) = socket.split();

        let state = self.host.current_state().await;
        self.send_state(&mut tx, &state).await?;

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
            match select3(rx.readable(), updates.next_message_pure(), Timer::at(next_heartbeat)).await {
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
                Either3::Second(state) => self.send_state(&mut tx, &state).await?,
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
                        Some(state) => state,
                        None => self.host.current_state().await,
                    };
                    self.send_state(&mut tx, &state).await?;
                }
            }
        }
    }

    /// A message from the client: a `HostEvent`.
    async fn client_event(&self, payload: &[u8]) {
        match self.host.event(payload, Transport::WebSocket).await {
            Ok(()) => debug!("websocket: event {:?}", Dbg(&payload)),
            Err(why) => warn!("websocket: event refused ({:?}): {:?}", Dbg(&why), Dbg(&payload)),
        }
    }

    /// Sends `state` as an envelope stamped now, with the BLE status for
    /// Choose, as one binary frame.
    async fn send_state<W: Write>(&self, socket: &mut W, state: &StateMessage) -> Result<(), WsError<W::Error>> {
        let mut frame = [0u8; ENVELOPE_MAX];
        let Ok(len) = self
            .host
            .encode_envelope(state, self.host.now(), self.host.ble_status_view(), &mut frame)
        else {
            // sized for the largest envelope, so this can't happen
            error!("websocket: {} state does not fit a frame", state.name);
            return Ok(());
        };
        send_frame(socket, &frame[..len]).await
    }
}

impl Handler for HttpHandler<'_> {
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
            conn.initiate_response(405, Some("Method Not Allowed"), &[]).await?;
            return Ok(());
        }
        debug!("http: GET {}", headers.path);
        let route = route(headers.path);
        // a file is read from its bundle in place until it is sent: a
        // lease, taken before the package is looked up, keeps the bundle's
        // sectors from being erased meanwhile should it be deleted (not for
        // the websocket, which reads no bundle and lives long)
        let _lease = (!matches!(route, Route::Socket)).then(|| self.host.lease());
        // the entry page must never come from a cache: a reload has to get
        // whichever package is active now
        let (found, no_cache) = match route {
            Route::Socket => (None, false),
            Route::Entry => {
                let id = self.host.current_state().await.active;
                (Some(self.file(id, "index.html")), true)
            }
            Route::File(id, file) => (Some(self.file(id, file)), false),
            Route::NotFound => (Some(None), false),
        };
        match found {
            Some(Some(file)) if no_cache => {
                conn.initiate_response(
                    200,
                    Some("OK"),
                    &[("Content-Type", "text/html"), ("Cache-Control", "no-cache, no-store, must-revalidate")],
                )
                .await?;
                conn.write_all(file).await?;
            }
            Some(Some(file)) => {
                conn.initiate_response(200, Some("OK"), &[]).await?;
                conn.write_all(file).await?;
            }
            Some(None) => {
                conn.initiate_response(404, Some("Not Found"), &[]).await?;
            }
            None if !conn.is_ws_upgrade_request()? => {
                conn.initiate_response(200, Some("OK"), &[("Content-Type", "text/plain")])
                    .await?;
                conn.write_all(b"Initiate WS Upgrade request to switch this connection to WS")
                    .await?;
            }
            None => {
                let mut buf = [0_u8; MAX_BASE64_KEY_RESPONSE_LEN];
                conn.initiate_ws_upgrade_response(&mut buf).await?;
                conn.complete().await?;
                info!("websocket: connected");

                let socket = conn.unbind()?;
                if let Err(e) = self.run_websocket(socket).await {
                    info!("websocket: disconnected: {:?}", Dbg(&e));
                }
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

/// Sends `payload` as one binary frame.
async fn send_frame<W: Write>(socket: &mut W, payload: &[u8]) -> Result<(), WsError<W::Error>> {
    let header = FrameHeader {
        mask_key: None,
        // `false`: not fragmented, this frame is the whole message
        frame_type: FrameType::Binary(false),
        payload_len: payload.len() as u64,
    };
    header.send(&mut *socket).await?;
    socket.write_all(payload).await.map_err(WsError::Io)?;
    socket.flush().await.map_err(WsError::Io)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use embassy_futures::block_on;
    use extreme_protocol::CHOOSE;

    #[test]
    fn routes() {
        assert_eq!(route("/"), Route::Entry);
        assert_eq!(route("/?x=1"), Route::Entry);
        assert_eq!(route("/index.html"), Route::Entry);
        assert_eq!(route("/socket"), Route::Socket);
        assert_eq!(route("/pkg/123/index.html"), Route::File(123, "index.html"));
        assert_eq!(route("/pkg/0/bundle-ab.js"), Route::File(CHOOSE, "bundle-ab.js"));
        assert_eq!(route("/pkg/123/watch.js"), Route::File(123, "watch.js"));
        // no entry pages but `/`
        assert_eq!(route("/pkg/123/"), Route::NotFound);
        assert_eq!(route("/pkg/123"), Route::NotFound);
        assert_eq!(route("/bundle-ab.js"), Route::NotFound);
        assert_eq!(route("/pkg/x/a.js"), Route::NotFound);
        assert_eq!(route("/pkg/1/a/b"), Route::NotFound);
    }

    #[test]
    fn frames_are_binary_and_whole() {
        let mut out = [0u8; ENVELOPE_MAX + 16];
        let capacity = out.len();
        let payload = [7u8; ENVELOPE_MAX];
        let written = {
            let mut writer: &mut [u8] = &mut out;
            block_on(send_frame(&mut writer, &payload)).unwrap();
            capacity - writer.len()
        };
        let mut reader: &[u8] = &out[..written];
        let header = block_on(FrameHeader::recv(&mut reader)).unwrap();
        assert_eq!(header.frame_type, FrameType::Binary(false));
        assert_eq!(header.payload_len as usize, ENVELOPE_MAX);
        assert_eq!(reader, &payload[..]);
    }
}
