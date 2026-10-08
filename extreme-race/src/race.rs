use core::f64::consts::PI;

use postcard_schema::Schema;
use serde::{Deserialize, Serialize};

use crate::line::Line;
use crate::types::Location;
use extreme_traits::{Engine, Fix, Outcome, StaticFiles, Velocity};

#[derive(Copy, Clone, PartialEq, Default)]
pub struct Race {
    pub state: State,
    pub line: Line,
    pub location: Location,
    /// Latest speed and heading. The heading is only shown while
    /// `Racing`; see [`RaceView`].
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

/// A client event for the race engine.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Schema)]
pub enum RaceEvent {
    /// Set the starboard end of the line at the current position.
    LineStbd,
    /// Set the port end.
    LinePort,
    /// Bump the start sequence. Not in a sequence: start one with the gun
    /// `seconds` from the tap. In a sequence: `seconds` > 0 moves the start
    /// earlier (less time on the countdown), < 0 later, and 0 syncs:
    /// rounds the time remaining down to a whole minute.
    ///
    /// `ago` is how many milliseconds before the event arrived the tap
    /// happened: a confirm step takes a second or two, and a sync must be
    /// timed from the tap. The client measures it on its own clock, so it
    /// needs no clock sync.
    BumpSeq { seconds: i32, ago: u32 },
    /// Finish the race, or abort a start sequence.
    RaceFinish,
}

/// What clients see of a race.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Schema)]
pub struct RaceView {
    /// Speed over ground, knots.
    pub speed: f32,
    pub phase: Phase,
}

/// Where the race is, with what each phase shows. Times are epoch
/// milliseconds on the device's clock.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Schema)]
pub enum Phase {
    /// No start sequence running; the line can be set.
    Active { line: LineView },
    /// Counting down to the start at `start_time`.
    InSequence { start_time: u64, line: LineView },
    /// Racing since `start_time`; the line is no longer shown.
    Racing {
        start_time: u64,
        /// Course over ground, degrees true; 0 until the first GPS update
        /// after the start.
        heading: f32,
    },
}

/// The start line, as far as it has been set.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Schema)]
pub enum LineView {
    None,
    /// Only the starboard end is set.
    Stbd,
    /// Only the port end is set.
    Port,
    /// Both ends: on the current course the boat crosses the line `cross`
    /// percent of the way along it (0 = port end, 100 = starboard end) at
    /// `time` (which is in the past if the boat is sailing away from it).
    Both { cross: u8, time: u64 },
}

impl Race {
    fn line_view(&self) -> LineView {
        match self.line {
            Line::None => LineView::None,
            Line::Stbd { .. } => LineView::Stbd,
            Line::Port { .. } => LineView::Port,
            Line::Both {
                line_cross,
                line_timestamp,
                ..
            } => LineView::Both {
                cross: line_cross,
                time: line_timestamp,
            },
        }
    }
}

impl Engine for Race {
    const NAME: &'static str = "Race";
    const STATIC_FILES: StaticFiles = extreme_traits::static_files!();

    type View = RaceView;
    type Event = RaceEvent;

    fn view(&self) -> RaceView {
        let phase = match self.state {
            State::Active => Phase::Active {
                line: self.line_view(),
            },
            State::InSequence { start_time } => Phase::InSequence {
                start_time,
                line: self.line_view(),
            },
            State::Racing { start_time } => Phase::Racing {
                start_time,
                heading: self.velocity.heading as f32,
            },
        };
        RaceView {
            speed: self.velocity.speed as f32,
            phase,
        }
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

    fn external_event(&mut self, timestamp: u64, event: &RaceEvent) -> Outcome {
        match *event {
            RaceEvent::LineStbd => Outcome::changed(self.line.set_stbd(self.location)),
            RaceEvent::LinePort => Outcome::changed(self.line.set_port(self.location)),
            RaceEvent::BumpSeq { seconds, ago } => {
                // when the tap happened
                let timestamp = timestamp.saturating_sub(ago as u64);
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
            RaceEvent::RaceFinish => {
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
