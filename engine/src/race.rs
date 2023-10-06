use ::serde::{Deserialize, Serialize, Serializer};
use ::serde::ser::SerializeStruct;

use flatdiff::{Atomic, FlatDiffSer};
use crate::callbacks;
use crate::core::EngineCore;

#[derive(FlatDiffSer, Copy, Clone, PartialEq, Default)]
pub struct Race {
    location: Location,
    line: Line,
    state: State,
}

#[derive(FlatDiffSer, Default, Copy, Clone, PartialEq)]
enum State {
    #[default]
    Setup,
    Idle,
    InSequence {
        start: f64,
    },
    Racing {
        start: f64,
    },
}

#[derive(Serialize, Copy, Clone, PartialEq, Default)]
pub struct Location {
    lat: f64,
    lon: f64,
}
impl Atomic for Location {}

#[derive(FlatDiffSer, Copy, Clone, PartialEq, Default)]
pub enum Line {
    #[default]
    None,

    // #[delta(skip_fields)]
    Stbd {
        location: Location,
    },

    // #[delta(skip_fields)]
    Port {
        location: Location,
    },

    Both {
        time: f64,
        point: u8,

        // #[delta(skip)]
        stbd: Location,

        // #[delta(skip)]
        port: Location,
    },
}

#[derive(Deserialize)]
pub enum EventType {
    SetupPushOff,

    LineStbd,
    LinePort,

    IdleSeq { seconds: f64 },
    SeqBump { seconds: f64 },

    RaceFinish,
}

#[derive(Deserialize)]
pub struct Event {
    pub timestamp: f64,
    pub event: EventType,
}

callbacks! {Race,
    pub RaceCallbacks {
        Start(()),
    }
}

impl EngineCore for Race {
    type Event = Event;
    type Callbacks = RaceCallbacks;

    fn handle_event(
        &mut self,
        event: Self::Event,
        sleep: &dyn FnMut(u32, RaceCallbacks),
    ) -> Result<(), &'static str> {
        match event.event {
            EventType::SetupPushOff => {
                self.state = State::Idle;
                Ok(())
            }
            EventType::LineStbd => {
                self.line = Line::Stbd {
                    location: Location::default(),
                };
                Ok(())
            }
            EventType::LinePort => {
                self.line = Line::Port {
                    location: Location::default(),
                };
                Ok(())
            }
            EventType::IdleSeq { seconds } => {
                self.state = State::InSequence { start: seconds };
                Ok(())
            }
            EventType::SeqBump { seconds } => {
                self.state = State::Racing { start: seconds };
                Ok(())
            }
            EventType::RaceFinish => {
                self.state = State::Idle;
                Ok(())
            }
        }
    }
}


