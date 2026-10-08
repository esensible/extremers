#![no_std]

// First, so its logging macros are visible to every module below.
#[macro_use]
mod fmt;

#[cfg(feature = "ble")]
pub mod ble;
pub mod ble_settings;
pub mod config;
pub mod http;
pub mod nmea;
pub mod protocol;
pub mod runtime;
pub mod tasks;
