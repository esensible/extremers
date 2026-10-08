#![no_std]

mod selector;
pub use selector::EngineSelector;

mod traits;
pub use crate::traits::*;

// Re-exported for the macros below; not part of the public API.
#[doc(hidden)]
pub use postcard_schema;
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
///     pub EngineType {
///         Race(extreme_race::Race),
///         TuneSpeed(extreme_tune::TuneSpeed<32>),
///     }
/// }
/// ```
///
/// Generates, with the given visibility:
///
/// - `enum EngineType { Selector(EngineSelector), Race(..), .. }`, which
///   starts as `Selector` and is the [`RawEngine`] the runtime drives;
/// - `enum EngineKind { Selector, Race, .. }`, which engine;
/// - `enum EngineState { Selector(()), Race(<Race as Engine>::View), .. }`,
///   what [`RawEngine::serialize_state`] writes: the active engine's tag
///   and its view;
/// - `enum EngineEvent { Select(EngineKind), Race(<Race as Engine>::Event), .. }`,
///   what [`RawEngine::external_event`] reads.
///
/// All of them are postcard on the wire, where an enum is its variant's
/// index (a varint; one byte here) then the variant's data: the selector is
/// 0 and the engines follow from 1 in declaration order, in `EngineKind`
/// and `EngineState` alike. `EngineEvent::Select` switches engine
/// (`Selector` returns to the chooser) and cancels any pending timer; an
/// event for an engine that is not active is refused (`Err`).
///
/// Every engine type must implement [`Engine`] and `Default`. Use the macro
/// once per module. The crate using it must depend on `extreme_traits`
/// under that name (the serde derives name it).
#[macro_export]
macro_rules! define_engines {
    ($vis:vis $enum_name:ident { $($variant:ident($engine_type:ty)),* $(,)? }) => {
        $vis enum $enum_name {
            Selector($crate::EngineSelector),
            $(
                $variant($engine_type),
            )*
        }

        /// Which engine: the chooser, or one of the engines.
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq,
            $crate::serde::Serialize, $crate::serde::Deserialize, $crate::postcard_schema::Schema,
        )]
        #[serde(crate = "extreme_traits::serde")]
        #[postcard(crate = $crate::postcard_schema)]
        $vis enum EngineKind {
            Selector,
            $(
                $variant,
            )*
        }

        /// The state clients see: the active engine and its view.
        #[derive(
            Clone, Debug, PartialEq,
            $crate::serde::Serialize, $crate::serde::Deserialize, $crate::postcard_schema::Schema,
        )]
        #[serde(crate = "extreme_traits::serde")]
        #[postcard(crate = $crate::postcard_schema)]
        $vis enum EngineState {
            Selector(<$crate::EngineSelector as $crate::Engine>::View),
            $(
                $variant(<$engine_type as $crate::Engine>::View),
            )*
        }

        /// A client event: switch engine, or an event for one engine.
        #[derive(
            Clone, Debug, PartialEq,
            $crate::serde::Serialize, $crate::serde::Deserialize, $crate::postcard_schema::Schema,
        )]
        #[serde(crate = "extreme_traits::serde")]
        #[postcard(crate = $crate::postcard_schema)]
        $vis enum EngineEvent {
            /// Switch to this engine (`Selector`: back to the chooser).
            Select(EngineKind),
            $(
                $variant(<$engine_type as $crate::Engine>::Event),
            )*
        }

        impl $enum_name {
            /// A fresh engine of this kind.
            $vis fn select(kind: EngineKind) -> Self {
                match kind {
                    EngineKind::Selector => Self::Selector($crate::EngineSelector),
                    $(
                        EngineKind::$variant => Self::$variant(Default::default()),
                    )*
                }
            }

            /// Which engine is active.
            #[allow(dead_code)]
            $vis fn engine_kind(&self) -> EngineKind {
                match self {
                    Self::Selector(_) => EngineKind::Selector,
                    $(
                        Self::$variant(_) => EngineKind::$variant,
                    )*
                }
            }

            /// The active engine's view.
            $vis fn state(&self) -> EngineState {
                match self {
                    Self::Selector(engine) => EngineState::Selector($crate::Engine::view(engine)),
                    $(
                        Self::$variant(engine) => EngineState::$variant($crate::Engine::view(engine)),
                    )*
                }
            }
        }

        impl Default for $enum_name {
            fn default() -> Self {
                Self::select(EngineKind::Selector)
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

            fn serialize_state(&self) -> Result<$crate::StateBytes, ()> {
                $crate::serialize_state(&self.state())
            }

            fn external_event(&mut self, timestamp: u64, event: &[u8]) -> Result<$crate::Outcome, ()> {
                let event: EngineEvent = $crate::deserialize_event(event)?;
                match (&mut *self, &event) {
                    (this, EngineEvent::Select(kind)) => {
                        *this = Self::select(*kind);
                        Ok($crate::Outcome::CHANGED.cancel_timer())
                    }
                    $(
                        (Self::$variant(engine), EngineEvent::$variant(event)) => {
                            Ok($crate::Engine::external_event(engine, timestamp, event))
                        }
                    )*
                    // an event for an engine that is not active
                    #[allow(unreachable_patterns)]
                    _ => Err(()),
                }
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
        }
    };
}
