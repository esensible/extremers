#![no_std]

mod geo_math;
mod maps;
mod race;
mod types;
pub use race::RaceMap;

#[cfg(test)]
mod race_tests;
