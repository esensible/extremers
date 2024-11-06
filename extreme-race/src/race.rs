use ::serde::Deserialize;
use core::f64::consts::PI;
use serde::Serialize;

use crate::line::Line;
use crate::types::Location;
use extreme_traits::Engine;


#[derive(Serialize, Copy, Clone, PartialEq, Default)]
pub struct Race {
    pub state: State,
    pub line: Line,
    #[serde(skip)]
    pub location: Location,
}

#[derive(Serialize, Copy, Clone, PartialEq)]
pub enum State {
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

impl Engine for Race {
    type Event = Event;


    fn timer_event(
        &mut self, 
        timestamp: u64
    ) -> (Option<()>, Option<u64>) 
    {
        let start_time = if let State::InSequence { start_time, .. } = self.state {
            start_time
        } else {
            // bad things happened, we were in an unexpected state. Roll with it as best we can.
            timestamp 
        };

        self.state = State::Racing {
            start_time,
            speed: 0.0,
            heading: 0.0,
        };

        // state is updated, no new timer
        (Some(()), None)
    }



    fn external_event(
        &mut self, 
        _timestamp: u64, 
        event: Self::Event
    ) -> (Option<()>, Option<u64>)
    {

        match event.event {
            EventType::LineStbd => {
                return (
                    self.line.set_stbd(self.location),
                    None
                );
            }
            EventType::LinePort => {
                return (
                    self.line.set_port(self.location), 
                    None
                );
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

                        // updated, and new timer
                        return (Some(()), Some(*start_time));
                    }

                    _ => {
                        // changing states - set absolute start time
                        let abs_seconds = seconds.unsigned_abs() as u64;
                        let new_start = if seconds.is_negative() {
                            timestamp - abs_seconds * 1000
                        } else {
                            timestamp + abs_seconds * 1000
                        };

                        let old_speed = match &self.state {
                            State::Active { speed } => *speed,
                            State::InSequence { speed, .. } => *speed,
                            State::Racing { speed, .. } => *speed,
                        };

                        // update now that start is scheduled
                        self.state = State::InSequence {
                            start_time: new_start,
                            speed: old_speed,
                        };

                        return (Some(()), Some(new_start));
                    }
                }
            }
            EventType::RaceFinish => {
                let old_speed = match &self.state {
                    State::Active { speed, } => *speed,
                    State::InSequence { speed, .. } => *speed,
                    State::Racing { speed, .. } => *speed,
                };

                if !matches!(self.state, State::Active { .. }) {
                    self.state = State::Active { speed: old_speed };
                    return (Some(()), None);
                } else {
                    return (None, None);
                }
            }
        }
    }

    fn location_event(
        &mut self, 
        timestamp: u64, 
        location: Option<(f64, f64)>, 
        speed: Option<(f64, f64)>
    ) -> (Option<()>, Option<u64>)
    {
        let mut result = None;

        if let Some((new_speed, new_heading)) = speed {
            match &mut self.state {
                State::Active { speed } => {
                    *speed = new_speed;
                }
                State::InSequence { speed, .. } => {
                    *speed = new_speed;
                }
                State::Racing { speed, heading, .. } => {
                    *speed = new_speed;
                    *heading = new_heading;
                }
            }
            result = Some(());
        };

        if let Some((lat, lon)) = location {
            let lat = lat * PI / 180.0;
            let lon = lon * PI / 180.0;
            self.location = Location { lat, lon };

            if let Some((speed, heading)) = speed {
                let heading = heading * PI / 180.0;
                self.line
                    .update_location(timestamp, (lat, lon), heading, speed);            
            }

            return (result, None)
        }
        return (None, None)
    }
}
