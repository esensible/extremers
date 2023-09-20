use std::sync::{Arc, Mutex};
use std::thread;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;

use crate::engine_traits::{Engine, SleepCB};

type NotifyFn = dyn Fn(String) + Send + Sync;

pub struct EngineContext {    
    sender: Mutex<Option<Sender<String>>>,
    kill_channel: (Sender<bool>, Arc<Receiver<bool>>),
}

impl Default for EngineContext {
    fn default() -> Self {
        let channel = crossbeam_channel::bounded(1);
        EngineContext {
            sender: Mutex::new(None),
            kill_channel: (channel.0, Arc::new(channel.1)),
        }
    }
}

impl EngineContext {

    pub fn handle_event(&self, event: String) {
        let sender_lock = self.sender.lock().unwrap();
        if let Some(ref sender) = *sender_lock {
            sender.send(event).expect("Failed to send event");
        }
    }

    pub fn set_engine<T: Engine + Send + 'static>(&self, engine: T, notify_cb: Box<NotifyFn>) {
        let engine = Arc::new(Mutex::new(engine));
        let notify_cb = Arc::new(notify_cb);

        let (sender, receiver) = crossbeam_channel::bounded(100);

        let sleep_fn = {
            let engine = engine.clone();
            let notify_cb = notify_cb.clone();
            let kill_receiver = self.kill_channel.1.clone();
            Box::new(move |duration, cb| EngineContext::sleep(engine.clone(), notify_cb.clone(), duration, cb, kill_receiver.clone()))
        };
    
        // intialize the main event loop
        {
            let engine = engine.clone();
            let notify_cb = notify_cb.clone();

            thread::spawn(move || {
                while let Ok(event) = receiver.recv() {
                    let mut sm = engine.lock().unwrap();
                    match sm.handle_event(event, &*sleep_fn) {
                        Ok(Some(result)) => {
                            (notify_cb)(result);
                        },
                        Ok(None) => {},
                        Err(_error_msg) => {
                            // TODO: We don't want to notify everybody... just the caller
                            // (notify_cb)(result);
                        }
                    }
                }
            });
        };

        let mut sender_lock = self.sender.lock().unwrap();
        *sender_lock = Some(sender);
    }

    pub fn terminate(&self) {
        self.kill_channel.0.send(false).expect("Failed to send kill signal");

        let mut sender_lock = self.sender.lock().unwrap();
        *sender_lock = None;
    }

    /// helper to generate a sleep handler
    fn sleep<T: Engine + Send + 'static>(
        engine: Arc<Mutex<T>>, 
        notify_cb: Arc<NotifyFn>, 
        duration: u32, 
        cb: Box<SleepCB<T>>, 
        kill_receiver: Arc<Receiver<bool>>
    ) {
        thread::spawn(move || {
            // NOTE: Ok response means we received a kill signal
            match kill_receiver.recv_timeout(std::time::Duration::from_millis(duration as u64)) {
                Ok(false) => {
                    // just exit
                }, 
                Ok(true) => {
                    // invoke callback, but don't notify
                    let mut sm = engine.lock().unwrap();
                    let _ = cb(&mut *sm);
                },
                Err(RecvTimeoutError::Timeout) => {
                    // nominal case -> we slept, we woke up
                    let mut sm = engine.lock().unwrap();
                    if let Some(result) = cb(&mut *sm) {
                        (notify_cb)(result);
                    }
                },
                Err(RecvTimeoutError::Disconnected) => {
                    // wtf?
                }
            }
        });
    }
}


impl Drop for EngineContext {
    fn drop(&mut self) {
        self.terminate();
    }
}

// implement a test like this
// let notify_cb = |msg| println!("notify: {}", msg);
// let engine = Arc::new(EngineContext::new());
// engine.set_engine(RaceEngine { data: 0 }, Box::new(notify_cb));

// engine.handle_event("hi".into());
// engine.handle_event("sleep(3)".into());

// let wc = engine.clone();
// thread::spawn(
//     move || {
//         wc.handle_event("inc".into());
//     }
// );

// engine.handle_event("other_event".into());
// thread::sleep(std::time::Duration::from_secs(5));

