#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
#[macro_use]
extern crate serde_json;

mod engine_httpd;

#[cfg(test)]
mod tests;

use crate::engine_httpd::EngineHttpd;
use engine::RaceEngine;

pub type RaceHttpd = EngineHttpd<RaceEngine>;
