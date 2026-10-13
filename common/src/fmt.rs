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

/// Bytes as lowercase hex digits, for `Disp`.
#[allow(dead_code)] // the BLE transport's
pub(crate) struct Hex<'a>(pub &'a [u8]);

impl core::fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// A duration in milliseconds, with as many decimals as it needs (a BLE
/// connection interval is a multiple of 1.25 ms), for `Disp`.
#[allow(dead_code)] // the BLE transport's
pub(crate) struct Millis(pub embassy_time::Duration);

impl core::fmt::Display for Millis {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let us = self.0.as_micros();
        let (ms, mut frac, mut digits) = (us / 1000, us % 1000, 3);
        write!(f, "{ms}")?;
        if frac == 0 {
            return Ok(());
        }
        while frac % 10 == 0 {
            frac /= 10;
            digits -= 1;
        }
        write!(f, ".{frac:0digits$}")
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

#[cfg(test)]
mod tests {
    extern crate std;
    use std::format;

    use embassy_time::Duration;

    use super::Millis;

    #[test]
    fn millis_has_the_decimals_it_needs() {
        let ms = |us| format!("{}", Millis(Duration::from_micros(us)));
        assert_eq!(ms(7_500), "7.5");
        assert_eq!(ms(11_250), "11.25");
        assert_eq!(ms(15_000), "15");
        assert_eq!(ms(1_001), "1.001");
        assert_eq!(ms(0), "0");
    }
}
