#![no_std]

use core::option::Option;

pub trait Engine: serde::Serialize {
    /// Update the location of the engine
    /// Returns:
    /// * Some(()) if the engine state has changed, None otherwise
    /// * Some(timestamp) if a timer event is needed at `timestamp`. Some(0) will cancel any existing timer. None will result in no changes to any existing timer.
    fn location_event(
        &mut self,
        timestamp: u64,
        location: Option<(f64, f64)>,
        speed: Option<(f64, f64)>,
    ) -> (Option<()>, Option<u64>);
    fn external_event(&mut self, timestamp: u64, event: &[u8]) -> (Option<()>, Option<u64>);
    fn timer_event(&mut self, timestamp: u64) -> (Option<()>, Option<u64>);

    /// Get a static file from the engine, if it exists
    fn get_static(&self, path: &'_ str) -> Option<&'static [u8]>;
}
