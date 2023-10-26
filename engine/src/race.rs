use ::serde::ser::SerializeStruct;
use ::serde::Deserialize;
use ::serde::Serializer;

use crate::callbacks;
use crate::core::{EngineCore, FlatDiffSer};
use crate::line::Line;
use crate::types::Location;

#[derive(FlatDiffSer, Copy, Clone, PartialEq, Default)]
pub struct Race {
    #[delta(skip)]
    location: Location,
    line: Line,
    state: State,
}

#[derive(FlatDiffSer, Copy, Clone, PartialEq, Default)]
enum State {
    #[default]
    Idle,
    Active {
        speed: f32,
    },
    InSequence {
        start_time: u64,
        speed: f32,
    },
    Racing {
        start_time: u64,
        speed: f32,
        heading: f32,
    },
}

#[derive(Deserialize)]
pub enum EventType {
    Activate,

    LineStbd,
    LinePort,

    BumpSeq { timestamp: u64, seconds: i32 },

    RaceFinish,
}

#[derive(Deserialize)]
pub struct Event {
    pub event: EventType,
}

callbacks! {Race,
    pub RaceCallbacks {
        Start(()),
    }
}

impl Race {
    fn start(&mut self, _: &()) {
        let start_time = if let State::InSequence { start_time, .. } = self.state {
            start_time
        } else {
            0 // bad things happened, we were in an unexpected state
        };

        self.state = State::Racing {
            start_time,
            speed: 0.0,
            heading: 0.0,
        };
    }
}

impl EngineCore for Race {
    type Event = Event;
    type Callbacks = RaceCallbacks;

    fn handle_event(
        &mut self,
        event: Self::Event,
        sleep: &mut dyn FnMut(u64, RaceCallbacks) -> Result<(), &'static str>,
    ) -> Result<bool, &'static str> {
        match event.event {
            EventType::Activate => {
                self.state = State::Active { speed: 0.0 };
                Ok(true)
            }
            EventType::LineStbd => {
                self.line.set_stbd(self.location);
                Ok(
                    matches!(self.line, Line::Stbd { .. })
                        || matches!(self.line, Line::Both { .. }),
                )
            }
            EventType::LinePort => {
                self.line.set_port(self.location);
                Ok(
                    matches!(self.line, Line::Port { .. })
                        || matches!(self.line, Line::Both { .. }),
                )
            }
            EventType::BumpSeq { timestamp, seconds } => {
                match &mut self.state {
                    State::InSequence { start_time, .. } => {
                        if seconds == 0 {
                            // round down to nearest minute
                            *start_time -= (*start_time - timestamp) % 60000;
                        } else {
                            // apply offset
                            let abs_seconds = seconds.unsigned_abs() as u64;
                            if seconds.is_negative() {
                                *start_time += abs_seconds * 1000;
                            } else {
                                *start_time -= abs_seconds * 1000;
                            }
                        }
                        sleep(*start_time, <()>::new(Race::start, ()))?;
                    }

                    _ => {
                        // changing states - set absolute start time
                        let abs_seconds = seconds.unsigned_abs() as u64;
                        let new_start = if seconds.is_negative() {
                            timestamp - abs_seconds * 1000
                        } else {
                            timestamp + abs_seconds * 1000
                        };
                        sleep(new_start, <()>::new(Race::start, ()))?;

                        // update now that start is scheduled
                        self.state = State::InSequence {
                            start_time: new_start,
                            speed: 0.0,
                        };
                    }
                };
                Ok(true)
            }
            EventType::RaceFinish => {
                if !matches!(self.state, State::Idle) {
                    self.state = State::Idle {};
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    fn update_location(
        &mut self,
        new_location: Option<(f32, f32)>,
        new_speed: Option<(f32, f32)>,
    ) -> bool {
        let mut updated = false;

        if let Some((new_lat, new_lon)) = new_location {
            self.location = Location {
                lat: new_lat,
                lon: new_lon,
            };
            updated = true;
        }

        if let Some((new_speed, new_heading)) = new_speed {
            match &mut self.state {
                State::Active { speed } => {
                    *speed = new_speed;
                    updated = true;
                }
                State::InSequence { speed, .. } => {
                    *speed = new_speed;
                    // TODO: Update line stuff
                    updated = true;
                }
                State::Racing { speed, heading, .. } => {
                    *speed = new_speed;
                    *heading = new_heading;
                    updated = true;
                }
                _ => {}
            }
        }

        updated
    }
}

#[cfg(test)]
#[path = "./race_tests.rs"]
mod race_tests;
