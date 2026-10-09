//! The packages the firmware ships with (README "Packages"): Race and Tune
//! preinstalled, and the built-in Choose's bundles. build.rs builds them
//! with tools/package and embeds them (in flash on the C6); every target's
//! host starts its registry from [`PACKAGES`].

#![no_std]

pub use extreme_protocol::Package;
use extreme_protocol::UiHashes;

/// What the firmware ships with.
pub struct Packages {
    /// The app packages, in Choose's order.
    pub preinstalled: &'static [Package],
    /// The built-in chooser (id 0, no engine).
    pub choose: Package,
}

pub static PACKAGES: Packages = include!(concat!(env!("OUT_DIR"), "/packages.rs"));
