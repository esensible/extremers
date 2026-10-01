use core::f64::consts::PI;
use extreme_traits::{Engine, Fix, Outcome, StaticFiles, Velocity};
use heapless::Deque;
use libm::{atan2, cos, fmod, sin};
use serde::{Serialize, Serializer, ser::SerializeStruct};

/// Length of the averaging window, in milliseconds.
const WINDOW_MS: u64 = 30_000;

#[derive(Default)]
pub struct TuneSpeed<const HISTORY_SIZE: usize> {
    // Public state variables
    pub speed: f64,
    pub speed_dev: f64,
    pub heading_dev: f64,

    // Internal state variables (not serialized)
    history: Deque<(Velocity, u64), HISTORY_SIZE>, // (velocity, timestamp)
    last_timestamp: Option<u64>,
}

impl<const HISTORY_SIZE: usize> TuneSpeed<HISTORY_SIZE> {
    fn push(&mut self, velocity: Velocity, timestamp: u64) {
        if self.history.is_full() {
            self.history.pop_front();
        }
        // cannot fail: there is room after the pop (unless HISTORY_SIZE is 0)
        self.history.push_back((velocity, timestamp)).ok();
    }
}

impl<const HISTORY_SIZE: usize> Engine for TuneSpeed<HISTORY_SIZE> {
    const NAME: &'static str = "TuneSpeed";
    const STATIC_FILES: StaticFiles = extreme_traits::static_files!();

    // we don't need events right now
    type Event<'a> = ();

    fn location_event(
        &mut self,
        timestamp: u64,
        _fix: Option<Fix>,
        velocity: Option<Velocity>,
    ) -> Outcome {
        let Some(current) = velocity else {
            return Outcome::NONE;
        };

        let Some(last_ts) = self.last_timestamp else {
            // First timestamp received
            self.push(current, timestamp);
            self.speed = current.speed;
            self.speed_dev = 0.0;
            self.heading_dev = 0.0;
            self.last_timestamp = Some(timestamp);
            return Outcome::CHANGED;
        };

        // Ignore repeated and out-of-order samples.
        if timestamp <= last_ts {
            return Outcome::NONE;
        }

        self.push(current, timestamp);

        // Time-weighted averages over the last 30 seconds: each sample's speed
        // and heading are weighted by the time until the next sample, clipped
        // to the window. The heading average is taken over the unit vectors;
        // atan2 does not need the sums normalised by the total time.
        let window_start = timestamp.saturating_sub(WINDOW_MS);
        let mut weighted_speed_sum = 0.0;
        let mut total_time = 0.0;
        let mut sum_sin = 0.0;
        let mut sum_cos = 0.0;

        let mut prev_ts = timestamp;
        for &(sample, ts) in self.history.iter().rev() {
            let in_window = ts >= window_start;
            let until = if in_window { ts } else { window_start };
            let dt = prev_ts.saturating_sub(until) as f64 / 1000.0; // seconds

            let heading_rad = sample.heading * PI / 180.0;
            weighted_speed_sum += sample.speed * dt;
            total_time += dt;
            sum_sin += sin(heading_rad) * dt;
            sum_cos += cos(heading_rad) * dt;

            if !in_window {
                break;
            }
            prev_ts = ts;
        }

        let mean_speed = if total_time > 0.0 {
            weighted_speed_sum / total_time
        } else {
            current.speed
        };

        self.speed = current.speed;
        self.speed_dev = current.speed - mean_speed;

        let avg_heading_deg = atan2(sum_sin, sum_cos) * 180.0 / PI;

        // Heading deviation, normalized to [-180, 180] degrees
        let heading_deviation = current.heading - avg_heading_deg;
        self.heading_dev = fmod(heading_deviation + 180.0, 360.0) - 180.0;

        self.last_timestamp = Some(timestamp);

        Outcome::CHANGED
    }

    fn external_event(&mut self, _timestamp: u64, _event: &()) -> Outcome {
        // No external events to handle
        Outcome::NONE
    }

    fn timer_event(&mut self, _timestamp: u64) -> Outcome {
        // No timer events needed
        Outcome::NONE
    }
}

impl<const HISTORY_SIZE: usize> Serialize for TuneSpeed<HISTORY_SIZE> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("TuneSpeed", 3)?;
        state.serialize_field("speed", &self.speed)?;
        state.serialize_field("speed_dev", &self.speed_dev)?;
        state.serialize_field("heading_dev", &self.heading_dev)?;
        state.end()
    }
}
