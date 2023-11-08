use ::serde::Deserialize;
use flatdiff_derive::FlatDiffSer;

use crate::*;
use crate as engine;

#[derive(FlatDiffSer, Copy, Clone, PartialEq, Default, Debug)]
pub struct ACore {
    pub f1: u32,
    pub f2: u32,
    pub loc: bool,
}

#[derive(Deserialize)]
pub enum SomeEvents {
    Event1 { value: u32 },
    Event2 { value: u32, timestamp: u64 },
}

#[derive(Deserialize)]
pub struct Event {
    pub event: SomeEvents,
}

callbacks! {ACore,
    pub ACoreCallbacks {
        Callback(u32),
    }
}

impl ACore {
    pub fn callback(&mut self, arg: &u32) {
        self.f2 = *arg;
    }
}

impl EngineCore for ACore {
    type Event = Event;
    type Callbacks = ACoreCallbacks;

    fn handle_event(
        &mut self,
        event: Self::Event,
        sleep: &mut dyn FnMut(u64, Self::Callbacks) -> Result<(), &'static str>,
    ) -> Result<bool, &'static str> {
        match event.event {
            SomeEvents::Event1 { value } => {
                self.f1 = value;
                Ok(true)
            }
            SomeEvents::Event2 { value, timestamp } => {
                sleep(timestamp, <u32>::new(ACore::callback, value))?;

                Ok(true)
            }
        }
    }

    fn update_location(
        &mut self,
        _timestamp: u64,
        _location: Option<(f64, f64)>,
        _speed: Option<(f64, f64)>,
    ) -> bool {
        self.loc = true;
        true
    }
}
