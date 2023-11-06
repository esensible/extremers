#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
#[macro_use]
extern crate serde_json;

mod engine_httpd;

#[cfg(test)]
mod tests;

pub use engine_httpd::Response;
pub use engine_httpd::Response::Complete;
pub use engine_httpd::StaticHttpTrait;
pub use engine_httpd::EngineHttpd;
