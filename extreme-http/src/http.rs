// Standard library imports
use core::{
    fmt::{Debug, Display},
    sync::atomic::Ordering,
};

// Embassy framework imports
use embassy_futures::select::{select, Either};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pubsub::PubSubChannel};
use embassy_time::{Duration, Timer};

// Networking imports
use edge_net::{
    http::{
        io::{
            server::{Connection, Handler},
            Error,
        },
        ws::MAX_BASE64_KEY_RESPONSE_LEN,
        Method,
    },
    ws::{FrameHeader, FrameType},
};

// Other external crates
use embedded_io_async::{Read, Write};
use heapless::Vec;
use panic_probe as _;
use portable_atomic::AtomicU64;

// Constants
pub const MAX_MESSAGE_SIZE: usize = 512;

// Type aliases
type UpdateMessage = Vec<u8, MAX_MESSAGE_SIZE>;

pub struct HttpHandler<Engine>
where
    Engine: extreme_traits::Engine,
{
    engine: embassy_sync::mutex::Mutex<CriticalSectionRawMutex, Engine>,
    tick_offset: AtomicU64,
    sleep_channel: PubSubChannel<CriticalSectionRawMutex, u64, 1, 2, 3>,
    broadcast_channel: PubSubChannel<CriticalSectionRawMutex, UpdateMessage, 1, 2, 3>,
}

impl<Engine> HttpHandler<Engine>
where
    Engine: extreme_traits::Engine,
{
    pub fn new(engine: Engine) -> Self {
        Self {
            broadcast_channel: PubSubChannel::new(),
            sleep_channel: PubSubChannel::new(),
            engine: embassy_sync::mutex::Mutex::new(engine),
            tick_offset: AtomicU64::new(0),
        }
    }

    pub async fn location_event(
        &self,
        time: Option<u64>,
        location: Option<(f64, f64)>,
        speed: Option<(f64, f64)>,
    ) {
        log::info!("location_event: {:?}, {:?}, {:?}", time, location, speed);
        let timestamp = match time {
            Some(timestamp) => {
                let mut offset = self.tick_offset.load(Ordering::Relaxed);
                let uptime = embassy_time::Instant::now().as_millis() as u64;
                if offset == 0 {
                    offset = timestamp - uptime;
                    self.tick_offset.store(offset, Ordering::Relaxed);
                }
                offset + timestamp
            }
            None => {
                self.tick_offset.load(Ordering::Relaxed)
                    + embassy_time::Instant::now().as_millis() as u64
            }
        };

        let mut engine = self.engine.lock().await;
        let (update, timer) = (*engine).location_event(timestamp, location, speed);

        // handle state update if there was one
        if let Some(()) = update {
            log::info!("broadcasting state update");

            match serde_json_core::to_vec(&*engine) {
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
                    let offset = self.tick_offset.load(Ordering::Relaxed);

                    let now = embassy_time::Instant::now().as_millis() + offset;
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
                            log::info!("Yay: sleep timed out");
                            let mut engine = self.engine.lock().await;
                            let (update, timer) = (*engine).timer_event(wake_time);

                            // handle state update if there was one
                            if let Some(()) = update {
                                log::info!("broadcasting state update");
                                match serde_json_core::to_vec(&*engine) {
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
    Engine: extreme_traits::Engine,
{
    type Error<E>
        = Error<E>
    where
        E: Debug;

    async fn handle<T, const N: usize>(
        &self,
        _task_id: impl Display + Clone,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write,
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
            let engine = self.engine.lock().await;
            if let Some(file) = (*engine).get_static(path) {
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
            {
                // scoped so we release the lock ASAP
                let engine = self.engine.lock().await;
                match serde_json_core::to_vec::<Engine, MAX_MESSAGE_SIZE>(&*engine) {
                    Ok(message) => {
                        // Build JSON wrapper manually since message is already serialized JSON
                        let mut wrapper = UpdateMessage::new();
                        wrapper.extend_from_slice(b"{\"timestamp\":").unwrap();
                        let timestamp = embassy_time::Instant::now().as_millis()
                            + self.tick_offset.load(Ordering::Relaxed);
                        if u64_to_heapless_vec(timestamp, &mut wrapper).is_err() {
                            log::error!("failed to render timestamp");
                        }
                        wrapper.extend_from_slice(b",\"engine\":").unwrap();
                        wrapper.extend_from_slice(&message).unwrap();
                        wrapper.extend_from_slice(b"}").unwrap();

                        let header = FrameHeader {
                            mask_key: None,
                            frame_type: FrameType::Text(false), // no clue why false is required, but it is
                            payload_len: wrapper.len() as u64,
                        };

                        if let Err(e) = header.send(&mut socket).await {
                            log::error!("Failed to send header: {:?}", e);
                        }

                        // Send the wrapped message
                        if let Err(e) = header.send_payload(&mut socket, wrapper.as_slice()).await {
                            log::error!("Failed to send payload: {:?}", e);
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to serialize engine state: {:?}", e);
                    }
                }
            }

            let mut subscriber = self.broadcast_channel.dyn_subscriber().unwrap();

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
                                log::info!("Got {header}, client closed the connection cleanly");
                                break;
                            }
                            FrameType::Ping => {
                                log::info!("Got {header}, sending pong");
                                let header = FrameHeader {
                                    mask_key: None,
                                    frame_type: FrameType::Pong,
                                    payload_len: 0,
                                };

                                if let Err(e) = header.send(&mut socket).await {
                                    log::error!("Failed to send header: {:?}", e);
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

                        let event = match serde_json_core::from_slice::<Engine::Event>(payload) {
                            Ok((event, _)) => event,
                            Err(e) => {
                                log::error!("Failed to deserialize event: {:?}", e);
                                break;
                            }
                        };

                        let mut engine = self.engine.lock().await;
                        // get the current time
                        let offset = self.tick_offset.load(Ordering::Relaxed);
                        let now = embassy_time::Instant::now().as_millis() + offset;
                        // handle the event
                        let (update, timer) = (*engine).external_event(now, event);

                        // handle state update if there was one
                        if let Some(()) = update {
                            match serde_json_core::to_vec(&*engine) {
                                Ok(message) => {
                                    if let Ok(publisher) = self.broadcast_channel.publisher() {
                                        publisher.publish_immediate(message);
                                    } else {
                                        log::error!("Failed to get broadcast channel publisher");
                                    }
                                }
                                Err(_) => {
                                    log::error!("Failed to serialize engine state");
                                }
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
                        log::info!("broadcast message");

                        // Build JSON wrapper manually since message is already serialized JSON
                        let mut wrapper = UpdateMessage::new();
                        wrapper.extend_from_slice(b"{\"timestamp\":").unwrap();
                        let timestamp = embassy_time::Instant::now().as_millis()
                            + self.tick_offset.load(Ordering::Relaxed);
                        if u64_to_heapless_vec(timestamp, &mut wrapper).is_err() {
                            log::error!("failed to render timestamp");
                        }
                        wrapper.extend_from_slice(b",\"engine\":").unwrap();
                        wrapper.extend_from_slice(&message).unwrap();
                        wrapper.extend_from_slice(b"}").unwrap();

                        let header = FrameHeader {
                            mask_key: None,
                            frame_type: FrameType::Text(false), // no clue why false is required, but it is
                            payload_len: wrapper.len() as u64,
                        };

                        if let Err(e) = header.send(&mut socket).await {
                            log::error!("Failed to send header: {:?}", e);
                        }

                        if let Err(e) = header.send_payload(&mut socket, wrapper.as_slice()).await {
                            log::error!("Failed to send payload: {:?}", e);
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

fn u64_to_heapless_vec<const N: usize>(mut num: u64, vec: &mut Vec<u8, N>) -> Result<(), ()> {
    if num == 0 {
        vec.extend_from_slice(&[b'0'])?;
        return Ok(());
    }

    // Convert number to digits in reverse
    let mut rev_digits = [0u8; 20]; // Max u64 length
    let mut rev_idx = 0;
    while num > 0 {
        rev_digits[rev_idx] = (num % 10) as u8 + b'0';
        num /= 10;
        rev_idx += 1;
    }

    // Extend vec with digits in correct order
    while rev_idx > 0 {
        rev_idx -= 1;
        if vec.push(rev_digits[rev_idx]).is_err() {
            return Err(());
        }
    }

    return Ok(());
}
