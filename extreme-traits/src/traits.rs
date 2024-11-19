use core::option::Option;

pub const MAX_MESSAGE_SIZE: usize = 512;

pub trait Engine: serde::Serialize {
    type Event<'a>: serde::Deserialize<'a>;

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

    fn external_event<'a>(
        &mut self,
        timestamp: u64,
        event: &Self::Event<'a>,
    ) -> (Option<()>, Option<u64>);

    fn timer_event(&mut self, timestamp: u64) -> (Option<()>, Option<u64>);

    /// Get a static file from the engine, if it exists
    fn get_static(&self, path: &str) -> Option<&'static [u8]>;
}

pub trait RawEngine {
    fn to_vec(&self) -> Result<heapless::Vec<u8, MAX_MESSAGE_SIZE>, ()>;

    fn external_event(
        &mut self,
        timestamp: u64,
        event: &[u8],
    ) -> Result<(Option<heapless::Vec<u8, MAX_MESSAGE_SIZE>>, Option<u64>), ()>;

    fn location_event(
        &mut self,
        timestamp: u64,
        location: Option<(f64, f64)>,
        speed: Option<(f64, f64)>,
    ) -> (Option<()>, Option<u64>);

    fn timer_event(&mut self, timestamp: u64) -> (Option<()>, Option<u64>);

    /// Get a static file from the engine, if it exists
    fn get_static(&self, path: &str) -> Option<&'static [u8]>;
}

impl<E: Engine> RawEngine for E {
    fn to_vec(&self) -> Result<heapless::Vec<u8, MAX_MESSAGE_SIZE>, ()> {
        if let Ok(vec) = serde_json_core::to_vec(self) {
            Ok(vec)
        } else {
            Err(())
        }
    }

    fn external_event<'a>(
        &mut self,
        timestamp: u64,
        event: &'a [u8],
    ) -> Result<(Option<heapless::Vec<u8, MAX_MESSAGE_SIZE>>, Option<u64>), ()> {
        let event = match serde_json_core::from_slice::<E::Event<'a>>(event) {
            Ok((event, _)) => event,
            Err(_) => return Err(()),
        };
        let (update, timer) = self.external_event(timestamp, &event);
        let update = if update.is_some() {
            match self.to_vec() {
                Ok(vec) => Some(vec),
                Err(_) => return Err(()),
            }
        } else {
            None
        };
        Ok((update, timer))
    }

    fn location_event(
        &mut self,
        timestamp: u64,
        location: Option<(f64, f64)>,
        speed: Option<(f64, f64)>,
    ) -> (Option<()>, Option<u64>) {
        Engine::location_event(self, timestamp, location, speed)
    }

    fn timer_event(&mut self, timestamp: u64) -> (Option<()>, Option<u64>) {
        Engine::timer_event(self, timestamp)
    }

    fn get_static(&self, path: &str) -> Option<&'static [u8]> {
        Engine::get_static(self, path)
    }
}
