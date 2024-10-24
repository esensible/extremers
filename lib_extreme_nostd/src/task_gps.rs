use core::sync::atomic::Ordering;
#[allow(unused_imports)]
use embassy_sync::pubsub::PubSubBehavior;

use engine_race::RaceHttpd;

use crate::consts::{UpdateMessage, OFFSET_LSB, OFFSET_MSB, UPDATES_BUS};
use crate::nmea_parser::{next_update, Tokeniser};

pub async fn gps_task_impl<T>(
    httpd_mutex: &'static embassy_sync::mutex::Mutex<
        embassy_sync::blocking_mutex::raw::ThreadModeRawMutex,
        RaceHttpd,
    >,
    tokeniser: &mut T,
) where
    T: Tokeniser,
{
    let mut offset: u64 = 0;

    loop {
        let (time, location, speed) = next_update(tokeniser).await;

        if let Some(time) = &time {
            let uptime = embassy_time::Instant::now().as_millis() as u64;
            {
                if OFFSET_MSB.load(Ordering::Relaxed) == 0 {
                    offset = time - uptime;
                    let offset_msb = (offset >> 32) as u32;
                    let offset_lsb = (offset & 0xFFFF_FFFF) as u32;

                    OFFSET_MSB.store(offset_msb, Ordering::Relaxed);
                    OFFSET_LSB.store(offset_lsb, Ordering::Relaxed);
                }
            }
        }

        let timestamp = offset + embassy_time::Instant::now().as_millis() as u64;
        let mut update = UpdateMessage::default();

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
            UPDATES_BUS.publisher().unwrap().publish_immediate(update.clone());
        }
    }
}
