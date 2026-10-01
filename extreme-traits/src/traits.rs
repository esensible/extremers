//! The engine API.
//!
//! An [`Engine`] is a pure state machine: it is fed GPS updates, timer
//! expiries and client events, and reports back whether its state changed
//! and whether it wants a timer. It performs no I/O and knows nothing about
//! clocks or hardware, which is what makes it testable on a host.
//!
//! [`RawEngine`] is the byte-level view of an engine that the runtime uses:
//! events arrive as JSON bytes and state leaves as JSON bytes.

use serde::{Deserialize, Serialize};

/// Largest serialized engine state or client event, in bytes.
pub const MAX_MESSAGE_SIZE: usize = 512;

/// A serialized engine state.
pub type StateJson = heapless::Vec<u8, MAX_MESSAGE_SIZE>;

/// A GPS position, in degrees. South and west are negative.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Fix {
    pub lat: f64,
    pub lon: f64,
}

/// Speed over ground in knots and course over ground in degrees true.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Velocity {
    pub speed: f64,
    pub heading: f64,
}

/// What an engine wants done with its timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Timer {
    /// Leave any existing timer alone.
    #[default]
    Keep,
    /// Deliver a [`Engine::timer_event`] at this timestamp (milliseconds
    /// since the epoch), replacing any existing timer.
    At(u64),
    /// Cancel any existing timer.
    Cancel,
}

/// The result of feeding an event to an engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[must_use]
pub struct Outcome {
    /// The engine's state changed and clients should be told.
    pub changed: bool,
    pub timer: Timer,
}

impl Outcome {
    /// Nothing changed and the timer is left alone.
    pub const NONE: Outcome = Outcome {
        changed: false,
        timer: Timer::Keep,
    };

    /// State changed, timer left alone.
    pub const CHANGED: Outcome = Outcome {
        changed: true,
        timer: Timer::Keep,
    };

    pub const fn changed(changed: bool) -> Self {
        Outcome {
            changed,
            timer: Timer::Keep,
        }
    }

    pub const fn with_timer(self, at: u64) -> Self {
        Outcome {
            timer: Timer::At(at),
            ..self
        }
    }

    pub const fn cancel_timer(self) -> Self {
        Outcome {
            timer: Timer::Cancel,
            ..self
        }
    }
}

/// Static files embedded in an engine, as `(path, contents)` pairs.
pub type StaticFiles = &'static [(&'static str, &'static [u8])];

/// A typed engine. See the module docs.
pub trait Engine: Serialize {
    /// The name clients see in the `kind` field of every state message.
    const NAME: &'static str;

    /// The engine's web UI. See [`static_files!`](crate::static_files).
    const STATIC_FILES: StaticFiles;

    /// Events sent by this engine's client, as JSON.
    type Event<'a>: Deserialize<'a>;

    /// A GPS update. `timestamp` is epoch milliseconds, from the GPS when
    /// available, otherwise the runtime's best estimate.
    fn location_event(
        &mut self,
        timestamp: u64,
        fix: Option<Fix>,
        velocity: Option<Velocity>,
    ) -> Outcome;

    /// An event from a client.
    fn external_event(&mut self, timestamp: u64, event: &Self::Event<'_>) -> Outcome;

    /// A timer requested through [`Timer::At`] has expired.
    fn timer_event(&mut self, timestamp: u64) -> Outcome;

    /// Look up a static file; `path` has no leading slash.
    fn get_static(&self, path: &str) -> Option<&'static [u8]> {
        Self::STATIC_FILES
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, contents)| *contents)
    }
}

/// The byte-level view of an engine used by the runtime.
///
/// Every [`Engine`] is a `RawEngine` through the blanket impl below;
/// [`define_engines!`](crate::define_engines) builds one that switches
/// between several engines.
pub trait RawEngine {
    /// The name of the currently active engine.
    fn kind(&self) -> &'static str;

    /// Serialize the current state to JSON.
    fn serialize_state(&self) -> Result<StateJson, ()>;

    /// Deliver a JSON event from a client. `Err` means the event could not
    /// be decoded.
    fn external_event(&mut self, timestamp: u64, event: &[u8]) -> Result<Outcome, ()>;

    fn location_event(
        &mut self,
        timestamp: u64,
        fix: Option<Fix>,
        velocity: Option<Velocity>,
    ) -> Outcome;

    fn timer_event(&mut self, timestamp: u64) -> Outcome;

    fn get_static(&self, path: &str) -> Option<&'static [u8]>;
}

/// Serialize an engine state to JSON. `Err` if it does not fit in
/// [`MAX_MESSAGE_SIZE`].
pub fn serialize_state<S: Serialize>(state: &S) -> Result<StateJson, ()> {
    serde_json_core::to_vec(state).map_err(|_| ())
}

/// Decode a client event from JSON.
pub fn deserialize_event<'a, E: Deserialize<'a>>(event: &'a [u8]) -> Result<E, ()> {
    serde_json_core::from_slice(event)
        .map(|(event, _)| event)
        .map_err(|_| ())
}

impl<E: Engine> RawEngine for E {
    fn kind(&self) -> &'static str {
        E::NAME
    }

    fn serialize_state(&self) -> Result<StateJson, ()> {
        serialize_state(self)
    }

    fn external_event(&mut self, timestamp: u64, event: &[u8]) -> Result<Outcome, ()> {
        let event = deserialize_event::<E::Event<'_>>(event)?;
        Ok(Engine::external_event(self, timestamp, &event))
    }

    fn location_event(
        &mut self,
        timestamp: u64,
        fix: Option<Fix>,
        velocity: Option<Velocity>,
    ) -> Outcome {
        Engine::location_event(self, timestamp, fix, velocity)
    }

    fn timer_event(&mut self, timestamp: u64) -> Outcome {
        Engine::timer_event(self, timestamp)
    }

    fn get_static(&self, path: &str) -> Option<&'static [u8]> {
        Engine::get_static(self, path)
    }
}
