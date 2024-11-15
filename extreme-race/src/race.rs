use ::serde::Deserialize;
use core::f64::consts::PI;
use serde::ser::SerializeStruct;
use serde::Serialize;

use crate::line::Line;
use crate::types::Location;
use extreme_traits::Engine;

include!(concat!(env!("OUT_DIR"), "/static_files.rs"));

#[derive(Copy, Clone, PartialEq, Default)]
// Serialize is implemented below because line serialization depends on Race state
pub struct Race {
    pub state: State,
    pub line: Line,
    pub location: Location,
}

#[derive(Serialize, Copy, Clone, PartialEq)]
#[serde(tag = "state")]
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

// Note: we use a struct to deserialize because serde
// can't use tag= (to flatten) with no_std
#[derive(Deserialize)]
pub struct Event {
    pub event: EventType,
}

impl Engine for Race {
    type Event<'a> = Event;
    fn get_static(&self, path: &'_ str) -> Option<&'static [u8]> {
        for &(k, v) in STATIC_FILES.iter() {
            if k == path {
                return Some(v);
            }
        }
        return None;
    }

    fn timer_event(&mut self, timestamp: u64) -> (Option<()>, Option<u64>) {
        let (start_time, speed) = if let State::InSequence {
            start_time, speed, ..
        } = self.state
        {
            (start_time, speed)
        } else {
            // bad things happened, we were in an unexpected state. Roll with it as best we can.
            (timestamp, 0.0)
        };

        self.state = State::Racing {
            start_time,
            speed: speed,
            heading: 0.0,
        };

        // state is updated, no new timer
        (Some(()), None)
    }

    fn external_event(
        &mut self,
        _timestamp: u64,
        event: &Self::Event<'_>,
    ) -> (Option<()>, Option<u64>) {
        match event.event {
            EventType::LineStbd => {
                return (self.line.set_stbd(self.location), None);
            }
            EventType::LinePort => {
                return (self.line.set_port(self.location), None);
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
                    State::Active { speed } => *speed,
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
        speed: Option<(f64, f64)>,
    ) -> (Option<()>, Option<u64>) {
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

            if !matches!(self.state, State::Racing { .. }) {
                if let Some((speed, heading)) = speed {
                    let heading = heading * PI / 180.0;
                    if Some(())
                        == self
                            .line
                            .update_location(timestamp, (lat, lon), heading, speed)
                    {
                        return (Some(()), None);
                    }
                }
            }
        }
        return (result, None);
    }
}

impl Serialize for Race {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut s = serializer.serialize_struct("Race", 7)?;

        match &self.state {
            State::Active { speed } => {
                s.serialize_field("state", "Active")?;
                s.serialize_field("speed", speed)?;
            }
            State::InSequence { start_time, speed } => {
                s.serialize_field("state", "InSequence")?;
                s.serialize_field("start_time", start_time)?;
                s.serialize_field("speed", speed)?;
            }
            State::Racing {
                start_time,
                speed,
                heading,
            } => {
                s.serialize_field("state", "Racing")?;
                s.serialize_field("start_time", start_time)?;
                s.serialize_field("speed", speed)?;
                s.serialize_field("heading", heading)?;
            }
        }

        // Conditionally serialize the `line` field based on `state`
        if !matches!(self.state, State::Racing { .. }) {
            // s.serialize_field("line", &self.line)?;
            match &self.line {
                Line::None => {
                    s.serialize_field("line", "None")?;
                }
                Line::Stbd { .. } => {
                    s.serialize_field("line", "Stbd")?;
                }
                Line::Port { .. } => {
                    s.serialize_field("line", "Port")?;
                }
                Line::Both {
                    line_cross,
                    line_timestamp,
                    ..
                } => {
                    s.serialize_field("line", "Both")?;
                    s.serialize_field("line_cross", line_cross)?;
                    s.serialize_field("line_timestamp", line_timestamp)?;
                }
            }
        }

        s.end()
    }
}
