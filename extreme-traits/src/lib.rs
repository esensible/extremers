#![no_std]

mod selector;
pub use selector::EngineSelector;

mod traits;
pub use crate::traits::*;

// Re-exported for the macros below; not part of the public API.
#[doc(hidden)]
pub use serde;

/// Embeds the engine's web UI, built by `extreme_build::embed_client_js` in
/// the crate's `build.rs`.
///
/// ```ignore
/// const STATIC_FILES: StaticFiles = extreme_traits::static_files!();
/// ```
#[macro_export]
macro_rules! static_files {
    () => {
        include!(concat!(env!("OUT_DIR"), "/static_files.rs"))
    };
}

/// Combines several engines into one [`RawEngine`] that clients can switch
/// between.
///
/// ```ignore
/// define_engines! {
///     EngineType {
///         Race(extreme_race::Race),
///         TuneSpeed(extreme_tune::TuneSpeed<32>),
///     }
/// }
/// ```
///
/// Generates `enum EngineType { Selector(EngineSelector), Race(..), .. }`,
/// which starts as `Selector`, serializes as whichever engine is active and
/// reports that engine's name as its [`kind`](RawEngine::kind).
///
/// Client events are externally tagged with the engine name:
/// `{"Race": <race event>}` goes to the race engine and is ignored unless it
/// is active; `{"Select": "Race"}` switches engine (any unknown name returns
/// to the selector) and cancels any pending timer.
///
/// On the compact (BLE) protocol the selector is kind code 0 and the engines
/// are numbered from 1 in declaration order; `[0x01, code]` selects one and
/// any other event goes to the active engine's `compact_event`.
///
/// Every engine type must implement [`Engine`] and `Default`. Use the macro
/// once per module: it also defines `EngineEvent` and `ENGINE_NAMES`.
#[macro_export]
macro_rules! define_engines {
    ($enum_name:ident { $($variant:ident($engine_type:ty)),* $(,)? }) => {
        /// Names of the selectable engines, in declaration order.
        const ENGINE_NAMES: &'static [&'static str] = &[$(stringify!($variant)),*];

        enum $enum_name {
            Selector($crate::EngineSelector),
            $(
                $variant($engine_type),
            )*
        }

        /// A client event, externally tagged with the engine it is for.
        enum EngineEvent<'a> {
            /// Switch to the named engine; any other name returns to the selector.
            Select(&'a str),
            $(
                $variant(<$engine_type as $crate::Engine>::Event<'a>),
            )*
        }

        impl $enum_name {
            fn selector() -> Self {
                Self::Selector($crate::EngineSelector::new(ENGINE_NAMES))
            }

            fn select(name: &str) -> Self {
                match name {
                    $(
                        stringify!($variant) => Self::$variant(Default::default()),
                    )*
                    _ => Self::selector(),
                }
            }
        }

        impl Default for $enum_name {
            fn default() -> Self {
                Self::selector()
            }
        }

        impl $crate::serde::Serialize for $enum_name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: $crate::serde::Serializer,
            {
                match self {
                    Self::Selector(engine) => engine.serialize(serializer),
                    $(
                        Self::$variant(engine) => engine.serialize(serializer),
                    )*
                }
            }
        }

        impl<'de> $crate::serde::Deserialize<'de> for EngineEvent<'de> {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: $crate::serde::Deserializer<'de>,
            {
                use $crate::serde::de::{Error, MapAccess, Visitor};

                struct EventVisitor;

                impl<'de> Visitor<'de> for EventVisitor {
                    type Value = EngineEvent<'de>;

                    fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                        f.write_str("a map with a single engine-name key")
                    }

                    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                    where
                        A: MapAccess<'de>,
                    {
                        const KEYS: &[&str] = &["Select", $(stringify!($variant)),*];

                        let key: &'de str = map
                            .next_key()?
                            .ok_or_else(|| Error::invalid_length(0, &self))?;
                        let event = match key {
                            "Select" => EngineEvent::Select(map.next_value()?),
                            $(
                                stringify!($variant) => EngineEvent::$variant(map.next_value()?),
                            )*
                            _ => return Err(Error::unknown_variant(key, KEYS)),
                        };
                        if map.next_key::<&str>()?.is_some() {
                            return Err(Error::custom("expected a single key"));
                        }
                        Ok(event)
                    }
                }

                deserializer.deserialize_map(EventVisitor)
            }
        }

        impl $crate::RawEngine for $enum_name {
            fn kind(&self) -> &'static str {
                match self {
                    Self::Selector(_) => <$crate::EngineSelector as $crate::Engine>::NAME,
                    $(
                        Self::$variant(_) => stringify!($variant),
                    )*
                }
            }

            fn serialize_state(&self) -> Result<$crate::StateJson, ()> {
                $crate::serialize_state(self)
            }

            fn external_event(&mut self, timestamp: u64, event: &[u8]) -> Result<$crate::Outcome, ()> {
                let event: EngineEvent<'_> = $crate::deserialize_event(event)?;
                Ok(match (&mut *self, &event) {
                    (this, EngineEvent::Select(name)) => {
                        *this = Self::select(name);
                        $crate::Outcome::CHANGED.cancel_timer()
                    }
                    $(
                        (Self::$variant(engine), EngineEvent::$variant(event)) => {
                            $crate::Engine::external_event(engine, timestamp, event)
                        }
                    )*
                    // an event for an engine that is not active
                    _ => $crate::Outcome::NONE,
                })
            }

            fn location_event(
                &mut self,
                timestamp: u64,
                fix: Option<$crate::Fix>,
                velocity: Option<$crate::Velocity>,
            ) -> $crate::Outcome {
                match self {
                    Self::Selector(engine) => $crate::Engine::location_event(engine, timestamp, fix, velocity),
                    $(
                        Self::$variant(engine) => $crate::Engine::location_event(engine, timestamp, fix, velocity),
                    )*
                }
            }

            fn timer_event(&mut self, timestamp: u64) -> $crate::Outcome {
                match self {
                    Self::Selector(engine) => $crate::Engine::timer_event(engine, timestamp),
                    $(
                        Self::$variant(engine) => $crate::Engine::timer_event(engine, timestamp),
                    )*
                }
            }

            fn get_static(&self, path: &str) -> Option<&'static [u8]> {
                match self {
                    Self::Selector(engine) => $crate::Engine::get_static(engine, path),
                    $(
                        Self::$variant(engine) => $crate::Engine::get_static(engine, path),
                    )*
                }
            }

            fn kind_code(&self) -> u8 {
                match self {
                    Self::Selector(_) => 0,
                    $(
                        Self::$variant(_) => $crate::engine_code(ENGINE_NAMES, stringify!($variant)),
                    )*
                }
            }

            fn compact_state(&self, now: u64) -> $crate::CompactState {
                let code = $crate::RawEngine::kind_code(self);
                match self {
                    Self::Selector(engine) => $crate::compact_state(code, engine, now),
                    $(
                        Self::$variant(engine) => $crate::compact_state(code, engine, now),
                    )*
                }
            }

            fn compact_event(&mut self, timestamp: u64, event: &[u8]) -> Result<$crate::Outcome, ()> {
                if let [$crate::COMPACT_SELECT, code] = *event {
                    *self = match code {
                        $(
                            c if c == $crate::engine_code(ENGINE_NAMES, stringify!($variant)) => {
                                Self::$variant(Default::default())
                            }
                        )*
                        _ => Self::selector(),
                    };
                    return Ok($crate::Outcome::CHANGED.cancel_timer());
                }
                match self {
                    Self::Selector(engine) => $crate::Engine::compact_event(engine, timestamp, event),
                    $(
                        Self::$variant(engine) => $crate::Engine::compact_event(engine, timestamp, event),
                    )*
                }
            }
        }
    };
}

/// Compact-protocol opcode that selects an engine: `[0x01, kind code]`.
pub const COMPACT_SELECT: u8 = 0x01;

/// The compact-protocol code of an engine: its position in `names` plus one
/// (0 is the selector).
#[doc(hidden)]
pub const fn engine_code(names: &[&str], name: &str) -> u8 {
    let mut i = 0;
    while i < names.len() {
        if str_eq(names[i], name) {
            return (i + 1) as u8;
        }
        i += 1;
    }
    0
}

const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}
