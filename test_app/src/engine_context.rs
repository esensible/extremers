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
        let _ = self.kill_channel.0.try_send(false);       
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
        assert_eq!(duration, 100);

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


#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;

    // Mock EventHandler for testing
    struct MockEventHandler;

    impl EventHandler for MockEventHandler {
        type Event = String;
        type Update = Self::Event;

        fn handle_event(&mut self, event: Self::Event, sleep_fn: &dyn Fn(u32, Box<EventSleepCB<Self>>)) -> Result<Option<Self::Event>, &'static str> {
            if event == "error" {
                Err("Event error")
            } else if event == "sleep100" {
                sleep_fn(100, Box::new(|_engine| Some("slept100".to_string())));
                Ok(None)
            } else {
                Ok(Some(event))
            }
        }
    }

    #[test]
    fn test_default() {
        let ctx = EngineContext::default();

        // Initially, no sender is set
        assert!(ctx.sender.lock().unwrap().is_none());
    }

    #[test]
    fn test_handle_event_no_engine() {
        let ctx = EngineContext::default();
        assert_eq!(ctx.handle_event("event"), Err("No engine set"));
    }

    #[test]
    fn test_set_engine_and_handle_event() {
        let ctx = EngineContext::default();
    
        let (notify_sender, notify_receiver) = unbounded::<String>();
    
        let notify_fn = move |event: String| {
            notify_sender.send(event).unwrap();
        };
    
        ctx.set_engine(MockEventHandler, Box::new(notify_fn));
    
        assert_eq!(ctx.handle_event("\"hello\""), Ok("Event scheduled"));
    
        // Introducing sleep to allow the spawned thread to process the message.
        std::thread::sleep(std::time::Duration::from_millis(100));
    
        assert_eq!(notify_receiver.recv().unwrap(), "\"hello\"");
    
        assert_eq!(ctx.handle_event("not JSON"), Err("Failed to deserialize event"));
    
        // Introducing sleep again for the same reason.
        std::thread::sleep(std::time::Duration::from_millis(100));
    
        // Here you could check for the expected outcome for the "error" event, if any.
        // For now, since we don't test that outcome in this test, we skip it.
    }
    
    #[test]
    fn test_terminate() {
        let ctx = EngineContext::default();
        let (notify_sender, notify_receiver) = unbounded::<String>();

        let notify_fn = move |result: String| {
            notify_sender.send(result).unwrap();
        };

        ctx.set_engine(MockEventHandler, Box::new(notify_fn));

        assert_eq!(ctx.handle_event("\"hello\""), Ok("Event scheduled"));
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(notify_receiver.recv().unwrap(), "\"hello\"");

        assert_eq!(ctx.handle_event("\"sleep100\""), Ok("Event scheduled"));
        std::thread::sleep(std::time::Duration::from_millis(250));
        assert_eq!(notify_receiver.recv().unwrap(), "\"slept100\"");

        // schedule another sleep and terminate immediately
        assert_eq!(ctx.handle_event("\"sleep100\""), Ok("Event scheduled"));
        ctx.terminate();
        assert!(matches!(notify_receiver.recv(), Err(_recv_error)));

        assert_eq!(ctx.handle_event("\"hello\""), Err("No engine set"));

        let (notify_sender, notify_receiver) = unbounded::<String>();
        let notify_fn = move |result: String| {
            notify_sender.send(result).unwrap();
        };
        ctx.set_engine(MockEventHandler, Box::new(notify_fn));

        assert_eq!(ctx.handle_event("\"hello\""), Ok("Event scheduled"));
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(notify_receiver.recv().unwrap(), "\"hello\"");

        assert_eq!(ctx.handle_event("\"sleep100\""), Ok("Event scheduled"));
        std::thread::sleep(std::time::Duration::from_millis(250));
        assert_eq!(notify_receiver.recv().unwrap(), "\"slept100\"");
    }
}
