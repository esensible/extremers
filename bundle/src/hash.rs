//! The manifest's hashes (README "Packages"): what `extreme-pack` writes
//! into `manifest.bin` and a host checks when a bundle is uploaded. Each is
//! the first 8 bytes of a SHA-256:
//!
//! - `content_hash`: `engine.bin` (none for a page alone), the Kindle page,
//!   the watch bundle, in that order: changes whenever any of them does;
//! - `ui.web`: the Kindle page alone, its files in name order, each as
//!   name, NUL, length (u32 LE) and contents;
//! - `ui.watch`: `watch.js` as is, or all zeros without one
//!   ([`NO_WATCH_BUNDLE`]), so a watch can check what it fetched.

use extreme_protocol::{Manifest, NO_WATCH_BUNDLE, UiHashes};
use sha2::{Digest, Sha256};

use crate::{
    archive::Archive,
    validate::{Invalid, reason},
};

/// The content hash of `engine`, the Kindle page `web` ((name without
/// `web/`, contents) in name order) and the watch bundle `watch`.
pub fn content_hash<'a>(engine: &[u8], web: impl IntoIterator<Item = (&'a str, &'a [u8])>, watch: &[u8]) -> [u8; 8] {
    let mut h = Sha256::new();
    h.update(engine);
    hash_web(&mut h, web);
    h.update(watch);
    prefix(h)
}

/// The Kindle page's own hash ([`UiHashes::web`]): its files as the content
/// hash takes them, nothing else.
pub fn web_hash<'a>(web: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> [u8; 8] {
    let mut h = Sha256::new();
    hash_web(&mut h, web);
    prefix(h)
}

/// The watch bundle's own hash ([`UiHashes::watch`]); [`NO_WATCH_BUNDLE`]
/// for none.
pub fn watch_hash(watch: &[u8]) -> [u8; 8] {
    if watch.is_empty() {
        return NO_WATCH_BUNDLE;
    }
    prefix(Sha256::new_with_prefix(watch))
}

fn hash_web<'a>(h: &mut Sha256, web: impl IntoIterator<Item = (&'a str, &'a [u8])>) {
    for (name, contents) in web {
        h.update(name.as_bytes());
        h.update([0]);
        h.update((contents.len() as u32).to_le_bytes());
        h.update(contents);
    }
}

fn prefix(h: Sha256) -> [u8; 8] {
    let digest = h.finalize();
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    out
}

/// The Kindle page's files in `archive` (`web/<name>`, as `(name, contents)`),
/// in name order.
pub fn web_files<'a>(archive: &Archive<'a>) -> impl Iterator<Item = (&'a str, &'a [u8])> + 'a {
    // names are sorted, so `web/` files are in order without the prefix too
    archive.files().filter_map(|(n, c)| n.strip_prefix("web/").map(|n| (n, c)))
}

/// The hashes `archive`'s files have: (content hash, UI hashes).
pub fn hashes(archive: &Archive) -> ([u8; 8], UiHashes) {
    let engine = archive.file("engine.bin").unwrap_or(&[]);
    let watch = archive.file("watch.js").unwrap_or(&[]);
    let content = content_hash(engine, web_files(archive), watch);
    (content, UiHashes { web: web_hash(web_files(archive)), watch: watch_hash(watch) })
}

/// Whether `manifest`'s hashes are those of `archive`'s files:
/// [`reason::MANIFEST`] with detail 2 (`content_hash`), 3 (`ui.web`) or 4
/// (`ui.watch`) if not.
pub fn check(archive: &Archive, manifest: &Manifest) -> Result<(), Invalid> {
    let (content, ui) = hashes(archive);
    let wrong = |detail| Err(Invalid::Reason { reason: reason::MANIFEST, detail });
    if manifest.content_hash != content {
        return wrong(2);
    }
    if manifest.ui.web != ui.web {
        return wrong(3);
    }
    if manifest.ui.watch != ui.watch {
        return wrong(4);
    }
    Ok(())
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use extreme_protocol::{Name, package_id};

    /// An archive's hashes are those of its parts: the page without `web/`,
    /// in name order, and nothing else of the archive.
    #[test]
    fn an_archive_is_hashed_as_its_parts() {
        let web: [(&str, &[u8]); 2] = [("a.js", b"x"), ("index.html", b"<p>")];
        let mut manifest = Manifest {
            id: package_id("t"),
            name: Name::try_from("t").unwrap(),
            version: 1,
            abi: 2,
            content_hash: content_hash(b"engine", web, b"watch"),
            ui: UiHashes { web: web_hash(web), watch: watch_hash(b"watch") },
            watch_len: 5,
        };
        let m = postcard::to_stdvec(&manifest).unwrap();
        let files: [(&str, &[u8]); 6] = [
            ("manifest.bin", &m),
            ("engine.bin", b"engine"),
            ("layout.bin", b"not hashed"),
            ("watch.js", b"watch"),
            ("web/index.html", b"<p>"),
            ("web/a.js", b"x"),
        ];
        let bytes = crate::archive::write(&files);
        let archive = Archive::parse(&bytes).unwrap();
        assert_eq!(hashes(&archive), (manifest.content_hash, manifest.ui));
        assert_eq!(check(&archive, &manifest), Ok(()));
        let reason = |detail| Err(Invalid::Reason { reason: reason::MANIFEST, detail });
        manifest.ui.watch = NO_WATCH_BUNDLE;
        assert_eq!(check(&archive, &manifest), reason(4));
        manifest.ui.web[0] ^= 1;
        assert_eq!(check(&archive, &manifest), reason(3));
        manifest.content_hash[7] ^= 1;
        assert_eq!(check(&archive, &manifest), reason(2));
        // SHA-256("abc") = ba7816bf 8f01cfea ...; no watch bundle: zeros
        assert_eq!(watch_hash(b"abc"), [0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea]);
        assert_eq!(watch_hash(b""), NO_WATCH_BUNDLE);
    }
}
