#![cfg_attr(not(test), no_std)]

#[cfg(test)]
#[macro_use]
extern crate serde_json;

mod engine_httpd;

#[cfg(test)]
mod tests;

use crate::engine_httpd::EngineHttpd;
use engine::RaceEngine;

pub type RaceHttpd = EngineHttpd<RaceEngine>;
pub use engine_httpd::EngineHttpdTrait;
pub use engine_httpd::Response;
pub use engine_httpd::Response::Complete;
