extern crate alloc;
use alloc::boxed::Box;
use serde_derive::{Deserialize, Serialize};
use std::result::Result;

use crate::engine_traits::{EventHandler, EventSleepFn};

pub struct Race {
    data: u32,
}

impl Default for Race {
    fn default () -> Self {
        Race { data: 0 }
    }
}

#[derive(Serialize)]
pub struct State {
    value: u32,
}

#[derive(Deserialize)]
pub enum Event {
    Sleep { value: u32 },
    Increment,
}

impl EventHandler for Race {
    type Event = Event;
    type Update = State;

    fn handle_event(&mut self, event: Self::Event, sleep: &EventSleepFn<Self>) -> Result<Option<Self::Update>, &'static str> {
        match event {
            Event::Sleep { value } => {
                sleep(2000, Box::new(move |sm: &mut Self| {
                    sm.data += value;
                    let result = State{value: sm.data };
                    Some(result)
                }));
                Ok(None)
            }
            Event::Increment => {
                // Handle the increment event
                self.data += 1;
                let result = State{value: self.data };
                Ok(Some(result))
            }
        }
    }
}
