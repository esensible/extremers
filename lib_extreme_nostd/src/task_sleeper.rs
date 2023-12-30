use core::sync::atomic::Ordering;
// use the trait
use embassy_sync::pubsub::PubSubBehavior;

use engine_race::RaceHttpd;

use crate::consts::{
    SleepMessage, UpdateMessage, OFFSET_LSB, OFFSET_MSB, SLEEP_BUS, UPDATES_BUS, UPDATE_BUF_SIZE,
};

pub async fn sleeper_task_impl(
    httpd_mutex: &'static embassy_sync::mutex::Mutex<
        embassy_sync::blocking_mutex::raw::ThreadModeRawMutex,
        RaceHttpd,
    >,
) {
    let mut sleep_time: Option<SleepMessage> = None;

    loop {
        let mut message_subscriber = SLEEP_BUS.dyn_subscriber().unwrap();

        match sleep_time {
            // just chillen, with nothin to do
            None => {
                sleep_time = Some(message_subscriber.next_message_pure().await);
                // log::info!("dude, you have a job");
            }

            // we have a sleep scheduled
            Some(message) => {
                // so sleep!
                // convert absolute wake time to a duration
                let offset: u64 = unsafe {
                    let offset_msb = OFFSET_MSB.load(Ordering::Relaxed) as u64;
                    let offset_lsb = OFFSET_LSB.load(Ordering::Relaxed) as u64;

                    (offset_msb << 32) + offset_lsb
                };

                let now = embassy_time::Instant::now().as_millis() + offset;
                let wake_time = message.wake_time;
                let sleep_ms = if wake_time > now { wake_time - now } else { 0 };

                // log::info!("sleeping for {} ms", sleep_ms);
                match embassy_time::with_timeout(
                    embassy_time::Duration::from_millis(sleep_ms),
                    message_subscriber.next_message_pure(),
                )
                .await
                {
                    // sleep was terminated early
                    Ok(message) => {
                        log::info!("sleep terminated early: {}", message.wake_time);
                        sleep_time = Some(message);
                    }

                    //
                    // !!!! sleep timed out - nominal case !!!!
                    //
                    Err(_timeout_error) => {
                        // log::info!("sleep timed out");

                        let mut buffer = [0u8; UPDATE_BUF_SIZE];
                        let result = {
                            let mut engine = httpd_mutex.lock().await;
                            (*engine).handle_sleep(&mut buffer, message.callback)
                        };
                        if let Some(len) = result {
                            let update = UpdateMessage(buffer, len);
                            UPDATES_BUS.publish_immediate(update);
                        }
                        sleep_time = None;
                    }
                }
            }
        }
    }
}
