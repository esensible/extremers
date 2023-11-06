use core::sync::atomic::Ordering;
use embassy_rp::peripherals::UART1;
use embassy_rp::uart::{Async, UartRx};
use embassy_sync::pubsub::PubSubBehavior;
use embassy_time::{Duration, Timer};

use engine_race::RaceHttpd;

use crate::consts::{UpdateMessage, OFFSET_LSB, OFFSET_MSB, UPDATES_BUS};
use crate::nmea_parser::{NMEAMessage, NMEAParser};

#[embassy_executor::task]
pub async fn gps_task(
    httpd_mutex: &'static embassy_sync::mutex::Mutex<
        embassy_sync::blocking_mutex::raw::ThreadModeRawMutex,
        RaceHttpd,
    >,
    rx: UartRx<'static, UART1, Async>,
) {
    log::info!("Reading...");

    let mut parser = NMEAParser::<32>::new(rx);
    loop {
        let token = parser.next_token().await;
        static mut OFFSET: u64 = 0;
        match token {
            Some(NMEAMessage::GNRMC(gnrmc)) => {
                if let (Some(time), Some(date)) = (&gnrmc.utc_time, &gnrmc.date) {
                    unsafe {
                        if OFFSET_MSB.load(Ordering::Relaxed) == 0 {
                            let gps_now = *time as u64 + *date as u64 * 24 * 60 * 60_000;
                            let uptime = embassy_time::Instant::now().as_millis() as u64;
                            OFFSET = gps_now - uptime;
                            let offset_msb = (OFFSET >> 32) as u32;
                            let offset_lsb = (OFFSET & 0xFFFF_FFFF) as u32;

                            OFFSET_MSB.store(offset_msb, Ordering::Relaxed);
                            OFFSET_LSB.store(offset_lsb, Ordering::Relaxed);
                        }
                    }
                }

                let location = if let (Some(latitude), Some(ew), Some(longitude), Some(ns)) = (
                    &gnrmc.latitude,
                    &gnrmc.ew_indicator,
                    &gnrmc.longitude,
                    &gnrmc.ns_indicator,
                ) {
                    let latitude_final = if *ns == 'S' {
                        -1.0 * latitude
                    } else {
                        *latitude
                    };

                    let longitude_final = if *ew == 'W' {
                        -1.0 * longitude
                    } else {
                        *longitude
                    };

                    Some((latitude_final, longitude_final))
                } else {
                    None
                };

                let speed = if let (Some(speed), Some(course)) =
                    (&gnrmc.speed_over_ground, &gnrmc.course_over_ground)
                {
                    Some((*speed, *course))
                } else {
                    None
                };

                let mut update = UpdateMessage::default();

                let timestamp = unsafe { OFFSET } + embassy_time::Instant::now().as_millis() as u64;

                let len = {
                    let mut engine = httpd_mutex.lock().await;

                    (*engine).update_location(timestamp, location, speed, &mut update.0)
                };

                if let Some(len) = len {
                    // log::info!(
                    //     "{}, {:?}",
                    //     len,
                    //     core::str::from_utf8(&update.0[..len]).unwrap()
                    // );

                    update.1 = len;
                    UPDATES_BUS.publish_immediate(update.clone());
                }
            }
            Some(NMEAMessage::Unknown) => {
                log::info!("Unknown");
            }
            None => {
                Timer::after(Duration::from_millis(100)).await;
                // log::info!("None");
            }
        }
    }
}
