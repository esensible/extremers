use core::sync::atomic::Ordering;
use embassy_net::tcp::TcpSocket;

// traits used
use embassy_sync::pubsub::PubSubBehavior;
use embedded_io_async::Write;

use engine_race::RaceHttpd;
use lib_httpd::Response;

use crate::consts::*;

fn sleep_fn(timeout: u64, callback: usize) -> Result<(), &'static str> {
    // Confirm we have a GPS reading first
    // Critical because pending sleeps are not adjusted on GPS time updates
    let offset: u64 = {
        let offset_msb = OFFSET_MSB.load(Ordering::Relaxed) as u64;
        let offset_lsb = OFFSET_LSB.load(Ordering::Relaxed) as u64;

        (offset_msb << 32) + offset_lsb
    };

    if offset == 0 {
        return Err("No GPS time");
    }

    let message = SleepMessage {
        wake_time: timeout,
        callback,
    };

    SLEEP_BUS.publish_immediate(message);
    Ok(())
}

pub async fn httpd_task_impl(
    httpd_mutex: &'static embassy_sync::mutex::Mutex<
        embassy_sync::blocking_mutex::raw::ThreadModeRawMutex,
        RaceHttpd,
    >,
    stack: &'static embassy_net::Stack<'_>,
) -> ! {
    let mut rx_buffer = [0; RX_BUF_SIZE];
    let mut tx_buffer = [0; TX_BUF_SIZE];
    let mut read_buffer = [0; READ_BUF_SIZE];
    let mut response_buffer = [0; RESPONSE_BUF_SIZE];
    let mut update = UpdateMessage::default();

    loop {
        let mut socket = TcpSocket::new(*stack, &mut rx_buffer, &mut tx_buffer);
        // socket.set_timeout(Some(Duration::from_secs(10)));

        // log::info!("Listening...");
        if let Err(e) = socket.accept(PORT).await {
            log::warn!("accept error: {:?}", e);
            continue;
        }

        // log::info!("Connect");

        let mut partial_offs = 0;
        loop {
            match socket.read(&mut read_buffer[partial_offs..]).await {
                Ok(0) => {
                    // log::warn!("read EOF");
                    break;
                }
                Ok(n) => {
                    let response = {
                        let mut engine = httpd_mutex.lock().await;

                        let offset: u64 = {
                            let offset_msb = OFFSET_MSB.load(Ordering::Relaxed) as u64;
                            let offset_lsb = OFFSET_LSB.load(Ordering::Relaxed) as u64;

                            (offset_msb << 32) + offset_lsb
                        };

                        let now = embassy_time::Instant::now().as_millis() + offset;

                        // log::info!("offset: {:?}", core::str::from_utf8(&read_buffer[..n]));
                        (*engine).handle_request(
                            now,
                            &read_buffer[..partial_offs + n],
                            &mut response_buffer,
                            &mut update.0,
                            &mut sleep_fn,
                        )
                    };

                    if let Err(len) = response {
                        log::warn!("handle_request error: {:?}", len);
                        let result = socket.write_all(&response_buffer[..len]).await;
                        if result.is_err() {
                            log::warn!("write error: {:?}", result);
                            break;
                        }
                        partial_offs = 0;
                        continue;
                    }

                    let response = response.unwrap();

                    match response {
                        Response::Partial(_to_go) => {
                            partial_offs = n;
                            continue;
                        }

                        Response::Complete(r_len, up_len, ex) => {
                            // log::info!("handle_request -> {:?}, {:?}", r_len, up_len);

                            if let Some(r_len) = r_len {
                                let result = socket.write_all(&response_buffer[..r_len]).await;
                                if result.is_err() {
                                    log::warn!("write error: {:?}", result);
                                    break;
                                }
                            }
                            if let Some(ex) = ex {
                                let result = socket.write_all(ex).await;
                                if result.is_err() {
                                    log::warn!("write error: {:?}", result);
                                    break;
                                }
                            }
                            if let Some(up_len) = up_len {
                                update.1 = up_len;
                                UPDATES_BUS.publish_immediate(update.clone());
                            }
                        }
                        Response::None => {
                            // log::info!("handle_request -> None");
                            let mut message_subscriber = UPDATES_BUS.dyn_subscriber().unwrap();
                            match embassy_time::with_timeout(
                                embassy_time::Duration::from_secs(5),
                                message_subscriber.next_message_pure(),
                            )
                            .await
                            {
                                Ok(message) => {
                                    // log::info!("update: {:?}", message.1);
                                    let result = socket.write_all(&message.0[..message.1]).await;
                                    if result.is_err() {
                                        log::warn!("write error: {:?}", result);
                                        break;
                                    }
                                }
                                Err(_) => {
                                    let result =
                                        socket.write_all(b"HTTP/1.1 204 Timeout\r\n\r\n").await;
                                    if result.is_err() {
                                        log::warn!("write error: {:?}", result);
                                        break;
                                    }
                                }
                            }
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
