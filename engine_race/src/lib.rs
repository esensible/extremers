#![no_std]

mod line;
mod types;
mod race;
mod geo_math;
pub use lib_httpd::{StaticHttpTrait, EngineHttpd};

use engine::EventEngine;

pub use race::Race;


include!(concat!(env!("OUT_DIR"), "/static_files.rs"));
#[derive(Default)]
pub struct RaceStaticFiles { }

impl StaticHttpTrait for RaceStaticFiles {
    fn lookup(key: &str) -> Option<&'static [u8]> {
        for &(k, v) in STATIC_FILES.iter() {
            if k == key {
                return Some(v);
            }
        }
        None
    }
}

pub type RaceHttpd = EngineHttpd<EventEngine<Race, 1>, RaceStaticFiles>;
