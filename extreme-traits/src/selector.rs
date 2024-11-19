use crate::traits::Engine;
use core::fmt;
use core::marker::PhantomData;
use serde::de::{self, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

include!(concat!(env!("OUT_DIR"), "/static_files.rs"));

pub trait StringList {
    fn index_of(value: &str) -> Option<usize>;
    fn list() -> &'static [&'static str];
}

pub struct SelectorEvent<Engines: StringList> {
    pub index: usize,
    _marker: PhantomData<Engines>,
}
impl<'de, Engines: StringList + 'static> Deserialize<'de> for SelectorEvent<Engines> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EventVisitor<Engines: StringList>(PhantomData<Engines>);

        impl<'de, Engines: StringList + 'static> Visitor<'de> for EventVisitor<Engines> {
            type Value = SelectorEvent<Engines>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str(r#"a map with a single key "index" pointing to a string"#)
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: de::SeqAccess<'de>,
            {
                // Attempt to deserialize the sequence into a single string (the engine name).
                let index = seq
                    .next_element::<&str>()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;

                Ok(SelectorEvent {
                    index: Engines::index_of(index).unwrap_or(usize::MAX),
                    _marker: PhantomData,
                })
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: de::MapAccess<'de>,
            {
                let mut index: Option<&str> = None;

                while let Some(key) = map.next_key::<&str>()? {
                    match key {
                        "index" => {
                            if index.is_some() {
                                return Err(de::Error::duplicate_field("index"));
                            }
                            index = Some(map.next_value()?);
                        }
                        _ => return Err(de::Error::unknown_field(key, &["index"])),
                    }
                }

                let index = index.ok_or_else(|| de::Error::missing_field("index"))?;

                Ok(SelectorEvent {
                    index: Engines::index_of(index).unwrap_or(usize::MAX),
                    _marker: PhantomData,
                })
            }
        }

        deserializer.deserialize_map(EventVisitor(PhantomData))
    }
}

pub struct EngineSelector<Engines: StringList> {
    pub index: usize,
    _marker: PhantomData<Engines>,
}

impl<Engines: StringList> Default for EngineSelector<Engines> {
    fn default() -> Self {
        Self {
            index: usize::MAX,
            _marker: PhantomData,
        }
    }
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

impl<Engines: StringList> Engine for EngineSelector<Engines>
where
    Engines: 'static,
{
    type Event<'a> = SelectorEvent<Engines>;

    fn get_static(&self, path: &'_ str) -> Option<&'static [u8]> {
        for &(k, v) in STATIC_FILES.iter() {
            if k == path {
                return Some(v);
            }
        }
        return None;
    }

    fn external_event<'a>(
        &mut self,
        _timestamp: u64,
        event: &Self::Event<'a>,
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
