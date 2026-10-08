//! A package as a host holds it (not a wire type): its manifest's fields
//! and its files, `'static` (the firmware embeds its preinstalled packages
//! in flash, `extreme-packages`).

use crate::{Manifest, Name, PackageId};

/// A package's manifest and files.
#[derive(Debug)]
pub struct Package {
    pub id: PackageId,
    pub name: &'static str,
    pub version: u16,
    /// The engine ABI its engine.wasm speaks.
    pub abi: u16,
    pub content_hash: [u8; 8],
    /// The linear memory engine.wasm declares, bytes (0 without an engine).
    pub memory: u32,
    /// engine.wasm; empty for Choose, whose view the host makes.
    pub engine: &'static [u8],
    /// The Kindle page: (file name, contents).
    pub web: &'static [(&'static str, &'static [u8])],
    /// The watch bundle; empty for a package without one.
    pub watch: &'static [u8],
}

impl Package {
    pub fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id,
            // tools/package refuses longer names
            name: Name::try_from(self.name).unwrap_or_default(),
            version: self.version,
            abi: self.abi,
            content_hash: self.content_hash,
        }
    }

    /// The web bundle's file `path` (no leading slash; "" is `index.html`).
    pub fn web_file(&self, path: &str) -> Option<&'static [u8]> {
        let path = if path.is_empty() { "index.html" } else { path };
        self.web
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, contents)| *contents)
    }
}
