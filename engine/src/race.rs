use ::serde::ser::SerializeStruct;
use ::serde::Deserialize;
use ::serde::Serializer;

use crate::callbacks;
use crate::core::{EngineCore, FlatDiffSer};
use crate::line::Line;
use crate::types::Location;

#[derive(FlatDiffSer, Copy, Clone, PartialEq, Default)]
pub struct Race {
    location: Location,
    line: Line,
    state: State,
}

#[derive(FlatDiffSer, Copy, Clone, PartialEq, Default)]
enum State {
    #[default]
    Idle,
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

#[derive(Deserialize)]
pub enum EventType {
    Activate,

    LineStbd,
    LinePort,

    BumpSeq {
        timestamp: u64,
        seconds: u64,
    },

    SetLocation {
        location: Location,
        speed: f64,
        heading: f64,
    },

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

impl EngineCore for Race {
    type Event = Event;
    type Callbacks = RaceCallbacks;

    fn handle_event(
        &mut self,
        event: Self::Event,
        sleep: &dyn FnMut(usize, RaceCallbacks),
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
                let new_start = match &mut self.state {
                    State::InSequence { start_time, .. } => {
                        if seconds == 0 {
                            // round down to nearest minute
                            *start_time -= (*start_time - timestamp) % 60000;
                        } else {
                            // apply offset
                            *start_time -= seconds * 1000;
                        }
                        *start_time
                    }

                    _ => {
                        // changing states - set absolute start time
                        let new_start = timestamp + seconds * 1000;
                        self.state = State::InSequence {
                            start_time: new_start,
                            speed: 0.0,
                        };
                        new_start
                    }
                };
                // TODO
                // const delta = state.start_time - now;
                // start_timeout = setTimeout(raceStart, delta);
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
            EventType::SetLocation {
                location,
                speed,
                heading,
            } => {
                self.location = location;
                match self.state {
                    State::Active {
                        speed: mut current_speed,
                    } => {
                        current_speed = speed;
                        Ok(true)
                    }
                    State::InSequence {
                        speed: mut current_speed,
                        ..
                    } => {
                        current_speed = speed;
                        // TODO: Update line stuff
                        Ok(true)
                    }
                    State::Racing {
                        speed: mut current_speed,
                        heading: mut current_heading,
                        ..
                    } => {
                        current_speed = speed;
                        current_heading = heading;
                        Ok(true)
                    }

                    _ => Ok(false),
                }
            }
        }
    }
}
