#![no_std]

// First, so its logging macros are visible to every module below.
#[macro_use]
mod fmt;

pub mod config;
pub mod http;
pub mod nmea;
pub mod runtime;
pub mod tasks;
