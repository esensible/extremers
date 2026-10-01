// Standard library imports
use core::{
    fmt::{Debug, Display, Write as _},
    sync::atomic::Ordering,
};

// Embassy framework imports
use embassy_futures::select::{Either, select};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pubsub::PubSubChannel};
use embassy_time::{Duration, Timer};

// Networking imports
use edge_http::{
    Method,
    io::{
        Error,
        server::{Connection, Handler},
    },
    ws::MAX_BASE64_KEY_RESPONSE_LEN,
};
use edge_nal::TcpSplit;
use edge_ws::{Error as WsError, FrameHeader, FrameType};

// Other external crates
use embedded_io_async::{Read, Write};
use heapless::Vec;
// use panic_probe as _;
use portable_atomic::AtomicU64;

use extreme_traits::RawEngine;

// Constants
pub const MAX_MESSAGE_SIZE: usize = 512;
pub const MAX_WEB_SOCKETS: usize = 4;
pub const SOCKET_BUFFER_SIZE: usize = MAX_MESSAGE_SIZE * 4;

// Type aliases
type UpdateMessage = Vec<u8, MAX_MESSAGE_SIZE>;

pub struct HttpHandler<Engine>
where
    Engine: RawEngine,
{
    engine: embassy_sync::mutex::Mutex<CriticalSectionRawMutex, Engine>,
    tick_offset: AtomicU64,
    sleep_channel: PubSubChannel<CriticalSectionRawMutex, u64, 1, 4, 4>,
    broadcast_channel: PubSubChannel<CriticalSectionRawMutex, UpdateMessage, 1, 4, 4>,
}

impl<Engine> HttpHandler<Engine>
where
    Engine: extreme_traits::RawEngine,
{
    pub fn new(engine: Engine) -> Self {
        Self {
            broadcast_channel: PubSubChannel::new(),
            sleep_channel: PubSubChannel::new(),
            engine: embassy_sync::mutex::Mutex::new(engine),
            tick_offset: AtomicU64::new(0),
        }
    }

    /// Current time in the same epoch-millisecond units as GPS timestamps.
    fn now(&self) -> u64 {
        embassy_time::Instant::now().as_millis() + self.tick_offset.load(Ordering::Relaxed)
    }

    pub async fn location_event(
        &self,
        time: Option<u64>,
        location: Option<(f64, f64)>,
        speed: Option<(f64, f64)>,
    ) {
        // log::info!("location_event: {:?}, {:?}, {:?}", time, location, speed);
        let timestamp = match time {
            Some(timestamp) => {
                // the first GPS time fixes the offset from uptime to epoch time
                if self.tick_offset.load(Ordering::Relaxed) == 0 {
                    let uptime = embassy_time::Instant::now().as_millis();
                    self.tick_offset
                        .store(timestamp.saturating_sub(uptime), Ordering::Relaxed);
                }
                timestamp
            }
            None => self.now(),
        };

        let mut engine = self.engine.lock().await;
        let (update, timer) = (*engine).location_event(timestamp, location, speed);

        // handle state update if there was one
        if let Some(()) = update {
            // log::info!("broadcasting state update");

            match (*engine).to_vec() {
                Ok(message) => {
                    if let Ok(publisher) = self.broadcast_channel.publisher() {
                        publisher.publish_immediate(message);
                    } else {
                        log::error!("Failed to get broadcast channel publisher");
                        return;
                    }
                }
                Err(_) => {
                    log::error!("Failed to serialize engine state");
                    return;
                }
            }
        }

        // handle sleep timer if there was one
        if let Some(timer) = timer {
            if let Ok(publisher) = self.sleep_channel.publisher() {
                publisher.publish_immediate(timer);
            } else {
                log::error!("Failed to get sleep channel publisher");
                return;
            }
        }
    }

    pub async fn run_sleeper(&self) -> ! {
        let mut sleep_time: Option<u64> = None;

        loop {
            let mut subscriber = match self.sleep_channel.dyn_subscriber() {
                Ok(sub) => sub,
                Err(_) => {
                    log::error!("Failed to get sleep channel subscriber");
                    Timer::after(Duration::from_secs(10)).await;
                    continue;
                }
            };

            match sleep_time {
                // just chillen, with nothin to do
                None => {
                    sleep_time = Some(subscriber.next_message_pure().await);
                    log::info!("dude, you have a job");
                }

                // we have a sleep scheduled
                Some(wake_time) => {
                    // so sleep!
                    // convert absolute wake time to a duration
                    let now = self.now();
                    let sleep_ms = if wake_time > now { wake_time - now } else { 0 };

                    log::info!("sleeping for {} ms", sleep_ms);
                    match embassy_time::with_timeout(
                        embassy_time::Duration::from_millis(sleep_ms),
                        subscriber.next_message_pure(),
                    )
                    .await
                    {
                        // sleep was terminated early
                        Ok(message) => {
                            log::info!("sleep terminated early: {}", message);
                            sleep_time = Some(message);
                        }

                        //
                        // !!!! sleep timed out - nominal case !!!!
                        //
                        Err(_timeout_error) => {
                            // log::info!("Yay: sleep timed out");
                            let mut engine = self.engine.lock().await;
                            let (update, timer) = (*engine).timer_event(wake_time);

                            // handle state update if there was one
                            if let Some(()) = update {
                                // log::info!("broadcasting state update");
                                match (*engine).to_vec() {
                                    Ok(message) => {
                                        if let Ok(publisher) = self.broadcast_channel.publisher() {
                                            publisher.publish_immediate(message);
                                        } else {
                                            log::error!(
                                                "Failed to get broadcast channel publisher"
                                            );
                                        }
                                    }
                                    Err(_) => {
                                        log::error!("Failed to serialize engine state");
                                    }
                                }
                            }

                            // next sleep timer, if required
                            sleep_time = timer;
                        }
                    }
                }
            }
        }
    }
}

impl<Engine> Handler for HttpHandler<Engine>
where
    Engine: extreme_traits::RawEngine,
{
    type Error<E>
        = Error<E>
    where
        E: Debug;

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
            let path = if headers.path == "/" || headers.path == "" {
                "index.html"
            } else if headers.path.starts_with('/') {
                &headers.path[1..]
            } else {
                headers.path
            };

            log::info!("serving static file: {}", path);
            // files are 'static, so release the engine before the (slow) write
            let file = self.engine.lock().await.get_static(path);
            if let Some(file) = file {
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

            log::info!("Connection upgraded to WS");

            // Now we have the TCP socket in a state where it can be operated as a WS connection

            let mut socket = conn.unbind()?;

            // send the current state to the client immediately
            let vec = {
                let engine = self.engine.lock().await;
                (*engine).to_vec()
            };

            // scoped so we release the lock ASAP
            match vec {
                Ok(message) => {
                    if let Err(e) = send_state(&mut socket, self.now(), &message).await {
                        log::error!("Failed to send state: {:?}", e);
                    }
                }
                Err(e) => {
                    log::error!("Failed to serialize engine state: {:?}", e);
                }
            }

            let mut subscriber = match self.broadcast_channel.dyn_subscriber() {
                Ok(s) => s,
                Err(e) => {
                    log::error!("Failed to create broadcast subscriber: {:?}", e);
                    return Ok(());
                }
            };

            loop {
                let header_future = FrameHeader::recv(&mut socket);
                let subscriber_future = subscriber.next_message_pure();

                match select(header_future, subscriber_future).await {
                    Either::First(header_result) => {
                        let header = match header_result {
                            Ok(h) => h,
                            Err(e) => {
                                log::error!("Failed to receive header: {:?}", e);
                                break;
                            }
                        };

                        match header.frame_type {
                            FrameType::Close => {
                                log::info!("Client closed connection");
                                break;
                            }
                            FrameType::Ping => {
                                log::info!("Sending pong");
                                let header = FrameHeader {
                                    mask_key: None,
                                    frame_type: FrameType::Pong,
                                    payload_len: 0,
                                };

                                if let Err(e) = header.send(&mut socket).await {
                                    log::error!("Failed to send pong: {:?}", e);
                                    break;
                                }
                                continue;
                            }
                            _ => {
                                log::info!("Got {header}");
                            }
                        }

                        // Deserialize the payload into an Engine::Event
                        let mut buf = [0_u8; MAX_MESSAGE_SIZE];
                        let payload = match header.recv_payload(&mut socket, &mut buf).await {
                            Ok(p) => p,
                            Err(e) => {
                                log::error!("Failed to receive payload: {:?}", e);
                                break;
                            }
                        };

                        // log::info!(
                        //     "payload: {}",
                        //     core::str::from_utf8(payload).unwrap_or("<invalid utf8>")
                        // );

                        let (update, timer) = {
                            let mut engine = self.engine.lock().await;
                            let now = self.now();

                            // handle the event
                            match RawEngine::external_event(&mut *engine, now, payload) {
                                Ok(result) => result,
                                Err(_) => {
                                    log::error!("Failed to handle external event");
                                    break;
                                }
                            }
                        };

                        // handle state update if there was one
                        if let Some(update) = update {
                            if let Ok(publisher) = self.broadcast_channel.publisher() {
                                publisher.publish_immediate(update);
                            } else {
                                log::error!("Failed to get broadcast channel publisher");
                                break;
                            }
                        }

                        if let Some(timer) = timer {
                            if let Ok(publisher) = self.sleep_channel.publisher() {
                                publisher.publish_immediate(timer);
                            } else {
                                log::error!("Failed to get sleep channel publisher");
                            }
                        }
                    }
                    Either::Second(message) => {
                        // send the message to the client
                        // break on any comms error
                        // log::info!("broadcast message");

                        if let Err(e) = send_state(&mut socket, self.now(), &message).await {
                            log::error!("Failed to send state: {:?}", e);
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Sends `{"timestamp":<timestamp>,"engine":<message>}` as a single websocket
/// text frame. `message` is already serialized JSON. The pieces are written
/// straight to the socket, so there is no intermediate buffer to overflow.
async fn send_state<W>(
    socket: &mut W,
    timestamp: u64,
    message: &[u8],
) -> Result<(), WsError<W::Error>>
where
    W: Write,
{
    const PREFIX: &[u8] = b"{\"timestamp\":";
    const MIDDLE: &[u8] = b",\"engine\":";
    const SUFFIX: &[u8] = b"}";

    let mut digits: heapless::String<20> = heapless::String::new();
    // a u64 has at most 20 digits, so this can't fail
    let _ = write!(digits, "{}", timestamp);

    let header = FrameHeader {
        mask_key: None,
        // `false`: not fragmented, this frame is the whole message
        frame_type: FrameType::Text(false),
        payload_len: (PREFIX.len() + digits.len() + MIDDLE.len() + message.len() + SUFFIX.len())
            as u64,
    };
    header.send(&mut *socket).await?;

    // server frames are not masked, so the payload can be written in pieces
    for piece in [PREFIX, digits.as_bytes(), MIDDLE, message, SUFFIX] {
        socket.write_all(piece).await.map_err(WsError::Io)?;
    }
    socket.flush().await.map_err(WsError::Io)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use embassy_futures::block_on;

    #[test]
    fn send_state_handles_max_size_message() {
        let mut message = [b'x'; MAX_MESSAGE_SIZE];
        message[0] = b'"';
        message[MAX_MESSAGE_SIZE - 1] = b'"';

        let mut out = [0u8; MAX_MESSAGE_SIZE + 128];
        let capacity = out.len();
        let written = {
            let mut writer: &mut [u8] = &mut out;
            block_on(send_state(&mut writer, u64::MAX, &message)).unwrap();
            capacity - writer.len()
        };

        let mut reader: &[u8] = &out[..written];
        let header = block_on(FrameHeader::recv(&mut reader)).unwrap();
        assert_eq!(header.frame_type, FrameType::Text(false));
        assert_eq!(header.payload_len as usize, reader.len());

        let expected = std::format!(
            "{{\"timestamp\":{},\"engine\":{}}}",
            u64::MAX,
            core::str::from_utf8(&message).unwrap()
        );
        assert_eq!(reader, expected.as_bytes());
    }
}
