//! Building packages (README "Packages") and their bundles.
//!
//! A package is:
//!
//! - `manifest.bin`: its [`Manifest`] (postcard), `abi` [`ABI`] with an
//!   engine, [`NO_ENGINE`] without (Choose);
//! - with an engine (an engine directory, `engine/race` style:
//!   `engine.expr`, `state.schema.json`, optional `view.schema.json` and
//!   `event.schema.json`; engine/PROFILE.md — `samples.json` tests the
//!   engine and isn't packaged):
//!   - `engine.bin`: the program, compiled by `lang/compiler` against the
//!     interface `engine/profile` generates from the schemas;
//!   - `layout.bin`, `view.map`, `event.map`, `enums.bin`: the host's
//!     tables for it (`engine/profile`'s `tables`);
//!   - `view.desc`: the wire descriptor of the published value, the view
//!     schema's (the state schema's when there is no view);
//!     `event.desc`: the author's `event.schema.json`'s, if there is one.
//!     Both compiled by `wire/js` (a node call, [`compile_schema`]);
//!   - `source.json`: the engine sources verbatim, `{engine, state, view,
//!     event}` (`view`/`event` null when absent), pretty-printed, for the
//!     studio to reopen; the device ignores it. The samples are not in it:
//!     they test the engine rather than being part of it;
//! - `web/`: its Kindle page (`client-js/`, built with npm): the host
//!   serves its index.html at `/` while the package is active, and its
//!   files at `/pkg/<id>/` (Choose's id is 0), where index.html points;
//! - `watch.js`: its watch bundle (`watch-js/`), absent if it has none.
//!
//! A bundle is those files as one archive (`extreme-bundle`'s `write`; web
//! files as `web/<name>`); [`Built::bundle`].
//!
//! The content hash is the first 8 bytes of the SHA-256 of engine.bin, the
//! web bundle and the watch bundle, in that order, where the web bundle is
//! its files in name order, each as its name, a NUL, its length (u32
//! little-endian) and its contents ([`content_hash`]). The tables and
//! descriptors are not hashed on their own: they are made from the
//! interface and engine.bin, and engine.bin carries the interface's hash,
//! which covers the schemas verbatim. Each client's UI has its own hash
//! too, the same prefix of the SHA-256 of the web bundle alone
//! ([`web_hash`]) and of the watch bundle alone ([`watch_hash`]): the
//! envelope's `ui`, so a client reloads only for a change to its own UI.
//!
//! `cargo run -p extreme-pack` writes them out (`target/packages/<name>/`);
//! `extreme-packages` embeds the committed `choose/choose.bundle`.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use extreme_protocol::{Manifest, Name, PackageId, UiHashes, package_id};
use serde_json::{Value as Json, json};

/// The engine ABI of a package with an engine: lang bytecode.
pub const ABI: u16 = 2;

/// The `abi` of a package without an engine (Choose).
pub const NO_ENGINE: u16 = 0;

/// A package's sources.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    /// The package's name ([`Manifest::name`]); its id is [`package_id`] of it.
    pub name: &'static str,
    pub version: u16,
    /// Its UI directory, from the repository root: `client-js/` (the
    /// Kindle page), `watch-js/` (the watch bundle, optional).
    pub dir: &'static str,
    /// Its engine directory, from the repository root; `None` for Choose.
    pub engine: Option<&'static str>,
}

impl Spec {
    pub fn id(&self) -> PackageId {
        id(self.name, self.engine.is_some())
    }
}

/// A package's id: [`package_id`] of its name with an engine, Choose's
/// without (the one package without an engine).
pub fn id(name: &str, engine: bool) -> PackageId {
    if engine { package_id(name) } else { extreme_protocol::CHOOSE }
}

/// The manifest of the package `name` `version` made of `program` (`None`
/// without an engine), the Kindle page `web` ((name, contents) in name
/// order, names without `web/`) and the watch bundle `watch` (empty if
/// none): its id, ABI, content hash and UI hashes (module docs).
pub fn manifest(name: &str, version: u16, program: Option<&[u8]>, web: &[(String, Vec<u8>)], watch: &[u8]) -> Result<Manifest, String> {
    Ok(Manifest {
        id: id(name, program.is_some()),
        name: Name::try_from(name).map_err(|_| format!("{name}: name longer than 16 bytes"))?,
        version,
        abi: if program.is_some() { ABI } else { NO_ENGINE },
        content_hash: content_hash(program.unwrap_or(&[]), web, watch),
        ui: UiHashes {
            web: web_hash(web),
            watch: watch_hash(watch),
        },
        watch_len: u32::try_from(watch.len()).map_err(|_| format!("{name}: watch.js over 4 GiB"))?,
    })
}

/// The bundle of `files` ((path, contents)): [`bundle::archive::write`],
/// which sorts them. `Err` where `write` would panic: a name twice, a name
/// over 65,535 bytes, more than 65,535 files, or a file over 4 GiB.
pub fn archive(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    names.sort_unstable();
    if let Some(w) = names.windows(2).find(|w| w[0] == w[1]) {
        return Err(format!("{}: a file name given twice", w[0]));
    }
    if files.len() > u16::MAX as usize {
        return Err(format!("{} files: at most 65,535", files.len()));
    }
    for (name, contents) in files {
        if name.len() > u16::MAX as usize {
            return Err(format!("{}…: a name over 65,535 bytes", &name[..name.floor_char_boundary(32)]));
        }
        if u32::try_from(contents.len()).is_err() {
            return Err(format!("{name}: over 4 GiB"));
        }
    }
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, c)| (n.as_str(), c.as_slice())).collect();
    Ok(bundle::archive::write(&refs))
}

/// The app packages, in Choose's order.
pub const PACKAGES: &[Spec] = &[Spec {
    name: "race",
    version: 1,
    dir: "extreme-race",
    engine: Some("engine/race"),
}];

/// The built-in Choose package.
pub const CHOOSE: Spec = Spec {
    name: "choose",
    version: 1,
    dir: "choose",
    engine: None,
};

/// The committed Choose bundle `extreme-packages` embeds, from the root.
pub const CHOOSE_BUNDLE: &str = "choose/choose.bundle";

/// The bundles region's size (`common::config::DATA_BUNDLES_LEN`: the
/// first 512 KiB of the C6's inactive app slot): what `--store-image`
/// fills.
pub const STORE_SIZE: usize = 0x8_0000;

/// A built engine: the program, its tables and descriptors, its sources.
pub struct Engine {
    /// `engine.bin`.
    pub program: Vec<u8>,
    /// What the program needs on the device.
    pub memory: compiler::Memory,
    /// `layout.bin`.
    pub layout: Vec<u8>,
    /// `view.desc`, `view.map`.
    pub view_desc: Vec<u8>,
    pub view_map: Vec<u8>,
    /// `event.desc`, `event.map`: with author events only.
    pub event: Option<(Vec<u8>, Vec<u8>)>,
    /// `enums.bin`: if any leaf is an enum.
    pub enums: Option<Vec<u8>>,
    /// `source.json`.
    pub source: Vec<u8>,
}

/// A built package.
pub struct Built {
    pub spec: Spec,
    pub manifest: Manifest,
    /// `None` for Choose.
    pub engine: Option<Engine>,
    /// (name, contents), in name order.
    pub web: Vec<(String, Vec<u8>)>,
    /// Empty if the package has no watch bundle.
    pub watch: Vec<u8>,
}

impl Built {
    pub fn manifest_bytes(&self) -> Vec<u8> {
        postcard::to_stdvec(&self.manifest).unwrap()
    }

    /// The package's files (module docs), as (path, contents) in name order:
    /// what `--out` writes and what the bundle holds.
    pub fn files(&self) -> Vec<(String, Vec<u8>)> {
        let mut out = vec![("manifest.bin".to_string(), self.manifest_bytes())];
        if let Some(e) = &self.engine {
            out.push(("engine.bin".into(), e.program.clone()));
            out.push(("layout.bin".into(), e.layout.clone()));
            out.push(("view.desc".into(), e.view_desc.clone()));
            out.push(("view.map".into(), e.view_map.clone()));
            if let Some((desc, map)) = &e.event {
                out.push(("event.desc".into(), desc.clone()));
                out.push(("event.map".into(), map.clone()));
            }
            if let Some(enums) = &e.enums {
                out.push(("enums.bin".into(), enums.clone()));
            }
            out.push(("source.json".into(), e.source.clone()));
        }
        for (name, contents) in &self.web {
            out.push((format!("web/{name}"), contents.clone()));
        }
        if !self.watch.is_empty() {
            out.push(("watch.js".into(), self.watch.clone()));
        }
        out.sort();
        out
    }

    /// The bundle: [`Built::files`] as one archive.
    pub fn bundle(&self) -> Vec<u8> {
        let files = self.files();
        let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, c)| (n.as_str(), c.as_slice())).collect();
        bundle::archive::write(&refs)
    }
}

/// The repository root, from this crate's directory.
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// See the module docs. The hashes are defined once, in `extreme-bundle`
/// ([`bundle::hash`]), which the device checks an upload with.
pub fn content_hash(engine: &[u8], web: &[(String, Vec<u8>)], watch: &[u8]) -> [u8; 8] {
    bundle::hash::content_hash(engine, web_refs(web), watch)
}

/// The Kindle page's own hash ([`UiHashes::web`]): its files as the content
/// hash takes them, nothing else.
pub fn web_hash(web: &[(String, Vec<u8>)]) -> [u8; 8] {
    bundle::hash::web_hash(web_refs(web))
}

/// The watch bundle's own hash ([`UiHashes::watch`]): its bytes, so a watch
/// can check what it fetched against it; all zeros for a package without
/// one ([`extreme_protocol::NO_WATCH_BUNDLE`]).
pub fn watch_hash(watch: &[u8]) -> [u8; 8] {
    bundle::hash::watch_hash(watch)
}

fn web_refs(web: &[(String, Vec<u8>)]) -> impl Iterator<Item = (&str, &[u8])> {
    web.iter().map(|(n, c)| (n.as_str(), c.as_slice()))
}

/// Builds `spec`: its engine, its bundles, its manifest.
pub fn build(root: &Path, spec: &Spec) -> Result<Built, String> {
    let dir = root.join(spec.dir);
    let engine = match spec.engine {
        Some(engine_dir) => Some(build_engine(root, &root.join(engine_dir))?),
        None => None,
    };
    let public_path = format!("/pkg/{}/", spec.id());
    let web_dist = npm_build(&dir.join("client-js"), Some(&public_path))?
        .ok_or_else(|| format!("{}: no client-js/ (the Kindle page)", spec.name))?;
    let web = read_files(&web_dist)?;
    // a watch bundle builds with solid-watch's rollup config and plugins
    let solid_watch = root.join("solid-watch");
    if dir.join("watch-js/package.json").is_file() && !solid_watch.join("node_modules").is_dir() {
        npm(&solid_watch, &["ci", "--no-audit", "--no-fund"])?;
    }
    let watch = match npm_build(&dir.join("watch-js"), None)? {
        Some(dist) => fs::read(dist.join("watch.js")).map_err(|e| format!("{}: watch-js/dist/watch.js: {e}", spec.name))?,
        None => Vec::new(),
    };
    let program = engine.as_ref().map(|e| e.program.as_slice());
    let manifest = manifest(spec.name, spec.version, program, &web, &watch)?;
    Ok(Built {
        spec: *spec,
        manifest,
        engine,
        web,
        watch,
    })
}

/// The store image of `bundles` (`--store-image`): the bundles region as
/// `extreme-bundle` lays it out, for `probe-rs download` at the inactive
/// slot's offset (`tgt-xiaoc6/README.md`).
pub fn store_image(bundles: &[Vec<u8>]) -> Vec<u8> {
    let refs: Vec<&[u8]> = bundles.iter().map(Vec::as_slice).collect();
    bundle::store::store_image(&refs, STORE_SIZE)
}

/// The manifest of the bundle file `bytes` (`--from`), checked as far as
/// [`store_image`] and the device's install need: an archive
/// ([`bundle::archive::Archive::parse`]) with a `manifest.bin` that reads,
/// whose hashes are its files' ([`bundle::hash::check`]), whose id is its
/// name's and not Choose's (the firmware embeds Choose), under 4 GiB.
pub fn bundle_manifest(bytes: &[u8]) -> Result<Manifest, String> {
    let archive = bundle::archive::Archive::parse(bytes).map_err(|e| format!("not a bundle archive ({e:?})"))?;
    let m = archive.file("manifest.bin").ok_or("not a bundle: no manifest.bin")?;
    let manifest: Manifest = postcard::from_bytes(m).map_err(|e| format!("manifest.bin doesn't read: {e}"))?;
    let name = manifest.name.as_str();
    if manifest.id == extreme_protocol::CHOOSE {
        return Err(format!("{name}: Choose's bundle (id 0): the firmware embeds it"));
    }
    if manifest.id != package_id(name) {
        return Err(format!("{name}: id {:08x} isn't its name's ({:08x})", manifest.id, package_id(name)));
    }
    bundle::hash::check(&archive, &manifest).map_err(|e| format!("{name}: the manifest's hashes aren't its files' ({e:?})"))?;
    u32::try_from(bytes.len()).map_err(|_| format!("{name}: over 4 GiB"))?;
    Ok(manifest)
}

/// The bytes of the bundles region `bundles` take in a [`store_image`]:
/// each a whole number of sectors, its record included.
pub fn store_used(bundles: &[Vec<u8>]) -> usize {
    bundles
        .iter()
        .map(|b| bundle::store::Record::sectors_for(b.len() as u32) as usize * bundle::store::SECTOR)
        .sum()
}

/// What a build of `spec` reads (only paths that exist).
pub fn inputs(root: &Path, spec: &Spec) -> Vec<PathBuf> {
    let dir = root.join(spec.dir);
    let mut out = Vec::new();
    if let Some(engine) = spec.engine {
        let engine = root.join(engine);
        for file in ENGINE_FILES {
            out.push(engine.join(file));
        }
        for js in ["index.js", "codec.js", "compile.js", "rollup.js", "scripts/compile-schema.js"] {
            out.push(root.join("wire/js").join(js));
        }
    }
    for js in ["client-js", "watch-js"] {
        for input in ["src", "package.json", "package-lock.json", "rollup.config.js"] {
            out.push(dir.join(js).join(input));
        }
    }
    out.push(dir.join("protocol-js"));
    out.push(dir.join("race-wire.mjs"));
    out.push(root.join("protocol-js/protocol.js"));
    out.push(root.join("client-common"));
    out.retain(|p| p.exists());
    out
}

/// An engine directory's files (module docs); all but the first two optional.
const ENGINE_FILES: [&str; 4] = [
    "engine.expr",
    "state.schema.json",
    "view.schema.json",
    "event.schema.json",
];

/// An engine's sources, as an engine directory holds them (module docs).
#[derive(Clone, Debug)]
pub struct Sources {
    /// `engine.expr`.
    pub engine: String,
    /// `state.schema.json`, and the optional `view.schema.json` and
    /// `event.schema.json`, verbatim.
    pub state: String,
    pub view: Option<String>,
    pub event: Option<String>,
}

/// Compiles the engine in `dir` and makes its tables (module docs).
fn build_engine(root: &Path, dir: &Path) -> Result<Engine, String> {
    let read = |name: &str| -> Result<Option<String>, String> {
        let path = dir.join(name);
        match fs::read_to_string(&path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    };
    let missing = |name: &str| format!("{}: missing", dir.join(name).display());
    let sources = Sources {
        engine: read("engine.expr")?.ok_or_else(|| missing("engine.expr"))?,
        state: read("state.schema.json")?.ok_or_else(|| missing("state.schema.json"))?,
        view: read("view.schema.json")?,
        event: read("event.schema.json")?,
    };
    engine_from_sources(root, dir, &sources)
}

/// Compiles an engine from its sources and makes its tables (module docs);
/// `dir` names its files in messages.
pub fn engine_from_sources(root: &Path, dir: &Path, sources: &Sources) -> Result<Engine, String> {
    let parse = |name: &str, src: &str| -> Result<Json, String> {
        serde_json::from_str(src).map_err(|e| format!("{}: {e}", dir.join(name).display()))
    };
    let source = sources.engine.clone();
    let (state_src, view_src, event_src) = (sources.state.clone(), sources.view.clone(), sources.event.clone());
    let state = parse("state.schema.json", &state_src)?;
    let view = view_src.as_deref().map(|s| parse("view.schema.json", s)).transpose()?;
    let event = event_src.as_deref().map(|s| parse("event.schema.json", s)).transpose()?;

    let rendered = |file: &str, src: &str, diags: Vec<compiler::diag::Diag>| {
        let file = dir.join(file).display().to_string();
        diags.iter().map(|d| d.render(&file, src)).collect::<Vec<_>>().join("\n")
    };
    let interface = engine_profile::engine_interface(&state, view.as_ref(), event.as_ref())
        .map_err(|d| rendered("engine.expr", &source, d))?;
    let compiled = compiler::compile(&source, &interface, &compiler::Options::default())
        .map_err(|d| rendered("engine.expr", &source, d))?;

    // the published value is the view, or the state when there is none
    let (published, published_src) = match &view_src {
        Some(v) => ("view.schema.json", v),
        None => ("state.schema.json", &state_src),
    };
    let view_desc = compile_schema_text(root, &dir.join(published), published_src)?;
    let event_desc = match &event_src {
        Some(e) => Some(compile_schema_text(root, &dir.join("event.schema.json"), e)?),
        None => None,
    };
    let tables = engine_profile::layout::tables(&interface, &compiled.bytes, &view_desc, event_desc.as_deref())
        .map_err(|e| format!("{}: {e}", dir.display()))?;

    // Pretty-printed: it's for people and agents to reopen, not for the device.
    // The samples stay a project file: they test the engine, they aren't part
    // of it.
    let source_json = json!({
        "engine": source,
        "state": state_src,
        "view": view_src,
        "event": event_src,
    });
    let event = match (event_desc, tables.event_map) {
        (Some(desc), Some(map)) => Some((desc, map)),
        (None, None) => None,
        _ => return Err(format!("{}: an event descriptor without its map, or the other way round", dir.display())),
    };
    Ok(Engine {
        program: compiled.bytes,
        memory: compiled.memory,
        layout: tables.layout,
        view_desc,
        view_map: tables.view_map,
        event,
        enums: tables.enums,
        source: serde_json::to_vec_pretty(&source_json).unwrap(),
    })
}

/// The wire descriptor of the JSON Schema at `schema`, compiled by
/// `wire/js` (`scripts/compile-schema.js`).
pub fn compile_schema(root: &Path, schema: &Path) -> Result<Vec<u8>, String> {
    let script = root.join("wire/js/scripts/compile-schema.js");
    let out = Command::new("node")
        .arg(&script)
        .arg(schema)
        .output()
        .map_err(|e| format!("node (needed to compile {}): {e}", schema.display()))?;
    if !out.status.success() {
        return Err(format!(
            "compiling {}: {}",
            schema.display(),
            String::from_utf8_lossy(&out.stderr).trim_end()
        ));
    }
    Ok(out.stdout)
}

/// [`compile_schema`] of a schema's text, `schema` naming it in messages
/// (the script reads it from its stdin).
pub fn compile_schema_text(root: &Path, schema: &Path, text: &str) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let script = root.join("wire/js/scripts/compile-schema.js");
    let mut child = Command::new("node")
        .arg(&script)
        .arg("/dev/stdin")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("node (needed to compile {}): {e}", schema.display()))?;
    let written = child.stdin.take().map(|mut stdin| stdin.write_all(text.as_bytes()));
    let out = child.wait_with_output().map_err(|e| format!("node (compiling {}): {e}", schema.display()))?;
    if !out.status.success() {
        return Err(format!(
            "compiling {}: {}",
            schema.display(),
            String::from_utf8_lossy(&out.stderr).trim_end()
        ));
    }
    if let Some(Err(e)) = written {
        return Err(format!("compiling {}: writing it to node: {e}", schema.display()));
    }
    Ok(out.stdout)
}

/// Runs `npm ci` (first time) and `npm run build` in `dir`; its `dist/`, or
/// `None` if there is no `dir`. A Kindle page gets `public_path`, where the
/// host serves its files (`EXTREMERS_PUBLIC_PATH`, its rollup config).
fn npm_build(dir: &Path, public_path: Option<&str>) -> Result<Option<PathBuf>, String> {
    if !dir.join("package.json").is_file() {
        return Ok(None);
    }
    if !dir.join("node_modules").is_dir() {
        npm(dir, &["ci", "--no-audit", "--no-fund"])?;
    }
    npm_env(dir, &["run", "build"], public_path)?;
    let dist = dir.join("dist");
    check_templates_declared(&dist)?;
    Ok(Some(dist))
}

fn npm(dir: &Path, args: &[&str]) -> Result<(), String> {
    npm_env(dir, args, None)
}

fn npm_env(dir: &Path, args: &[&str], public_path: Option<&str>) -> Result<(), String> {
    let mut cmd = Command::new("npm");
    match public_path {
        Some(path) => cmd.env("EXTREMERS_PUBLIC_PATH", path),
        None => cmd.env_remove("EXTREMERS_PUBLIC_PATH"),
    };
    let status = cmd
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_covers_every_part() {
        let web = vec![("a.js".to_string(), b"x".to_vec())];
        let h = content_hash(b"engine", &web, b"watch");
        assert_ne!(h, content_hash(b"engine!", &web, b"watch"));
        assert_ne!(h, content_hash(b"engine", &web, b"watch!"));
        let renamed = vec![("b.js".to_string(), b"x".to_vec())];
        assert_ne!(h, content_hash(b"engine", &renamed, b"watch"));
        assert_eq!(h, content_hash(b"engine", &web, b"watch"));
        // nothing at all is still a hash
        assert_eq!(content_hash(&[], &[], &[])[..4], [0xe3, 0xb0, 0xc4, 0x42]);
    }

    /// Each UI hash covers its own client's files and nothing else.
    #[test]
    fn ui_hashes_cover_their_own_client() {
        let web = vec![("a.js".to_string(), b"x".to_vec())];
        let edited = vec![("a.js".to_string(), b"y".to_vec())];
        assert_ne!(web_hash(&web), web_hash(&edited));
        assert_ne!(watch_hash(b"watch"), watch_hash(b"watch!"));
        // the engine is in neither; the web bundle is hashed as the
        // content hash takes it, the watch bundle as plain SHA-256 (what a
        // watch checks its download against)
        assert_eq!(web_hash(&web), content_hash(&[], &web, &[]));
        assert_eq!(watch_hash(b"watch"), content_hash(&[], &[], b"watch"));
        assert_ne!(watch_hash(b"watch"), extreme_protocol::NO_WATCH_BUNDLE);
        // SHA-256("abc") = ba7816bf 8f01cfea ...
        assert_eq!(watch_hash(b"abc"), [0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea]);
        // no watch bundle: all zeros, not the hash of no bytes
        assert_eq!(watch_hash(b""), [0; 8]);
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

    /// Choose's files: the manifest, the Kindle page under `web/`, the
    /// watch bundle; nothing of an engine.
    #[test]
    fn files_without_an_engine() {
        let built = Built {
            spec: CHOOSE,
            manifest: Manifest {
                id: 0,
                name: Name::try_from("choose").unwrap(),
                version: 1,
                abi: NO_ENGINE,
                content_hash: [0; 8],
                ui: UiHashes::default(),
                watch_len: 1,
            },
            engine: None,
            web: vec![("index.html".into(), b"<p>".to_vec()), ("a.js".into(), b"x".to_vec())],
            watch: b"w".to_vec(),
        };
        let names: Vec<_> = built.files().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["manifest.bin", "watch.js", "web/a.js", "web/index.html"]);
        // the bundle holds exactly those files
        let bytes = built.bundle();
        let archive = bundle::Archive::parse(&bytes).unwrap();
        let read: Vec<_> = archive.files().map(|(n, c)| (n.to_string(), c.to_vec())).collect();
        assert_eq!(read, built.files());
    }

    /// A bundle file as `--from` takes it: an archive whose manifest is its
    /// files', with an engine.
    fn test_bundle(name: &str, engine: &[u8], page: &[u8]) -> Vec<u8> {
        let web = vec![("index.html".to_string(), page.to_vec())];
        let m = manifest(name, 3, Some(engine), &web, &[]).unwrap();
        let files = vec![
            ("manifest.bin".to_string(), postcard::to_stdvec(&m).unwrap()),
            ("engine.bin".to_string(), engine.to_vec()),
            ("web/index.html".to_string(), page.to_vec()),
        ];
        archive(&files).unwrap()
    }

    /// `--from`: an image of two bundle files holds exactly them, in order,
    /// committed as seq 1 and 2, each where the previous extent ends, the
    /// rest erased; files that aren't app bundles are refused.
    #[test]
    fn store_image_from_bundle_files() {
        use bundle::store::{COMMITTED, Record, SECTOR};
        // the second over a sector, so its extent is two
        let bundles = vec![test_bundle("alpha", b"one", b"<p>a"), test_bundle("beta", b"two", &[b'x'; 5000])];
        for (b, name) in bundles.iter().zip(["alpha", "beta"]) {
            let m = bundle_manifest(b).unwrap();
            assert_eq!((m.name.as_str(), m.id, m.version), (name, package_id(name), 3));
        }
        let img = store_image(&bundles);
        assert_eq!(img.len(), STORE_SIZE);
        assert_eq!(store_used(&bundles), 3 * SECTOR);
        // walk it as the store does: a record, its extent, the next record
        let mut at = 0;
        let mut found = Vec::new();
        while at < img.len() {
            match Record::parse(&img[at..]) {
                Ok(r) => {
                    let archive = &img[at + 64..at + 64 + r.len as usize];
                    found.push((r.state, r.seq, r.id, archive.to_vec()));
                    at += r.sectors as usize * SECTOR;
                }
                Err(_) => {
                    assert!(img[at..at + SECTOR].iter().all(|&b| b == 0xFF), "free sector {} not erased", at / SECTOR);
                    at += SECTOR;
                }
            }
        }
        let expected: Vec<_> = bundles
            .iter()
            .enumerate()
            .map(|(i, b)| (COMMITTED, i as u32 + 1, bundle_manifest(b).unwrap().id, b.clone()))
            .collect();
        assert_eq!(found, expected);

        // not an archive; Choose's; a manifest whose hashes aren't its files'
        assert!(bundle_manifest(b"hello").unwrap_err().contains("not a bundle archive"));
        let choose = fs::read(root().join(CHOOSE_BUNDLE)).unwrap();
        assert!(bundle_manifest(&choose).unwrap_err().contains("Choose"));
        let web = vec![("index.html".to_string(), b"<p>".to_vec())];
        let m = manifest("alpha", 1, Some(b"one"), &web, &[]).unwrap();
        let edited = archive(&[
            ("manifest.bin".to_string(), postcard::to_stdvec(&m).unwrap()),
            ("engine.bin".to_string(), b"one!".to_vec()),
            ("web/index.html".to_string(), b"<p>".to_vec()),
        ])
        .unwrap();
        assert!(bundle_manifest(&edited).unwrap_err().contains("hashes"));
    }

    /// The committed Choose bundle is an archive whose manifest is Choose's.
    #[test]
    fn committed_choose_bundle_reads() {
        let bytes = fs::read(root().join(CHOOSE_BUNDLE)).unwrap();
        let archive = bundle::Archive::parse(&bytes).unwrap();
        let manifest: Manifest = postcard::from_bytes(archive.file("manifest.bin").unwrap()).unwrap();
        assert_eq!((manifest.id, manifest.abi), (extreme_protocol::CHOOSE, NO_ENGINE));
        assert!(archive.file("web/index.html").is_some());
        assert_eq!(archive.file("watch.js").map(<[u8]>::len), Some(manifest.watch_len as usize));
    }
}
