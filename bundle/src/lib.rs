//! Bundles: an engine and its UIs as one flat archive, as they travel (BLE,
//! the studio) and as the device stores them in its `bundles` partition.
//!
//! - [`archive`]: the archive format (`"XBND"`), read in place; [`archive::write`] (std) packs one.
//! - [`layout`]: `layout.bin`, where the host's inputs and outputs are among the program's leaves.
//! - [`map`]: the node maps and enums, and [`ViewSource`]/[`EventSink`], which encode the
//!   published value from the VM's output leaves and decode an event into its input leaves.
//! - [`validate`]: the install check; tables are validated, never trusted.
//! - [`hash`]: the manifest's content and UI hashes, as the packer writes them and a host checks them.
//! - [`store`]: the store's record; [`store::store_image`] (std) builds a partition image.
//!
//! `engine/profile` writes the tables; this crate reads them. `no_std`, no
//! allocation without the `std` feature.

#![cfg_attr(not(feature = "std"), no_std)]

pub mod archive;
pub mod hash;
pub mod layout;
pub mod map;
pub mod store;
pub mod validate;

pub use archive::Archive;
pub use layout::Layout;
pub use map::{Enums, EventSink, Map, ViewSource};
pub use validate::{Invalid, Limits, Validated, validate};
