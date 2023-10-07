use serde::de::DeserializeOwned;
use serde_json_core::{from_slice, to_slice};

use crate::core::FlatDiffSer;

pub trait EngineCore: FlatDiffSer {
    type Event: DeserializeOwned;
    type Callbacks;

    fn handle_event(
        &mut self,
        event: Self::Event,
        sleep: &dyn FnMut(usize, Self::Callbacks),
    ) -> Result<bool, &'static str>;
}

pub trait EventEngineTrait {
    type State;
    type Event: DeserializeOwned;

    fn handle_event(
        &mut self,
        event: Self::Event,
        sleep: &dyn Fn(usize, usize),
    ) -> Result<bool, &'static str>;

    fn get_state(&self) -> Self::State;
}

pub struct EventEngine<T: EngineCore, const N: usize>(T, [Option<T::Callbacks>; N])
where
    T::Event: DeserializeOwned;

impl<T: EngineCore + Clone, const N: usize> EventEngineTrait for EventEngine<T, N>
where
    T::Event: DeserializeOwned,
{
    type Event = T::Event;
    type State = T;

    fn get_state(&self) -> Self::State {
        self.0.clone()
    }

    fn handle_event(
        &mut self,
        event: Self::Event,
        sleep: &dyn Fn(usize, usize),
    ) -> Result<bool, &'static str> {
        let sleep_fn = |time, callback| {
            self.1[0] = Some(callback);
            sleep(time, 0);
        };
        self.0.handle_event(event, &sleep_fn)
    }
}

pub trait SerdeEngineTrait {
    fn handle_event(
        &mut self,
        event: &[u8],
        result: &mut [u8],
        sleep: &dyn Fn(usize, usize),
    ) -> Result<usize, &'static str>;

    fn get_state(&self, state: usize, result: &mut [u8]) -> Result<Option<usize>, &'static str>;
}

pub struct SerdeEngine<T: EventEngineTrait>(T, usize);

impl<T: EventEngineTrait> SerdeEngineTrait for SerdeEngine<T>
where
    T::State: FlatDiffSer,
    T::Event: DeserializeOwned,
{
    fn handle_event(
        &mut self,
        event: &[u8],
        result: &mut [u8],
        sleep: &dyn Fn(usize, usize),
    ) -> Result<usize, &'static str> {
        let (event, _): (T::Event, usize) = from_slice(event).expect("Invalid JSON event");

        let old_state = self.0.get_state();
        let updated = self.0.handle_event(event, sleep)?;
        if updated {
            let new_state = self.0.get_state();
            let delta =
                crate::UpdateResp::new(self.1, crate::core::FlatDiff(&new_state, &old_state));
            let len = to_slice(&delta, result).map_err(|_| "Failed to serialize delta")?;
            self.1 += 1;
            Ok(len)
        } else {
            Ok(0)
        }
    }

    fn get_state(&self, state: usize, result: &mut [u8]) -> Result<Option<usize>, &'static str> {
        if state < self.1 {
            let state = self.0.get_state();
            let state = crate::UpdateResp::new(self.1, crate::core::Flat(&state));
            let len = to_slice(&state, result).map_err(|_| "Failed to serialize state")?;
            Ok(Some(len))
        } else {
            Ok(None)
        }
    }
}
//     // fn wakeup(&mut self, pos: usize) {
//     //     if let Some(callback) = self.1[pos] {
//     //         self.1[pos] = None;
//     //         let mut args = &self.0;
//     //         CallbackTrait::invoke(&callback, &mut args);
//     //     }
//     // }

// }

impl<T: EngineCore + crate::core::FlatDiffSer + Default, const N: usize> Default
    for EventEngine<T, N>
where
    <T as EngineCore>::Callbacks: Copy,
{
    fn default() -> Self {
        EventEngine(T::default(), [None; N])
    }
}
