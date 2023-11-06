use crate::event_core::{EventEngineTrait, SleepFn};
use flatdiff::{Flat, FlatDiff};
use serde_json_core::{from_slice, to_slice};

pub struct SerdeEngine<T: EventEngineTrait>(T, usize);

impl<T: EventEngineTrait + Default> Default for SerdeEngine<T> {
    fn default() -> Self {
        // cnt starts higher to force the client to "catch up" initially
        SerdeEngine(T::default(), 2)
    }
}

impl<T: EventEngineTrait> SerdeEngine<T> {
    pub fn handle_event(
        &mut self,
        event: &[u8],
        result: &mut [u8],
        sleep: &mut SleepFn,
    ) -> Result<Option<usize>, &'static str> {
        let (event, _): (T::Event, usize) = from_slice(event).expect("Invalid JSON event");

        let old_state = self.0.get_state();
        let updated = self.0.handle_event(event, sleep)?;
        if updated {
            let new_state = self.0.get_state();
            let delta = crate::UpdateResp::new(self.1, flatdiff::FlatDiff(&new_state, &old_state));
            let len = to_slice(&delta, result).map_err(|_| "Failed to serialize delta")?;
            self.1 += 1;
            Ok(Some(len))
        } else {
            Ok(None)
        }
    }

    pub fn get_state(
        &self,
        state: usize,
        result: &mut [u8],
    ) -> Result<Option<usize>, &'static str> {
        if state < self.1 - 1 {
            let state = self.0.get_state();
            let state = crate::UpdateResp::new(self.1 - 1, Flat(&state));
            let len = to_slice(&state, result).map_err(|_| "Failed to serialize state")?;
            Ok(Some(len))
        } else {
            Ok(None)
        }
    }

    pub fn update_location(
        &mut self,
        timestamp: u64,
        location: Option<(f64, f64)>,
        speed: Option<(f64, f64)>,
        result: &mut [u8],
    ) -> Option<usize> {
        let old_state = self.0.get_state();

        let updated = self.0.update_location(timestamp, location, speed);

        if updated {
            let new_state = self.0.get_state();
            let delta = crate::UpdateResp::new(self.1, FlatDiff(&new_state, &old_state));
            let len = to_slice(&delta, result).ok()?;
            self.1 += 1;
            Some(len)
        } else {
            None
        }
    }

    pub fn handle_sleep(&mut self, result: &mut [u8], callback: usize) -> Option<usize> {
        let old_state = self.0.get_state();

        let updated = self.0.handle_sleep(callback);

        if updated {
            let new_state = self.0.get_state();
            let delta = crate::UpdateResp::new(self.1, FlatDiff(&new_state, &old_state));
            let len = to_slice(&delta, result).ok()?;
            self.1 += 1;
            Some(len)
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "./serde_engine_tests.rs"]
mod serde_engine_tests;
