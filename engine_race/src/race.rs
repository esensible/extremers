use ::serde::Deserialize;
use core::f64::consts::PI;

use crate::line::Line;
use crate::types::Location;
use engine::engine_callbacks;
use engine::{EngineCore, FlatDiffSer};

#[derive(FlatDiffSer, Copy, Clone, PartialEq, Default)]
pub struct Race {
    #[delta(skip)]
    location: Location,
    line: Line,
    state: State,
}

#[derive(FlatDiffSer, Copy, Clone, PartialEq)]
enum State {
    Active {
        speed: f64,
    },
    InSequence {
        start_time: u64,
        speed: f64,
    },
    Racing {
        start_time: u64,
        speed: f64,
        heading: f64,
    },
}

impl Default for State {
    fn default() -> Self {
        State::Active { speed: 0.0 }
    }
}

#[derive(Deserialize)]
pub enum EventType {
    LineStbd,
    LinePort,

    BumpSeq { timestamp: u64, seconds: i32 },

    RaceFinish,
}

#[derive(Deserialize)]
pub struct Event {
    pub event: EventType,
}

engine_callbacks! {Race,
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
                if !matches!(self.state, State::Active { .. }) {
                    self.state = State::Active { speed: 0.0 };
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    fn update_location(
        &mut self,
        timestamp: u64,
        new_location: Option<(f64, f64)>,
        new_speed: Option<(f64, f64)>,
    ) -> bool {
        let mut updated = false;

        let new_location = if let Some((lat, lon)) = new_location {
            Some((lat * PI / 180.0, lon * PI / 180.0))
        } else {
            None
        };
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
            }
        }

        if let Some(location) = new_location {
            if let Some((speed, heading)) = new_speed {
                let heading = heading * PI / 180.0;
                self.line
                    .update_location(timestamp, location, heading, speed);
            }
        }
        updated
    }
}

#[cfg(test)]
#[path = "./race_tests.rs"]
mod race_tests;
