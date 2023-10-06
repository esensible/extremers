use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json_core::{from_slice, to_slice};

use crate::core::FlatDiffSer;


pub trait EngineCore
where
    Self::Event: Deserialize<'static> + DeserializeOwned,
{
    type Event;
    type Callbacks;

    fn handle_event(
        &mut self,
        event: Self::Event,
        sleep: &dyn FnMut(u32, Self::Callbacks),
    ) -> Result<(), &'static str>;
}

pub struct Engine<T: EngineCore + FlatDiffSer, const N: usize>(T, [Option<T::Callbacks>; N]);

impl<T, const N: usize> Engine<T, N>
where
    T: EngineCore + FlatDiffSer + Clone,
    <T as EngineCore>::Callbacks: super::CallbackTrait,
{
    pub fn handle_event(
        &mut self,
        event: &[u8],
        result: &mut [u8],
        sleep: &dyn Fn(usize, usize),
    ) -> Result<usize, &'static str> {
        let (event, _): (T::Event, usize) = from_slice(event).expect("zzInvalid JSON event");

        let transformed_sleep = |time: u32, callback: T::Callbacks| {
            if let Some(pos) = self.1.iter_mut().position(|x| x.is_none()) {
                self.1[pos] = Some(callback);
                sleep(time as usize, pos);
            } else {
                panic!();
            }
        };

        let old_value = self.0.clone();
        self.0.handle_event(event, &transformed_sleep)?;
        let delta = crate::core::FlatDiff(&self.0, &old_value);
        let len = to_slice(&delta, result).map_err(|_| "Failed to serialize delta")?;
        Ok(len)
    }

    // fn wakeup(&mut self, pos: usize) {
    //     if let Some(callback) = self.1[pos] {
    //         self.1[pos] = None;
    //         let mut args = &self.0;
    //         CallbackTrait::invoke(&callback, &mut args);
    //     }
    // }


}


impl<T: EngineCore + crate::core::FlatDiffSer + Default, const N: usize> Default for Engine<T, N>
where
    <T as EngineCore>::Callbacks: Copy,
{
    fn default() -> Self {
        Engine(T::default(), [None; N])
    }
}


