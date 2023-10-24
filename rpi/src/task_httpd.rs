use core::sync::atomic::Ordering;
use embassy_net::tcp::TcpSocket;
use embassy_time::Duration;

// traits used
use embassy_sync::pubsub::PubSubBehavior;
use embedded_io_async::Write;

use lib_httpd::{EngineHttpdTrait, RaceHttpd, Response};

use crate::consts::*;

fn sleep_fn(timeout: usize, callback: usize) {
    let message = SleepMessage {
        time: timeout,
        callback,
    };
    SLEEP_BUS.publish_immediate(message);
}

#[embassy_executor::task(pool_size = MAX_SOCKETS)]
pub async fn httpd_task(
    httpd_mutex: &'static embassy_sync::mutex::Mutex<
        embassy_sync::blocking_mutex::raw::ThreadModeRawMutex,
        RaceHttpd,
    >,
    stack: &'static embassy_net::Stack<cyw43::NetDriver<'static>>,
) -> ! {
    let mut rx_buffer = [0; RX_BUF_SIZE];
    let mut tx_buffer = [0; TX_BUF_SIZE];
    let mut read_buffer = [0; READ_BUF_SIZE];
    let mut response_buffer = [0; RESPONSE_BUF_SIZE];
    let mut update = UpdateMessage::default();

    loop {
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        // socket.set_timeout(Some(Duration::from_secs(10)));

        log::info!("Listening...");
        if let Err(e) = socket.accept(PORT).await {
            log::warn!("accept error: {:?}", e);
            continue;
        }

        log::info!("Connect");

        let mut partial_offs = 0;
        loop {
            match socket.read(&mut read_buffer[partial_offs..]).await {
                Ok(0) => {
                    log::warn!("read EOF");
                    break;
                }
                Ok(n) => {
                    unsafe {
                        log::info!("Received");
                    }

                    let response = {
                        let mut engine = httpd_mutex.lock().await;

                        let offset: u64 = unsafe {
                            let offset_msb = OFFSET_MSB.load(Ordering::Relaxed) as u64;
                            let offset_lsb = OFFSET_LSB.load(Ordering::Relaxed) as u64;

                            (offset_msb << 32) + offset_lsb
                        };

                        let now = embassy_time::Instant::now().as_millis() + offset;

                        (*engine).handle_request(
                            now,
                            &read_buffer[..partial_offs + n],
                            &mut response_buffer,
                            &mut update.0,
                            &sleep_fn,
                        )
                    };

                    if let Err(len) = response {
                        log::warn!("handle_request error: {:?}", len);
                        socket.write_all(&response_buffer[..len]).await;
                        partial_offs = 0;
                        continue;
                    }

                    let response = response.unwrap();

                    match response {
                        Response::Partial(to_go) => {
                            partial_offs = n;
                            continue;
                        }

                        Response::Complete(r_len, up_len, ex) => {
                            log::info!("handle_request -> {:?}, {:?}", r_len, up_len);

                            if let Some(r_len) = r_len {
                                socket.write_all(&response_buffer[..r_len]).await;
                            }
                            if let Some(ex) = ex {
                                socket.write_all(ex).await;
                            }
                            if let Some(up_len) = up_len {
                                update.1 = up_len;
                                UPDATES_BUS.publish_immediate(update.clone());
                            }
                        }
                        Response::None => {
                            log::info!("handle_request -> None");
                            let mut message_subscriber = UPDATES_BUS.dyn_subscriber().unwrap();
                            match embassy_time::with_timeout(
                                Duration::from_secs(5),
                                message_subscriber.next_message_pure(),
                            )
                            .await
                            {
                                Ok(message) => {
                                    log::info!("update: {:?}", message.1);
                                    socket.write_all(&message.0[..message.1]).await;
                                }
                                Err(_) => {
                                    socket.write_all(b"HTTP/1.1 204 Timeout\r\n\r\n").await;
                                }
                            }
                        }
                        _ => {
                            log::warn!("Invalid response type");
                            break;
                        }
                    }
                }
                Err(e) => {
                    log::warn!("read error: {:?}", e);
                    break;
                }
            };
            partial_offs = 0;
        }
    }
}
