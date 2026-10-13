//! A package as the host holds it: a validated bundle (`extreme-bundle`),
//! `'static` (in flash on the C6), with its manifest's fields at hand and
//! its engine's program and tables ready to run.

use bundle::{
    Archive, Invalid, Limits,
    validate::{self, Engine, reason},
};
use extreme_protocol::{Manifest, Name, PackageId, UiHashes};
use serde::Deserialize;

/// A bundle, validated (see [`Package::load`]).
#[derive(Clone, Copy, Debug)]
pub struct Package {
    pub id: PackageId,
    /// Borrowed from `manifest.bin`.
    pub name: &'static str,
    pub version: u16,
    /// `bundle::validate::ABI` with an engine, 0 without (Choose).
    pub abi: u16,
    pub content_hash: [u8; 8],
    /// Its Kindle page's and watch bundle's own hashes.
    pub ui: UiHashes,
    pub archive: Archive<'static>,
    /// The program (loaded: parsed and validated) and its tables; `None`
    /// for a page only (Choose).
    pub engine: Option<Engine<'static>>,
}

/// `manifest.bin` with the name borrowed, so a [`Package`] stays `Copy`
/// (the same fields, in [`Manifest`]'s order).
#[derive(Deserialize)]
struct ManifestRef<'a> {
    id: PackageId,
    name: &'a str,
    version: u16,
    abi: u16,
    content_hash: [u8; 8],
    ui: UiHashes,
    #[allow(dead_code)] // `watch.js`'s length, checked by `validate`
    watch_len: u32,
}

impl Package {
    /// Parses and validates the bundle `bytes` for a host with `limits`
    /// (`scratch` is lent to `vm::load`, at least `limits.code_bytes`
    /// long). Nothing in a bundle is trusted before this.
    pub fn load(bytes: &'static [u8], limits: &Limits, scratch: &mut [u16]) -> Result<Package, Invalid> {
        let archive = Archive::parse(bytes).map_err(|e| Invalid::Reason {
            reason: reason::ARCHIVE,
            detail: e as u32,
        })?;
        let validated = validate::validate(&archive, limits, scratch)?;
        // `validate` decoded it already, owned; this borrows the name
        let manifest = archive
            .file("manifest.bin")
            .and_then(|b| postcard::from_bytes::<ManifestRef>(b).ok())
            .ok_or(Invalid::Reason {
                reason: reason::MANIFEST,
                detail: 0,
            })?;
        Ok(Package {
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            abi: manifest.abi,
            content_hash: manifest.content_hash,
            ui: manifest.ui,
            archive,
            engine: validated.engine,
        })
    }

    /// The id, name and version in the archive `bytes`'s `manifest.bin`,
    /// read without validating anything else: for listings (the `dump`
    /// tool), never for running.
    pub fn manifest_of(bytes: &[u8]) -> Option<ManifestInfo<'_>> {
        let archive = Archive::parse(bytes).ok()?;
        let m = postcard::from_bytes::<ManifestRef>(archive.file("manifest.bin")?).ok()?;
        Some(ManifestInfo { id: m.id, name: m.name, version: m.version })
    }

    pub fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id,
            // `validate` decoded it as a `Name`, so it fits
            name: Name::try_from(self.name).unwrap_or_default(),
            version: self.version,
            abi: self.abi,
            content_hash: self.content_hash,
            ui: self.ui,
            watch_len: self.watch().len() as u32,
        }
    }

    /// The Kindle page's file `name` (no leading slash; "" is `index.html`):
    /// `web/<name>` in the archive.
    pub fn web_file(&self, name: &str) -> Option<&'static [u8]> {
        let name = if name.is_empty() { "index.html" } else { name };
        let mut path = heapless::String::<WEB_PATH_MAX>::new();
        path.push_str("web/").ok()?;
        path.push_str(name).ok()?;
        self.archive.file(&path)
    }

    /// The watch bundle; empty for a package without one.
    pub fn watch(&self) -> &'static [u8] {
        self.archive.file("watch.js").unwrap_or(&[])
    }
}

/// What [`Package::manifest_of`] reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManifestInfo<'a> {
    pub id: PackageId,
    pub name: &'a str,
    pub version: u16,
}

/// Longest `web/<name>` looked up; longer names are not found.
const WEB_PATH_MAX: usize = 128;
