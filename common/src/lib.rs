// The tests have std (host/tests.rs shares wire/rust's std test adapter).
#![cfg_attr(not(test), no_std)]

// First, so its logging macros are visible to every module below.
#[macro_use]
mod fmt;

#[cfg(feature = "ble")]
pub mod ble;
pub mod ble_settings;
pub mod bulk;
pub mod config;
pub mod engine;
pub mod host;
pub mod http;
pub mod nmea;
pub mod package;
pub mod tasks;
pub mod track;
