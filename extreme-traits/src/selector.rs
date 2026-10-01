//! The engine chooser shown when no engine is active.
//!
//! Selection itself is handled by the enum that [`define_engines!`]
//! generates; this engine only serves the chooser UI and reports the list of
//! engine names.
//!
//! [`define_engines!`]: crate::define_engines

use serde::Serialize;

use crate::traits::{Engine, Fix, Outcome, StaticFiles, Velocity};

#[derive(Clone, Copy, Serialize)]
pub struct EngineSelector {
    engines: &'static [&'static str],
}

impl EngineSelector {
    pub const fn new(engines: &'static [&'static str]) -> Self {
        Self { engines }
    }
}

impl Engine for EngineSelector {
    const NAME: &'static str = "Selector";
    const STATIC_FILES: StaticFiles = crate::static_files!();

    type Event<'a> = ();

    fn location_event(&mut self, _: u64, _: Option<Fix>, _: Option<Velocity>) -> Outcome {
        Outcome::NONE
    }

    fn external_event(&mut self, _: u64, _: &()) -> Outcome {
        Outcome::NONE
    }

    fn timer_event(&mut self, _: u64) -> Outcome {
        Outcome::NONE
    }
}
