//! Building packages (README "Packages").
//!
//! A package is:
//!
//! - `manifest.bin`: its [`Manifest`] (postcard);
//! - `engine.wasm`: its engine (an `extreme_sdk::Engine` behind
//!   `export_engine!`), built for `wasm32-unknown-unknown` with 1-byte
//!   memory pages and exactly the linear memory it needs; none for Choose,
//!   whose view the host makes itself;
//! - `web/`: its Kindle page (`client-js/`, built with npm), served by the
//!   host at `/pkg/<id>/` (Choose's at `/`);
//! - `watch.js`: its watch bundle (`watch-js/`), empty if it has none;
//! - `schema.json`: its view's and events' postcard schemas, from
//!   `tools/bindgen` (kept with the build output, not on the device).
//!
//! The content hash is the first 8 bytes of the SHA-256 of engine.wasm,
//! the web bundle and the watch bundle, in that order, where the web bundle
//! is its files in name order, each as its name, a NUL, its length (u32
//! little-endian) and its contents ([`content_hash`]).
//!
//! `extreme-packages/build.rs` builds the preinstalled packages into the
//! firmware with this; `cargo run -p extreme-pack` writes them out
//! (`target/packages/<name>/`).

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use extreme_protocol::{Manifest, Name, PackageId, package_id};
use sha2::{Digest, Sha256};

/// The engine ABI the packages are built for (`extreme_sdk::abi::VERSION`).
pub const ABI: u16 = 1;

/// The module's stack, bytes: the engines are small and do not recurse.
/// `common::engine`'s tests check what they use of it.
pub const ENGINE_STACK: u32 = 2048;

/// A package's sources.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    /// The package's name ([`Manifest::name`]); its id is [`package_id`] of it.
    pub name: &'static str,
    pub version: u16,
    /// Its directory, from the repository root: `client-js/` (the Kindle
    /// page), `watch-js/` (the watch bundle, optional), `protocol-js/`
    /// (bindgen's output for it, with `schema.json`).
    pub dir: &'static str,
    /// The crate whose cdylib is the engine; `None` for Choose.
    pub engine: Option<&'static str>,
}

impl Spec {
    pub fn id(&self) -> PackageId {
        if self.engine.is_none() {
            extreme_protocol::CHOOSE
        } else {
            package_id(self.name)
        }
    }
}

/// The app packages preinstalled in the firmware, in Choose's order.
pub const PACKAGES: &[Spec] = &[
    Spec {
        name: "race",
        version: 1,
        dir: "extreme-race",
        engine: Some("extreme-race"),
    },
    Spec {
        name: "tune",
        version: 1,
        dir: "extreme-tune",
        engine: Some("extreme-tune"),
    },
];

/// The built-in Choose package.
pub const CHOOSE: Spec = Spec {
    name: "choose",
    version: 1,
    dir: "choose",
    engine: None,
};

/// A built package.
pub struct Built {
    pub spec: Spec,
    pub manifest: Manifest,
    /// Empty for Choose.
    pub engine: Vec<u8>,
    /// The linear memory the engine declares, bytes (0 for Choose).
    pub memory: u32,
    /// (name, contents), in name order.
    pub web: Vec<(String, Vec<u8>)>,
    /// Empty if the package has no watch bundle.
    pub watch: Vec<u8>,
}

impl Built {
    pub fn manifest_bytes(&self) -> Vec<u8> {
        postcard::to_stdvec(&self.manifest).unwrap()
    }
}

/// The repository root, from this crate's directory.
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// See the module docs.
pub fn content_hash(engine: &[u8], web: &[(String, Vec<u8>)], watch: &[u8]) -> [u8; 8] {
    let mut h = Sha256::new();
    h.update(engine);
    for (name, contents) in web {
        h.update(name.as_bytes());
        h.update([0]);
        h.update((contents.len() as u32).to_le_bytes());
        h.update(contents);
    }
    h.update(watch);
    let digest = h.finalize();
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    out
}

/// Builds `spec`: its engine, its bundles, its manifest.
pub fn build(root: &Path, spec: &Spec) -> Result<Built, String> {
    let dir = root.join(spec.dir);
    let (engine, memory) = match spec.engine {
        Some(krate) => {
            let wasm = build_engine(root, krate)?;
            let memory = memory_size(&wasm).ok_or_else(|| format!("{krate}: engine.wasm declares no memory"))?;
            (wasm, memory)
        }
        None => (Vec::new(), 0),
    };
    let web_dist = npm_build(&dir.join("client-js"))?
        .ok_or_else(|| format!("{}: no client-js/ (the Kindle page)", spec.name))?;
    let web = read_files(&web_dist)?;
    // a watch bundle builds with solid-watch's rollup config and plugins
    let solid_watch = root.join("solid-watch");
    if dir.join("watch-js/package.json").is_file() && !solid_watch.join("node_modules").is_dir() {
        npm(&solid_watch, &["ci", "--no-audit", "--no-fund"])?;
    }
    let watch = match npm_build(&dir.join("watch-js"))? {
        Some(dist) => fs::read(dist.join("watch.js")).map_err(|e| format!("{}: watch-js/dist/watch.js: {e}", spec.name))?,
        None => Vec::new(),
    };
    let manifest = Manifest {
        id: spec.id(),
        name: Name::try_from(spec.name).map_err(|_| format!("{}: name longer than 16 bytes", spec.name))?,
        version: spec.version,
        abi: ABI,
        content_hash: content_hash(&engine, &web, &watch),
    };
    Ok(Built {
        spec: *spec,
        manifest,
        engine,
        memory,
        web,
        watch,
    })
}

/// What a build of `spec` reads, for `cargo:rerun-if-changed` (only paths
/// that exist: cargo treats a missing one as always changed).
pub fn inputs(root: &Path, spec: &Spec) -> Vec<PathBuf> {
    let dir = root.join(spec.dir);
    let mut out = Vec::new();
    if spec.engine.is_some() {
        out.push(dir.join("src"));
        out.push(dir.join("Cargo.toml"));
        out.push(root.join("extreme-sdk/src"));
        out.push(root.join("extreme-sdk/Cargo.toml"));
        out.push(root.join("Cargo.lock"));
    }
    for js in ["client-js", "watch-js"] {
        for input in ["src", "package.json", "package-lock.json", "rollup.config.js"] {
            out.push(dir.join(js).join(input));
        }
    }
    out.push(dir.join("protocol-js"));
    out.push(root.join("protocol-js/protocol.js"));
    out.push(root.join("client-common"));
    out.retain(|p| p.exists());
    out
}

/// The engine crate `krate` as a wasm32 cdylib, its memory exactly what it
/// needs: linked once to find `__heap_base` (the end of its stack, data and
/// statics), then again with that much memory in 1-byte pages. Each link
/// has its own target directory, so neither invalidates the other.
fn build_engine(root: &Path, krate: &str) -> Result<Vec<u8>, String> {
    let probe = link(root, krate, "probe", &["--export=__heap_base".into()])?;
    let wasm = fs::read(&probe).map_err(|e| format!("{}: {e}", probe.display()))?;
    let heap_base = exported_global(&wasm, "__heap_base")
        .ok_or_else(|| format!("{krate}: no __heap_base in {}", probe.display()))?;
    let fit = link(
        root,
        krate,
        "fit",
        &[
            format!("--initial-memory={heap_base}"),
            format!("--max-memory={heap_base}"),
            "--page-size=1".into(),
        ],
    )?;
    fs::read(&fit).map_err(|e| format!("{}: {e}", fit.display()))
}

/// `cargo rustc` for the engine; the path of the .wasm.
fn link(root: &Path, krate: &str, phase: &str, link_args: &[String]) -> Result<PathBuf, String> {
    let target_dir = root.join("target/engines").join(phase);
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(root)
        .args(["rustc", "-p", krate, "--lib", "--crate-type", "cdylib"])
        .args(["--target", "wasm32-unknown-unknown", "--profile", "engine"])
        .arg("--target-dir")
        .arg(&target_dir)
        .arg("--")
        .args(["-C", &format!("link-arg=-zstack-size={ENGINE_STACK}")]);
    for a in link_args {
        cmd.args(["-C", &format!("link-arg={a}")]);
    }
    // When run from a build script: nothing of the outer build (its target,
    // its rustflags, its target dir) may leak into this one.
    for (key, _) in env::vars_os() {
        let key = key.to_string_lossy().into_owned();
        let leaks = key == "RUSTFLAGS"
            || key == "RUSTDOCFLAGS"
            || key == "RUSTC_WORKSPACE_WRAPPER"
            || (key.starts_with("CARGO_") && key != "CARGO_HOME" && key != "CARGO_NET_OFFLINE");
        if leaks {
            cmd.env_remove(&key);
        }
    }
    let status = cmd.status().map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err(format!("building the {krate} engine ({phase}) failed"));
    }
    Ok(target_dir
        .join("wasm32-unknown-unknown/engine")
        .join(format!("{}.wasm", krate.replace('-', "_"))))
}

/// Runs `npm ci` (first time) and `npm run build` in `dir`; its `dist/`, or
/// `None` if there is no `dir`.
fn npm_build(dir: &Path) -> Result<Option<PathBuf>, String> {
    if !dir.join("package.json").is_file() {
        return Ok(None);
    }
    if !dir.join("node_modules").is_dir() {
        npm(dir, &["ci", "--no-audit", "--no-fund"])?;
    }
    npm(dir, &["run", "build"])?;
    let dist = dir.join("dist");
    check_templates_declared(&dist)?;
    Ok(Some(dist))
}

fn npm(dir: &Path, args: &[&str]) -> Result<(), String> {
    let status = Command::new("npm")
        .args(args)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("npm (needed to build {}): {e}", dir.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`npm {}` failed in {}", args.join(" "), dir.display()))
    }
}

/// A bundle calling a Solid template that was never declared: the
/// minifier renames every declared `_tmpl$…`, so the literal surviving means
/// "ReferenceError: _tmpl$ is not defined" and a blank page at runtime
/// (seen when babel-preset-solid and preset-env ran in one pass).
fn check_templates_declared(dist: &Path) -> Result<(), String> {
    for (name, contents) in read_files(dist)? {
        if name.ends_with(".js") && String::from_utf8_lossy(&contents).contains("_tmpl$") {
            return Err(format!(
                "{}/{name} calls a Solid template (_tmpl$) that is never declared: check that \
                 babel-preset-solid runs in its own babel pass",
                dist.display()
            ));
        }
    }
    Ok(())
}

/// The files in `dir` (not recursive), in name order.
fn read_files(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_file() {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let contents = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            out.push((name, contents));
        }
    }
    out.sort();
    Ok(out)
}

// ---- just enough of the wasm binary format ----

fn leb(b: &[u8], i: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    let mut shift = 0;
    loop {
        let byte = *b.get(*i)?;
        *i += 1;
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

/// (id, contents) of each section.
fn sections(wasm: &[u8]) -> Option<Vec<(u8, &[u8])>> {
    if wasm.get(..8)? != b"\0asm\x01\0\0\0" {
        return None;
    }
    let mut i = 8;
    let mut out = Vec::new();
    while i < wasm.len() {
        let id = wasm[i];
        i += 1;
        let len = leb(wasm, &mut i)? as usize;
        out.push((id, wasm.get(i..i + len)?));
        i += len;
    }
    Some(out)
}

fn name<'a>(b: &'a [u8], i: &mut usize) -> Option<&'a [u8]> {
    let len = leb(b, i)? as usize;
    let s = b.get(*i..*i + len)?;
    *i += len;
    Some(s)
}

/// The value of the exported i32 global `export` (an `i32.const` init).
pub fn exported_global(wasm: &[u8], export: &str) -> Option<u32> {
    let sections = sections(wasm)?;
    let (_, exports) = sections.iter().find(|(id, _)| *id == 7)?;
    let mut i = 0;
    let mut index = None;
    for _ in 0..leb(exports, &mut i)? {
        let n = name(exports, &mut i)?;
        let kind = *exports.get(i)?;
        i += 1;
        let idx = leb(exports, &mut i)?;
        if n == export.as_bytes() && kind == 3 {
            index = Some(idx);
        }
    }
    let index = index?;
    // imported globals come first in the index space; the engines import
    // none
    let (_, globals) = sections.iter().find(|(id, _)| *id == 6)?;
    let mut i = 0;
    for g in 0..leb(globals, &mut i)? {
        let _ty = globals.get(i..i + 2)?; // value type, mutability
        i += 2;
        // init expression: i32.const n, end
        if *globals.get(i)? != 0x41 {
            return None;
        }
        i += 1;
        let value = leb_signed32(globals, &mut i)?;
        if *globals.get(i)? != 0x0b {
            return None;
        }
        i += 1;
        if g == index {
            return Some(value as u32);
        }
    }
    None
}

fn leb_signed32(b: &[u8], i: &mut usize) -> Option<i32> {
    let mut value: i64 = 0;
    let mut shift = 0;
    loop {
        let byte = *b.get(*i)?;
        *i += 1;
        value |= ((byte & 0x7f) as i64) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            if shift < 64 && byte & 0x40 != 0 {
                value |= -1i64 << shift;
            }
            return Some(value as i32);
        }
        if shift > 35 {
            return None;
        }
    }
}

/// The initial size of the module's memory in bytes (1-byte pages when it
/// says so, custom-page-sizes flag 0x08 with page size log2 0).
pub fn memory_size(wasm: &[u8]) -> Option<u32> {
    let sections = sections(wasm)?;
    let (_, memories) = sections.iter().find(|(id, _)| *id == 5)?;
    let mut i = 0;
    if leb(memories, &mut i)? != 1 {
        return None;
    }
    let flags = leb(memories, &mut i)?;
    let initial = leb(memories, &mut i)?;
    if flags & 1 != 0 {
        leb(memories, &mut i)?; // maximum
    }
    let page_log2 = if flags & 8 != 0 { leb(memories, &mut i)? } else { 16 };
    Some((initial << page_log2) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_covers_every_part() {
        let web = vec![("a.js".to_string(), b"x".to_vec())];
        let h = content_hash(b"wasm", &web, b"watch");
        assert_ne!(h, content_hash(b"wasm!", &web, b"watch"));
        assert_ne!(h, content_hash(b"wasm", &web, b"watch!"));
        let renamed = vec![("b.js".to_string(), b"x".to_vec())];
        assert_ne!(h, content_hash(b"wasm", &renamed, b"watch"));
        assert_eq!(h, content_hash(b"wasm", &web, b"watch"));
        // nothing at all is still a hash
        assert_eq!(content_hash(&[], &[], &[])[..4], [0xe3, 0xb0, 0xc4, 0x42]);
    }

    #[test]
    fn ids_are_distinct() {
        let mut ids: Vec<_> = PACKAGES.iter().map(Spec::id).collect();
        ids.push(CHOOSE.id());
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PACKAGES.len() + 1);
        assert!(PACKAGES.len() + 1 <= extreme_protocol::REGISTRY_MAX);
    }
}
