use std::sync::{Arc, Mutex};
use std::thread;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;

use crate::engine_traits::{EventHandler, EventSleepCB};

type NotifyFn = dyn Fn(String) + Send + Sync;
type SenderFn = dyn Fn(&str) -> Result<&'static str, &'static str> + Send + Sync;


pub struct EngineContext {    
    sender: Mutex<Option<Box<SenderFn>>>,
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

    pub fn handle_event(&self, event: &str) -> Result<&'static str, &'static str> {
        let sender_lock = self.sender.lock().unwrap();
        if let Some(sender) = &*sender_lock {
            sender(event)
        } else {
            Err("No engine set")
        }
    }

    pub fn set_engine<T: EventHandler + Send + 'static>(&self, engine: T, notify_cb: Box<NotifyFn>) 
    where <T as EventHandler>::Event: Send {
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
                            (notify_cb)(serde_json::to_string(&result).unwrap());
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

        let sender_fn: Box<SenderFn> = Box::new(move |value: &str| {
            match serde_json::from_str(value) {
                Ok(event) => {
                    if sender.send(event).is_ok() {
                        Ok("Event scheduled")
                    } else {
                        Err("Failed to send send event")
                    }
                },
                Err(_) => Err("Failed to deserialize event"),
            }
        });
        let mut sender_lock = self.sender.lock().unwrap();
        *sender_lock = Some(sender_fn);
    }

    pub fn terminate(&self) {
        self.kill_channel.0.send(false).expect("Failed to send kill signal");

        let mut sender_lock = self.sender.lock().unwrap();
        *sender_lock = None;
    }

    /// helper to generate a sleep handler
    fn sleep<T: EventHandler + Send + 'static>(
        engine: Arc<Mutex<T>>, 
        notify_cb: Arc<NotifyFn>, 
        duration: u32, 
        cb: Box<EventSleepCB<T>>, 
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
                        (notify_cb)(serde_json::to_string(&result).unwrap());
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
