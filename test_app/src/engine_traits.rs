extern crate alloc;
use alloc::boxed::Box;
use serde::{Deserialize, Serialize};

pub type EventSleepCB<T> = dyn Fn(&mut T) -> Option<<T as EventHandler>::Update> + Send;
pub type EventSleepFn<'a, T> = dyn Fn(u32, Box<EventSleepCB<T>>) + 'a;

pub trait EventHandler where 
    Self::Event: for<'de> Deserialize<'de>,
    Self::Update: Serialize {
    type Event;
    type Update;

    fn handle_event<'a>(&mut self, event: Self::Event, sleep: &EventSleepFn<'a, Self>) -> Result<Option<Self::Update>, &'static str>;
}
