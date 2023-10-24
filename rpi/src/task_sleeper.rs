use embassy_sync::pubsub::PubSubBehavior;
use embassy_time::{with_timeout, Duration};

use lib_httpd::{EngineHttpdTrait, RaceHttpd};

use crate::consts::{
    SleepMessage, UpdateMessage, MAX_SOCKETS, SLEEP_BUS, UPDATES_BUS, UPDATE_BUF_SIZE,
};

#[embassy_executor::task]
pub async fn sleeper_task(
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
            }

            // we have a sleep scheduled
            Some(message) => {
                // so sleep!
                match with_timeout(
                    Duration::from_secs(message.time as u64),
                    message_subscriber.next_message_pure(),
                )
                .await
                {
                    // sleep was terminated early
                    Ok(message) => {
                        sleep_time = Some(message);
                    }

                    //
                    // !!!! sleep timed out - nominal case !!!!
                    //
                    Err(TimeoutError) => {
                        let mut buffer = [0; UPDATE_BUF_SIZE];
                        let result = {
                            let mut engine = httpd_mutex.lock().await;
                            (*engine).handle_sleep(buffer, message.callback)
                        };
                        if let Some(len) = result {
                            let update = UpdateMessage(buffer, len);
                            UPDATES_BUS.publish_immediate(update);
                        }
                        sleep_time = None;
                    }

                    // something not good happened
                    Err(_) => {
                        log::warn!("Error waiting on sleep bus");
                        sleep_time = None;
                    }
                }
            }
        }
    }
}
