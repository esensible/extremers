//! What the firmware ships with (README "Packages"): the built-in Choose,
//! as the bundle `tools/package` builds (`cargo run -p extreme-pack --
//! --bundle` writes it, `-- --check` checks it is current). Embedded in
//! flash on the C6, 64-byte aligned like a bundle in the store.

#![no_std]

#[repr(C, align(64))]
struct Aligned<const N: usize>([u8; N]);

static CHOOSE_BUNDLE: &Aligned<{ include_bytes!("../../choose/choose.bundle").len() }> =
    &Aligned(*include_bytes!("../../choose/choose.bundle"));

/// Choose's bundle (no engine: `manifest.bin`, `web/*`, `watch.js`).
pub static CHOOSE: &[u8] = &CHOOSE_BUNDLE.0;
