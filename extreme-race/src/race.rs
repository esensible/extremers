use core::f64::consts::PI;

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};

use crate::line::Line;
use crate::types::Location;
use extreme_traits::{Engine, Fix, Outcome, StaticFiles, Velocity};

#[derive(Copy, Clone, PartialEq, Default)]
// Serialize is implemented below because line serialization depends on Race state
pub struct Race {
    pub state: State,
    pub line: Line,
    pub location: Location,
    /// Latest speed and heading. Only `speed` is reported outside of
    /// `Racing`; see `impl Serialize for Race`.
    pub velocity: Velocity,
}

#[derive(Copy, Clone, PartialEq, Default, Debug)]
pub enum State {
    #[default]
    Active,
    InSequence {
        start_time: u64,
    },
    Racing {
        start_time: u64,
    },
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

/// Compact (BLE) protocol, see BLE.md "Race".
mod compact {
    /// State codes.
    pub const ACTIVE: u8 = 0;
    pub const IN_SEQUENCE: u8 = 1;
    pub const RACING: u8 = 2;
    /// Line codes.
    pub const LINE_NONE: u8 = 0;
    pub const LINE_STBD: u8 = 1;
    pub const LINE_PORT: u8 = 2;
    pub const LINE_BOTH: u8 = 3;
    /// Event opcodes.
    pub const OP_LINE_STBD: u8 = 0x10;
    pub const OP_LINE_PORT: u8 = 0x11;
    pub const OP_BUMP_SEQ: u8 = 0x12;
    pub const OP_RACE_FINISH: u8 = 0x13;
    /// Encoded state length.
    pub const STATE_LEN: usize = 15;
}

/// Milliseconds from `now` to `at`, clamped to an `i32`.
fn millis_until(now: u64, at: u64) -> i32 {
    (at as i64 - now as i64).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

impl Engine for Race {
    const NAME: &'static str = "Race";
    const STATIC_FILES: StaticFiles = extreme_traits::static_files!();

    type Event<'a> = Event;

    /// `[state][line][line_cross][start_in i32][line_in i32][speed u16][heading u16]`,
    /// little-endian; times in ms relative to `now`, speed in knots x 100,
    /// heading in degrees x 10.
    fn compact_state(&self, now: u64, out: &mut [u8]) -> usize {
        if out.len() < compact::STATE_LEN {
            return 0;
        }
        let (state, start_time) = match self.state {
            State::Active => (compact::ACTIVE, now),
            State::InSequence { start_time } => (compact::IN_SEQUENCE, start_time),
            State::Racing { start_time } => (compact::RACING, start_time),
        };
        let (line, line_cross, line_timestamp) = match self.line {
            Line::None => (compact::LINE_NONE, 0, now),
            Line::Stbd { .. } => (compact::LINE_STBD, 0, now),
            Line::Port { .. } => (compact::LINE_PORT, 0, now),
            Line::Both {
                line_cross,
                line_timestamp,
                ..
            } => (compact::LINE_BOTH, line_cross, line_timestamp),
        };
        let speed = (self.velocity.speed * 100.0).clamp(0.0, u16::MAX as f64) as u16;
        let heading = (self.velocity.heading * 10.0).clamp(0.0, u16::MAX as f64) as u16;

        out[0] = state;
        out[1] = line;
        out[2] = line_cross;
        out[3..7].copy_from_slice(&millis_until(now, start_time).to_le_bytes());
        out[7..11].copy_from_slice(&millis_until(now, line_timestamp).to_le_bytes());
        out[11..13].copy_from_slice(&speed.to_le_bytes());
        out[13..15].copy_from_slice(&heading.to_le_bytes());
        compact::STATE_LEN
    }

    /// `[0x10]` line stbd, `[0x11]` line port, `[0x13]` finish,
    /// `[0x12][seconds i16][ago u16]` bump the sequence: `seconds` as in
    /// `BumpSeq`, `ago` how many ms before `timestamp` the tap happened.
    fn compact_event(&mut self, timestamp: u64, event: &[u8]) -> Result<Outcome, ()> {
        let event = match *event {
            [compact::OP_LINE_STBD] => EventType::LineStbd,
            [compact::OP_LINE_PORT] => EventType::LinePort,
            [compact::OP_RACE_FINISH] => EventType::RaceFinish,
            [compact::OP_BUMP_SEQ, s0, s1, a0, a1] => EventType::BumpSeq {
                timestamp: timestamp.saturating_sub(u16::from_le_bytes([a0, a1]) as u64),
                seconds: i16::from_le_bytes([s0, s1]) as i32,
            },
            _ => return Err(()),
        };
        Ok(Engine::external_event(self, timestamp, &Event { event }))
    }

    fn timer_event(&mut self, _timestamp: u64) -> Outcome {
        // The only timer this engine sets is the start gun. One arriving in
        // any other state is stale (e.g. raced with a RaceFinish) and is
        // ignored rather than starting a race nobody is counting down to.
        let State::InSequence { start_time } = self.state else {
            return Outcome::NONE;
        };

        self.state = State::Racing { start_time };
        // Heading is reported as 0 until the first GPS update after the start.
        self.velocity.heading = 0.0;

        Outcome::CHANGED
    }

    fn external_event(&mut self, _timestamp: u64, event: &Event) -> Outcome {
        match event.event {
            EventType::LineStbd => Outcome::changed(self.line.set_stbd(self.location)),
            EventType::LinePort => Outcome::changed(self.line.set_port(self.location)),
            EventType::BumpSeq { timestamp, seconds } => {
                let offset = seconds.unsigned_abs() as u64 * 1000;
                let start_time = match &mut self.state {
                    State::InSequence { start_time } => {
                        if seconds == 0 {
                            // Round the time remaining down to a whole minute.
                            // If the start has already passed (the timer has
                            // not been delivered yet) there is nothing to
                            // round, so the start time is left alone and the
                            // timer below fires straight away.
                            *start_time -= start_time.saturating_sub(timestamp) % 60_000;
                        } else if seconds.is_negative() {
                            *start_time = start_time.saturating_add(offset);
                        } else {
                            // Bumping past the epoch clamps to 0: the start is
                            // in the past and the timer fires straight away.
                            *start_time = start_time.saturating_sub(offset);
                        }
                        *start_time
                    }

                    _ => {
                        // changing states - set absolute start time, clamped
                        // as above
                        let start_time = if seconds.is_negative() {
                            timestamp.saturating_sub(offset)
                        } else {
                            timestamp.saturating_add(offset)
                        };
                        self.state = State::InSequence { start_time };
                        start_time
                    }
                };

                Outcome::CHANGED.with_timer(start_time)
            }
            EventType::RaceFinish => {
                if matches!(self.state, State::Active) {
                    Outcome::NONE
                } else {
                    // also aborts a start sequence, so drop its timer
                    self.state = State::Active;
                    Outcome::CHANGED.cancel_timer()
                }
            }
        }
    }

    fn location_event(
        &mut self,
        timestamp: u64,
        fix: Option<Fix>,
        velocity: Option<Velocity>,
    ) -> Outcome {
        if let Some(velocity) = velocity {
            self.velocity = velocity;
        }

        if let Some(fix) = fix {
            self.location = fix.into();

            if !matches!(self.state, State::Racing { .. }) {
                if let Some(velocity) = velocity {
                    self.line.update_location(
                        timestamp,
                        self.location,
                        velocity.heading * PI / 180.0,
                        velocity.speed,
                    );
                }
            }
        }

        // A line update only happens alongside a velocity, which is itself a change.
        Outcome::changed(velocity.is_some())
    }
}

impl Serialize for Race {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut s = serializer.serialize_struct("Race", 7)?;

        match &self.state {
            State::Active => {
                s.serialize_field("state", "Active")?;
                s.serialize_field("speed", &self.velocity.speed)?;
            }
            State::InSequence { start_time } => {
                s.serialize_field("state", "InSequence")?;
                s.serialize_field("start_time", start_time)?;
                s.serialize_field("speed", &self.velocity.speed)?;
            }
            State::Racing { start_time } => {
                s.serialize_field("state", "Racing")?;
                s.serialize_field("start_time", start_time)?;
                s.serialize_field("speed", &self.velocity.speed)?;
                s.serialize_field("heading", &self.velocity.heading)?;
            }
        }

        // The line is only reported before the start
        if !matches!(self.state, State::Racing { .. }) {
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
