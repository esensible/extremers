//! Logging for every target: defmt (RTT, read with probe-rs) when the
//! `defmt` feature is on (the firmware targets), the `log` crate otherwise
//! (tgt-std and the host tests).
//!
//! The macros take one format string and positional arguments, the subset
//! both crates accept. A value that only implements `Debug` or `Display`
//! goes through [`Dbg`] or [`Disp`], which format it the same way in both.

#![allow(unused_macros)]

#[cfg(feature = "defmt")]
macro_rules! trace { ($s:literal $(, $x:expr)* $(,)?) => { ::defmt::trace!($s $(, $x)*) }; }
#[cfg(feature = "defmt")]
macro_rules! debug { ($s:literal $(, $x:expr)* $(,)?) => { ::defmt::debug!($s $(, $x)*) }; }
#[cfg(feature = "defmt")]
macro_rules! info { ($s:literal $(, $x:expr)* $(,)?) => { ::defmt::info!($s $(, $x)*) }; }
#[cfg(feature = "defmt")]
macro_rules! warn { ($s:literal $(, $x:expr)* $(,)?) => { ::defmt::warn!($s $(, $x)*) }; }
#[cfg(feature = "defmt")]
macro_rules! error { ($s:literal $(, $x:expr)* $(,)?) => { ::defmt::error!($s $(, $x)*) }; }

#[cfg(not(feature = "defmt"))]
macro_rules! trace { ($s:literal $(, $x:expr)* $(,)?) => { ::log::trace!($s $(, $x)*) }; }
#[cfg(not(feature = "defmt"))]
macro_rules! debug { ($s:literal $(, $x:expr)* $(,)?) => { ::log::debug!($s $(, $x)*) }; }
#[cfg(not(feature = "defmt"))]
macro_rules! info { ($s:literal $(, $x:expr)* $(,)?) => { ::log::info!($s $(, $x)*) }; }
#[cfg(not(feature = "defmt"))]
macro_rules! warn { ($s:literal $(, $x:expr)* $(,)?) => { ::log::warn!($s $(, $x)*) }; }
#[cfg(not(feature = "defmt"))]
macro_rules! error { ($s:literal $(, $x:expr)* $(,)?) => { ::log::error!($s $(, $x)*) }; }

/// A `Debug` value as a log argument, formatted with `{:?}`.
#[cfg(feature = "defmt")]
pub(crate) use defmt::Debug2Format as Dbg;
/// A `Display` value as a log argument, formatted with `{}`.
#[cfg(feature = "defmt")]
pub(crate) use defmt::Display2Format as Disp;

/// A `Debug` value as a log argument, formatted with `{:?}`.
#[cfg(not(feature = "defmt"))]
pub(crate) struct Dbg<'a, T: core::fmt::Debug + ?Sized>(pub &'a T);

#[cfg(not(feature = "defmt"))]
impl<T: core::fmt::Debug + ?Sized> core::fmt::Debug for Dbg<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

/// A `Display` value as a log argument, formatted with `{}`.
#[cfg(not(feature = "defmt"))]
pub(crate) struct Disp<'a, T: core::fmt::Display + ?Sized>(pub &'a T);

#[cfg(not(feature = "defmt"))]
impl<T: core::fmt::Display + ?Sized> core::fmt::Display for Disp<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}
