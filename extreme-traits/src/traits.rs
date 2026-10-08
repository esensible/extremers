//! The engine API.
//!
//! An [`Engine`] is a pure state machine: it is fed GPS updates, timer
//! expiries and client events, and reports back whether its state changed
//! and whether it wants a timer. It performs no I/O and knows nothing about
//! clocks or hardware, which is what makes it testable on a host.
//!
//! What clients see of an engine is its [`Engine::View`], a plain typed
//! value; what they send it is its [`Engine::Event`]. Both travel as
//! [postcard](https://docs.rs/postcard), on the websocket and over BLE alike,
//! and both derive postcard-schema's `Schema`, from which `tools/bindgen`
//! generates the clients' JS decoders and encoders.
//!
//! [`RawEngine`] is the byte-level view of an engine that the runtime uses:
//! events arrive as postcard bytes and state leaves as postcard bytes.

use core::fmt::Debug;

use postcard_schema::Schema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Largest serialized engine state ([`RawEngine::serialize_state`]), in
/// bytes. The engines' own tests check their largest views against it.
pub const MAX_STATE_SIZE: usize = 48;

/// A serialized engine state: postcard bytes of the engine's view (for a
/// [`define_engines!`](crate::define_engines) engine, of its `EngineState`:
/// the engine's tag, then its view).
pub type StateBytes = heapless::Vec<u8, MAX_STATE_SIZE>;

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
pub trait Engine {
    /// The engine's name, for logs ([`RawEngine::kind`]).
    const NAME: &'static str;

    /// The engine's web UI. See [`static_files!`](crate::static_files).
    const STATIC_FILES: StaticFiles;

    /// What clients see of the engine: a typed value built from its state
    /// by [`view`](Self::view), leaving out internals. Its serialized form
    /// must fit [`MAX_STATE_SIZE`] (less the one-byte engine tag).
    ///
    /// Absolute times in it are epoch milliseconds of the device clock;
    /// every state message also carries the device's time when it was
    /// sent, so clients can count down against their own clocks.
    type View: Clone + Debug + PartialEq + Serialize + DeserializeOwned + Schema;

    /// Events from clients. `()` for an engine that takes none.
    type Event: Clone + Debug + PartialEq + Serialize + DeserializeOwned + Schema;

    /// The view of the current state.
    fn view(&self) -> Self::View;

    /// A GPS update. `timestamp` is epoch milliseconds, from the GPS when
    /// available, otherwise the runtime's best estimate.
    fn location_event(
        &mut self,
        timestamp: u64,
        fix: Option<Fix>,
        velocity: Option<Velocity>,
    ) -> Outcome;

    /// An event from a client, arriving at `timestamp`.
    fn external_event(&mut self, timestamp: u64, event: &Self::Event) -> Outcome;

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
/// Every [`Engine`] is a `RawEngine` through the blanket impl below (state
/// = its view, events = its events);
/// [`define_engines!`](crate::define_engines) builds one that switches
/// between several engines.
pub trait RawEngine {
    /// The name of the currently active engine, for logs.
    fn kind(&self) -> &'static str;

    /// The current state, as postcard.
    fn serialize_state(&self) -> Result<StateBytes, ()>;

    /// Deliver a postcard event from a client. `Err` means the event could
    /// not be decoded, or is for an engine that is not active.
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

/// Serialize a state as postcard. `Err` if it does not fit in
/// [`MAX_STATE_SIZE`].
pub fn serialize_state<S: Serialize>(state: &S) -> Result<StateBytes, ()> {
    let mut bytes = StateBytes::new();
    // cannot fail: resizing to the capacity
    let _ = bytes.resize(MAX_STATE_SIZE, 0);
    let len = postcard::to_slice(state, &mut bytes[..])
        .map_err(|_| ())?
        .len();
    bytes.truncate(len);
    Ok(bytes)
}

/// Decode a client event from postcard. `Err` unless `event` is exactly one
/// value: trailing bytes are an error too.
pub fn deserialize_event<E: DeserializeOwned>(event: &[u8]) -> Result<E, ()> {
    match postcard::take_from_bytes(event) {
        Ok((event, [])) => Ok(event),
        _ => Err(()),
    }
}

impl<E: Engine> RawEngine for E {
    fn kind(&self) -> &'static str {
        E::NAME
    }

    fn serialize_state(&self) -> Result<StateBytes, ()> {
        serialize_state(&self.view())
    }

    fn external_event(&mut self, timestamp: u64, event: &[u8]) -> Result<Outcome, ()> {
        let event = deserialize_event::<E::Event>(event)?;
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
