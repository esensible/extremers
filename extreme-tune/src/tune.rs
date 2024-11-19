use core::f64::consts::PI;
use extreme_traits::Engine;
use heapless::Deque;
use libm::{atan2, cos, fmod, sin};
use serde::{ser::SerializeStruct, Serialize, Serializer};

include!(concat!(env!("OUT_DIR"), "/static_files.rs"));

#[derive(Default)]
pub struct TuneSpeed<const HISTORY_SIZE: usize> {
    // Public state variables
    pub speed: f64,
    pub speed_dev: f64,
    pub heading_dev: f64,

    // Internal state variables (not serialized)
    speed_history: Deque<(f64, u64), HISTORY_SIZE>, // (speed, timestamp)
    heading_history: Deque<(f64, u64), HISTORY_SIZE>, // (heading, timestamp)
    last_timestamp: Option<u64>,
}

impl<const HISTORY_SIZE: usize> Engine for TuneSpeed<HISTORY_SIZE> {
    // we don't need events right now
    type Event<'a> = ();

    fn get_static(&self, path: &'_ str) -> Option<&'static [u8]> {
        for &(k, v) in STATIC_FILES.iter() {
            if k == path {
                return Some(v);
            }
        }
        return None;
    }

    fn location_event(
        &mut self,
        timestamp: u64,
        _location: Option<(f64, f64)>,
        speed_heading: Option<(f64, f64)>,
    ) -> (Option<()>, Option<u64>) {
        if let Some((current_speed, current_heading)) = speed_heading {
            // Update speed history
            if let Some(last_ts) = self.last_timestamp {
                let delta_time = timestamp - last_ts;
                if delta_time > 0 {
                    if self.speed_history.is_full() {
                        self.speed_history.pop_front();
                    }
                    self.speed_history
                        .push_back((current_speed, timestamp))
                        .ok();

                    if self.heading_history.is_full() {
                        self.heading_history.pop_front();
                    }
                    self.heading_history
                        .push_back((current_heading, timestamp))
                        .ok();

                    // Calculate weighted average speed over the last 30 seconds
                    let mut weighted_speed_sum = 0.0;
                    let mut total_time = 0.0;
                    let window_start = timestamp.saturating_sub(30_000); // 30 seconds in milliseconds

                    let mut prev_ts = timestamp;
                    for &(speed, ts) in self.speed_history.iter().rev() {
                        let dt = prev_ts.saturating_sub(ts) as f64 / 1000.0; // delta time in seconds
                        if ts >= window_start {
                            weighted_speed_sum += speed * dt;
                            total_time += dt;
                            prev_ts = ts;
                        } else {
                            let dt = prev_ts.saturating_sub(window_start) as f64 / 1000.0;
                            weighted_speed_sum += speed * dt;
                            total_time += dt;
                            break;
                        }
                    }
                    let mean_speed = if total_time > 0.0 {
                        weighted_speed_sum / total_time
                    } else {
                        current_speed
                    };

                    self.speed = current_speed;

                    // Calculate speed deviation
                    self.speed_dev = current_speed - mean_speed;

                    // Calculate weighted average heading over the last 30 seconds
                    let mut sum_sin = 0.0;
                    let mut sum_cos = 0.0;
                    prev_ts = timestamp;
                    // total_time = 0.0;
                    for &(heading, ts) in self.heading_history.iter().rev() {
                        let dt = prev_ts.saturating_sub(ts) as f64 / 1000.0; // delta time in seconds
                        let heading_rad = heading * PI / 180.0;
                        if ts >= window_start {
                            sum_sin += sin(heading_rad) * dt;
                            sum_cos += cos(heading_rad) * dt;
                            // total_time += dt;
                            prev_ts = ts;
                        } else {
                            let dt = prev_ts.saturating_sub(window_start) as f64 / 1000.0;
                            sum_sin += sin(heading_rad) * dt;
                            sum_cos += cos(heading_rad) * dt;
                            // total_time += dt;
                            break;
                        }
                    }
                    // No need to divide by total_time for heading calculation

                    // The atan2 function automatically handles the weighting through the accumulated sums
                    let avg_heading_rad = atan2(sum_sin, sum_cos);
                    let avg_heading_deg = avg_heading_rad * 180.0 / PI;

                    // Calculate heading deviation and normalize to [-180, 180] degrees
                    let mut heading_deviation = current_heading - avg_heading_deg;
                    heading_deviation = fmod(heading_deviation + 180.0, 360.0) - 180.0;
                    self.heading_dev = heading_deviation;

                    self.last_timestamp = Some(timestamp);

                    return (Some(()), None);
                }
            } else {
                // First timestamp received
                self.speed_history
                    .push_back((current_speed, timestamp))
                    .ok();
                self.heading_history
                    .push_back((current_heading, timestamp))
                    .ok();
                self.speed = current_speed;
                self.speed_dev = 0.0;
                self.heading_dev = 0.0;
                self.last_timestamp = Some(timestamp);
                return (Some(()), None);
            }
        }

        (None, None)
    }

    fn external_event<'a>(
        &mut self,
        _timestamp: u64,
        _event: &Self::Event<'a>,
    ) -> (Option<()>, Option<u64>) {
        // No external events to handle
        (None, None)
    }

    fn timer_event(&mut self, _timestamp: u64) -> (Option<()>, Option<u64>) {
        // No timer events needed
        (None, None)
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
