extern crate alloc;
use alloc::string::String;
use alloc::boxed::Box;
use serde_json;
use serde::{Deserialize, Serialize};

pub type SleepCB<T> = dyn Fn(&mut T) -> Option<String> + Send;
pub type SleepFn<T> = dyn Fn(u32, Box<SleepCB<T>>) + Send;

pub trait Engine {
    fn handle_event(&mut self, event: String, sleep: &SleepFn<Self>) -> Result<Option<String>, &'static str>;
}

pub type EventSleepCB<T> = dyn Fn(&mut T) -> Option<<T as EventHandler>::Update> + Send;
pub type EventSleepFn<'a, T> = dyn Fn(u32, Box<EventSleepCB<T>>) + 'a;

pub trait EventHandler where 
    Self::Event: for<'de> Deserialize<'de>,
    Self::Update: Serialize {
    type Event;
    type Update;

    fn handle_event<'a>(&mut self, event: Self::Event, sleep: &EventSleepFn<'a, Self>) -> Result<Option<Self::Update>, &'static str>;
}

pub struct EngineWrapper<T: EventHandler>(T);

impl<T: EventHandler + 'static> Engine for EngineWrapper<T> {
    fn handle_event(&mut self, event: String, sleep: &SleepFn<Self>) -> Result<Option<String>, &'static str> {
        let event = serde_json::from_str(&event).map_err(|_| "Failed to deserialize event")?;

        let adapted_sleep = {
            let sleep = sleep; // Capture the sleep reference here.
            move |duration: u32, cb: Box<EventSleepCB<T>>| {
                // This is the adapted callback
                let adapted_cb = Box::new(move |engine_wrapper: &mut Self| {
                    let result = cb(&mut engine_wrapper.0);
                    match result {
                        Some(update) => Some(serde_json::to_string(&update).unwrap()),
                        None => None,
                    }
                });
    
                sleep(duration, adapted_cb);
            }
        };

        let update = self.0.handle_event(event, &adapted_sleep)?;
        if let Some(update) = update {
            let update = serde_json::to_string(&update).map_err(|_| "Failed to serialize update")?;
            Ok(Some(update))
        } else {
            Ok(None)
        }
    }
}

impl<T: EventHandler + Default> Default for EngineWrapper<T> {
    fn default() -> Self {
        EngineWrapper(T::default())
    }
}