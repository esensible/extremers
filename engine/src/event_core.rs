use serde::de::DeserializeOwned;

use crate::callbacks::CallbackTrait;
use crate::flatdiff::FlatDiffSer;

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

pub type SleepFn = dyn FnMut(u64, usize) -> Result<(), &'static str>;

pub trait EventEngineTrait {
    type State: FlatDiffSer;
    type Event: DeserializeOwned;

    fn handle_event(&mut self, event: Self::Event, sleep: &mut SleepFn) -> Result<bool, &'static str>;

    fn get_state(&self) -> Self::State;

    fn update_location(
        &mut self,
        timestamp: u64,
        location: Option<(f64, f64)>,
        speed: Option<(f64, f64)>,
    ) -> bool;

    fn handle_sleep(&mut self, callback: usize) -> bool;
}

pub struct EventEngine<T: EngineCore, const N: usize>(pub T, [Option<T::Callbacks>; N])
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

    fn handle_event(&mut self, event: Self::Event, sleep: &mut SleepFn) -> Result<bool, &'static str> {
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

impl<T: EngineCore + FlatDiffSer + Default, const N: usize> Default
    for EventEngine<T, N>
where
    T::Callbacks: Copy + CallbackTrait<T>,
{
    fn default() -> Self {
        EventEngine(T::default(), [None; N])
    }
}

