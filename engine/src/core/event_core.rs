use serde::de::DeserializeOwned;
use serde_json_core::{from_slice, to_slice};

use crate::core::callbacks::CallbackTrait;
use crate::core::FlatDiffSer;

pub trait EngineCore: FlatDiffSer {
    type Event: DeserializeOwned;
    type Callbacks;

    fn handle_event(
        &mut self,
        event: Self::Event,
        sleep: &mut dyn FnMut(u64, Self::Callbacks) -> Result<(), &'static str>,
    ) -> Result<bool, &'static str>;

    fn update_location(
        &mut self,
        timestamp: u64,
        location: Option<(f64, f64)>,
        speed: Option<(f64, f64)>,
    ) -> bool;
}

pub type SleepFn = dyn Fn(u64, usize) -> Result<(), &'static str>;

pub trait EventEngineTrait {
    type State: FlatDiffSer;
    type Event: DeserializeOwned;

    fn handle_event(&mut self, event: Self::Event, sleep: &SleepFn) -> Result<bool, &'static str>;

    fn get_state(&self) -> Self::State;

    fn update_location(
        &mut self,
        timestamp: u64,
        location: Option<(f64, f64)>,
        speed: Option<(f64, f64)>,
    ) -> bool;

    fn handle_sleep(&mut self, callback: usize) -> bool;
}

pub struct EventEngine<T: EngineCore, const N: usize>(T, [Option<T::Callbacks>; N])
where
    T::Event: DeserializeOwned,
    T::Callbacks: CallbackTrait<T>;

impl<T: EngineCore + Default + Clone, const N: usize> EventEngineTrait for EventEngine<T, N>
where
    T::Event: DeserializeOwned,
    T::Callbacks: CallbackTrait<T>,
{
    type Event = T::Event;
    type State = T;

    fn get_state(&self) -> Self::State {
        self.0.clone()
    }

    fn handle_event(&mut self, event: Self::Event, sleep: &SleepFn) -> Result<bool, &'static str> {
        let mut sleep_fn = |time, callback| -> Result<(), &'static str> {
            self.1[0] = Some(callback);
            sleep(time, 0)
        };
        self.0.handle_event(event, &mut sleep_fn)
    }

    fn update_location(
        &mut self,
        timestamp: u64,
        location: Option<(f64, f64)>,
        speed: Option<(f64, f64)>,
    ) -> bool {
        self.0.update_location(timestamp, location, speed)
    }

    fn handle_sleep(&mut self, callback: usize) -> bool {
        let result = if let Some(callback) = &self.1[callback] {
            CallbackTrait::invoke(callback, &mut self.0);
            true
        } else {
            false
        };
        self.1[0] = None;
        result
    }
}

pub trait SerdeEngineTrait {
    fn handle_event(
        &mut self,
        event: &[u8],
        result: &mut [u8],
        sleep: &SleepFn,
    ) -> Result<Option<usize>, &'static str>;

    fn get_state(&self, state: usize, result: &mut [u8]) -> Result<Option<usize>, &'static str>;

    fn update_location(
        &mut self,
        timestamp: u64,
        location: Option<(f64, f64)>,
        speed: Option<(f64, f64)>,
        result: &mut [u8],
    ) -> Option<usize>;

    fn handle_sleep(&mut self, updates: &mut [u8], callback: usize) -> Option<usize>;
}

pub struct SerdeEngine<T: EventEngineTrait>(T, usize);

impl<T: EventEngineTrait + Default> Default for SerdeEngine<T> {
    fn default() -> Self {
        // self.1 is the NEXT state number
        // initialize the NEXT state to 2 so the CURRENT state is 1
        // the client begins with state of 0, so this forces the initial update
        SerdeEngine(T::default(), 2)
    }
}

impl<T: EventEngineTrait> SerdeEngineTrait for SerdeEngine<T> {
    fn handle_event(
        &mut self,
        event: &[u8],
        result: &mut [u8],
        sleep: &SleepFn,
    ) -> Result<Option<usize>, &'static str> {
        let (event, _): (T::Event, usize) = from_slice(event).expect("Invalid JSON event");

        let old_state = self.0.get_state();
        let updated = self.0.handle_event(event, sleep)?;
        if updated {
            let new_state = self.0.get_state();
            let delta =
                crate::UpdateResp::new(self.1, crate::core::FlatDiff(&new_state, &old_state));
            let len = to_slice(&delta, result).map_err(|_| "Failed to serialize delta")?;
            self.1 += 1;
            Ok(Some(len))
        } else {
            Ok(None)
        }
    }

    fn get_state(&self, state: usize, result: &mut [u8]) -> Result<Option<usize>, &'static str> {
        if state + 1 < self.1 {
            let state = self.0.get_state();
            let state = crate::UpdateResp::new(self.1 - 1, crate::core::Flat(&state));
            let len = to_slice(&state, result).map_err(|_| "Failed to serialize state")?;
            Ok(Some(len))
        } else {
            Ok(None)
        }
    }

    fn update_location(
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
            let delta =
                crate::UpdateResp::new(self.1, crate::core::FlatDiff(&new_state, &old_state));
            let len = to_slice(&delta, result).ok()?;
            self.1 += 1;
            Some(len)
        } else {
            None
        }
    }

    fn handle_sleep(&mut self, result: &mut [u8], callback: usize) -> Option<usize> {
        let old_state = self.0.get_state();

        let updated = self.0.handle_sleep(callback);

        if updated {
            let new_state = self.0.get_state();
            let delta =
                crate::UpdateResp::new(self.1, crate::core::FlatDiff(&new_state, &old_state));
            let len = to_slice(&delta, result).ok()?;
            self.1 += 1;
            Some(len)
        } else {
            None
        }
    }
}

impl<T: EngineCore + crate::core::FlatDiffSer + Default, const N: usize> Default
    for EventEngine<T, N>
where
    T::Callbacks: Copy + CallbackTrait<T>,
{
    fn default() -> Self {
        EventEngine(T::default(), [None; N])
    }
}

#[cfg(test)]
#[path = "./event_core_tests.rs"]
mod event_core_tests;
