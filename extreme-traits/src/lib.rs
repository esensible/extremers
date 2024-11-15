#![no_std]
#![feature(adt_const_params)]

mod selector;
pub use selector::EngineSelector;

mod traits;
pub use crate::traits::*;

#[macro_export]
macro_rules! define_engines {
    ($enum_name:ident { $($variant:ident($engine_type:ty)),* $(,)? }) => {

        struct $enum_name_Labels;

        const $enum_name_VARIANTS: &'static [&'static str] = &[$(stringify!($variant)),*];

        impl $crate::StringList for $enum_name_Labels {
            fn index_of(value: &str) -> Option<usize> {
                $enum_name_VARIANTS.iter().position(|&x| x == value)
            }

            fn list() -> &'static [&'static str] {
                $enum_name_VARIANTS
            }
        }



        #[derive(serde::Serialize)]
        #[serde(tag = "Engine")]
        enum $enum_name {
            $(
                $variant($engine_type),
            )*
        }

        impl $crate::Engine for $enum_name {
            type Event<'a> = [u8];

            fn get_static(&self, path: &'_ str) -> Option<&'static [u8]> {
                match self {
                    $(
                        $enum_name::$variant(engine) => engine.get_static(path),
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
                    $(
                        $enum_name::$variant(engine) => engine.location_event(timestamp, location, speed),
                    )*
                }
            }

            fn external_event(&mut self, timestamp: u64, event: &Self::Event) -> (Option<()>, Option<u64>) {
                match self {
                    $(
                        $enum_name::$variant(engine) => {
                            if let Ok(event) = serde_json_core::from_slice::<$engine_type::Event>(event) {
                                return engine.external_event(timestamp, &event);
                            }
                        }
                    )*
                }
                // TODO: Try to deserialize as a selector event

                (None, None)
            }

            fn timer_event(&mut self, timestamp: u64) -> (Option<()>, Option<u64>) {
                match self {
                    $(
                        $enum_name::$variant(engine) => engine.timer_event(timestamp),
                    )*
                }
            }
        }

        impl $enum_name {
            fn from_index(index: usize) -> Option<Self> {
                match index {
                    $(
                        VARIANTS.iter().position(|&x| x == stringify!($variant)).unwrap() => Some($enum_name::$variant(Default::default())),
                    )*
                    _ => None,
                }
            }
        }
    };

}
