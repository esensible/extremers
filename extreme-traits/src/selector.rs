//! The engine chooser shown when no engine is active.
//!
//! Selection itself is handled by the enum that [`define_engines!`]
//! generates; this engine only serves the chooser UI. It has no state of
//! its own: the engines to choose from are the variants of the generated
//! `EngineKind`, which the client knows from its generated bindings.
//!
//! [`define_engines!`]: crate::define_engines

use crate::traits::{Engine, Fix, Outcome, StaticFiles, Velocity};

#[derive(Clone, Copy, Default)]
pub struct EngineSelector;

impl Engine for EngineSelector {
    const NAME: &'static str = "Selector";
    const STATIC_FILES: StaticFiles = crate::static_files!();

    type View = ();
    type Event = ();

    fn view(&self) {}

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
