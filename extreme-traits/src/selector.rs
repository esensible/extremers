extern crate alloc; // Needed for no_std with serde
use crate::traits::Engine;
use core::fmt;
use core::marker::PhantomData;
use serde::de::{self, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub trait StringList {
    fn index_of(value: &str) -> Option<usize>;
    fn list() -> &'static [&'static str];
}

pub struct Event<Engines: StringList> {
    index: usize,
    _marker: PhantomData<Engines>,
}

impl<'de, Engines: StringList> Deserialize<'de> for Event<Engines> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EventVisitor<Engines>(PhantomData<Engines>);

        impl<'de, Engines: StringList> Visitor<'de> for EventVisitor<Engines> {
            type Value = Event<Engines>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string representing the engine name")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Event {
                    index: Engines::index_of(value).unwrap_or(0),
                    _marker: PhantomData,
                })
            }
        }

        deserializer.deserialize_str(EventVisitor(PhantomData))
    }
}

pub struct EngineSelector<Engines: StringList> {
    index: usize,
    _marker: PhantomData<Engines>,
}

impl<Engines: StringList> Serialize for EngineSelector<Engines> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("EngineSelector", 2)?;
        let engines = Engines::list();
        state.serialize_field("engines", &engines)?;
        state.end()
    }
}

impl<Engines: StringList> Engine for EngineSelector<Engines> {
    type Event<'a> = Event<Engines>;

    fn get_static(&self, _path: &'_ str) -> Option<&'static [u8]> {
        None
    }

    fn external_event(
        &mut self,
        _timestamp: u64,
        event: &Self::Event<'_>,
    ) -> (Option<()>, Option<u64>) {
        if event.index != self.index {
            self.index = event.index;
            (Some(()), None)
        } else {
            (None, None)
        }
    }

    fn location_event(
        &mut self,
        _: u64,
        _: Option<(f64, f64)>,
        _: Option<(f64, f64)>,
    ) -> (Option<()>, Option<u64>) {
        (None, None)
    }

    fn timer_event(&mut self, _: u64) -> (Option<()>, Option<u64>) {
        (None, None)
    }
}
