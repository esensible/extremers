#![no_std]
// #![feature(adt_const_params)]
// #![feature(inline_const_pat)]

mod selector;
pub use selector::{EngineSelector, SelectorEvent, StringList};

mod traits;
pub use crate::traits::*;
pub use paste::paste;

#[macro_export]
macro_rules! define_engines {
    ($enum_name:ident { $($variant:ident($engine_type:ty)),* $(,)? }) => {
        $crate::paste! {
            struct [<$enum_name Labels>];

            const [<$enum_name VARIANTS>]: &'static [&'static str] = &[$(stringify!($variant)),*];

            impl $crate::StringList for [<$enum_name Labels>] {
                fn index_of(value: &str) -> Option<usize> {
                    [<$enum_name VARIANTS>].iter().position(|&x| x == value)
                }

                fn list() -> &'static [&'static str] {
                    [<$enum_name VARIANTS>]
                }
            }

            #[derive(serde::Serialize)]
            #[serde(tag = "fuck_yeah")]
            enum $enum_name {
                Selector($crate::EngineSelector<[<$enum_name Labels>]>),
                $(
                    $variant($engine_type),
                )*
            }

            impl Default for $enum_name {
                fn default() -> Self {
                    Self::Selector(Default::default())
                }
            }

            impl $crate::RawEngine for $enum_name {

                fn to_vec(&self) -> Result<heapless::Vec<u8, MAX_MESSAGE_SIZE>, ()> {
                    match serde_json_core::ser::to_vec(self) {
                        Ok(vec) => Ok(vec),
                        Err(_) => Err(()),
                    }
                }

                fn get_static(&self, path: &'_ str) -> Option<&'static [u8]> {
                    match self {
                        Self::Selector(engine) => engine.get_static(path),
                        $(
                            Self::$variant(engine) => engine.get_static(path),
                        )*
                    }
                }

                fn location_event(
                    &mut self,
                    timestamp: u64,
                    location: Option<(f64, f64)>,
                    speed: Option<(f64, f64)>,
                ) -> (Option<()>, Option<u64>) {
                    match self {
                        Self::Selector(engine) => engine.location_event(timestamp, location, speed),
                        $(
                            Self::$variant(engine) => engine.location_event(timestamp, location, speed),
                        )*
                    }
                }

                fn external_event<'a>(
                    &mut self,
                    timestamp: u64,
                    event: &'a [u8],
                ) -> Result<(Option<heapless::Vec<u8, MAX_MESSAGE_SIZE>>, Option<u64>), ()> {
                    match self {
                        $(
                            Self::$variant(engine) => {

                                match serde_json_core::from_slice::<<$engine_type as $crate::Engine>::Event<'a>>(event) {
                                    Ok((event, _)) => {
                                        let (update, timer) = $crate::Engine::external_event(engine, timestamp, &event);
                                        let update = if let Some(update) = update {
                                            Some(self.to_vec()?)
                                        } else {
                                            None
                                        };
                                        return Ok((update, timer));
                                    }
                                    Err(e) => {
                                        log::error!("Failed to deserialize engine event: {:?}", e);
                                        // fall through
                                    }
                                }
                            },
                        )*
                        // Default case, fall through
                        _ => {},
                    }

                    // Try to deserialize as a selector event
                    // This allows engines to exit themselves
                    match serde_json_core::from_slice::<$crate::SelectorEvent<[<$enum_name Labels>]>>(event) {
                        Ok((event, _)) => {
                            *self = Self::from_index(event.index);
                            return Ok((Some(self.to_vec()?), None));
                        }
                        Err(e) => {
                            log::error!("Failed to deserialize selector event: {:?}", e);
                        }
                    }

                    return Err(())
                }

                fn timer_event(&mut self, timestamp: u64) -> (Option<()>, Option<u64>) {
                    match self {
                        Self::Selector(engine) => engine.timer_event(timestamp),
                        $(
                            Self::$variant(engine) => engine.timer_event(timestamp),
                        )*
                    }
                }
            }

            impl $enum_name {
                fn from_index(index: usize) -> Self {
                    match index {
                        $(
                            i if i == <[<$enum_name Labels>] as  $crate::StringList>::index_of(stringify!($variant)).unwrap() => {
                                Self::$variant(Default::default())
                            }
                        )*
                        _ => Self::Selector(Default::default()),
                    }
                }
            }
        }
    };
}
