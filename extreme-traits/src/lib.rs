
pub trait Engine: serde::Serialize {
    type Event: serde::de::DeserializeOwned;

    /// Update the location of the engine
    /// Returns:
    /// * Some(()) if the engine state has changed, None otherwise
    /// * Some(timestamp) if a timer event is needed at `timestamp`. Some(0) will cancel any existing timer. None will result in no changes to any existing timer.
    fn location_event(&mut self, timestamp: u64, location: Option<(f64, f64)>, speed: Option<(f64, f64)>) -> (Option<()>, Option<u64>);
    fn external_event(&mut self, timestamp: u64, event: Self::Event) -> (Option<()>, Option<u64>);
    fn timer_event(&mut self, timestamp: u64) -> (Option<()>, Option<u64>);
}
