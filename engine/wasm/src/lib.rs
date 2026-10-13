//! The studio's engine toolchain and host, as one WebAssembly module
//! (wasm-bindgen, `--target web`): what `langc`, `engine/profile`,
//! `extreme-pack` and the C6 do, run in the browser.
//!
//! Across the boundary: JSON as strings, bytes as `Uint8Array`. A function
//! that can fail says so in its result (`{ok: false, …}`) rather than
//! throwing; the inputs `bundle::archive::write` would panic on are checked
//! first. (A panic in the module would still be a bug: on wasm32 it aborts,
//! the hook logging it to the console.)
//!
//! Every export is a plain Rust function too: the native tests call them as
//! such. The few that take or give JS values (a list of files, a list of
//! bundles, a list of envelopes) are thin wrappers over a native sibling.

pub mod clock;
pub mod host;
pub mod inspect;
pub mod track;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bundle::Archive;
use compiler::diag::Diag;
use engine_profile::layout::{self, MAP_ENTRY_BYTES};
use extreme_protocol::Manifest;
use serde_json::{Value as Json, json};
use wasm_bindgen::prelude::*;

pub use host::HostHandle;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn error(message: impl Into<String>) -> Json {
    json!({"ok": false, "error": message.into()})
}

/// `text` as JSON; `Err` names `what`, with the line and column.
fn parse(what: &str, text: &str) -> Result<Json, String> {
    serde_json::from_str(text).map_err(|e| format!("{what}: {e}"))
}

fn diagnostics(diags: &[Diag], src: &str) -> Json {
    json!({"ok": false, "diagnostics": diags.iter().map(|d| d.to_json(src)).collect::<Vec<_>>()})
}

/// `{engine_wasm, lang_spec, bytecode_version, source_sha}`: this module's
/// version, the language spec's draft, the bytecode format it emits, and
/// the monorepo commit it was built from (null if not given at build time,
/// `STUDIO_SOURCE_SHA`).
#[wasm_bindgen]
pub fn version() -> String {
    json!({
        "engine_wasm": env!("CARGO_PKG_VERSION"),
        "lang_spec": env!("LANG_SPEC"),
        "bytecode_version": bytecode::VERSION,
        "source_sha": option_env!("STUDIO_SOURCE_SHA"),
    })
    .to_string()
}

/// The sailing builtins' declarations, as the compiler takes them.
#[wasm_bindgen]
pub fn builtins() -> String {
    engine_profile::declarations().to_string()
}

/// An engine's interface from its schemas (JSON text; `view` and `event`
/// optional): `{ok: true, interface}`, `{ok: false, diagnostics}` from the
/// profile, or `{ok: false, error}` for a schema that isn't JSON. (In JS
/// `engine_interface`: `interface` is a reserved word.)
#[wasm_bindgen(js_name = engine_interface)]
pub fn interface(state: &str, view: Option<String>, event: Option<String>) -> String {
    let result = (|| {
        let state = parse("state.schema.json", state)?;
        let view = view.as_deref().map(|v| parse("view.schema.json", v)).transpose()?;
        let event = event.as_deref().map(|e| parse("event.schema.json", e)).transpose()?;
        Ok::<_, String>(match engine_profile::engine_interface(&state, view.as_ref(), event.as_ref()) {
            Ok(interface) => json!({"ok": true, "interface": interface}),
            Err(d) => diagnostics(&d, ""),
        })
    })();
    result.unwrap_or_else(error).to_string()
}

/// A compilation: [`compile`]'s result.
#[wasm_bindgen]
pub struct Compiled {
    ok: bool,
    json: String,
    bytes: Vec<u8>,
}

#[wasm_bindgen]
impl Compiled {
    /// Whether it compiled.
    pub fn ok(&self) -> bool {
        self.ok
    }

    /// `{ok: true, memory, interface_hash, imports, program_bytes}` or
    /// `{ok: false, diagnostics}` (or `{ok: false, error}`).
    pub fn json(&self) -> String {
        self.json.clone()
    }

    /// The program (`engine.bin`); empty if it didn't compile.
    pub fn bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }
}

/// Compiles `source` against `interface` (JSON); `opts` is JSON
/// `{limit_bytes?, code_in_ram?}` or empty. The JSON on success:
/// `{ok: true, memory: {stack_slots, locals, held_slots, output_slots,
/// slot_bytes, program_bytes, total_bytes}, interface_hash (hex), imports (the
/// builtins' signatures), program_bytes}`; diagnostics are
/// `{rule, severity, span: {start, end: {line, col, offset}} | null,
/// message, help}`.
#[wasm_bindgen]
pub fn compile(source: &str, interface: &str, opts: &str) -> Compiled {
    let failed = |json: Json| Compiled { ok: false, json: json.to_string(), bytes: Vec::new() };
    let iface = match parse("interface", interface) {
        Ok(i) => i,
        Err(e) => return failed(error(e)),
    };
    let opts = match options(opts) {
        Ok(o) => o,
        Err(e) => return failed(error(e)),
    };
    match compiler::compile(source, &iface, &opts) {
        Err(d) => failed(diagnostics(&d, source)),
        Ok(c) => {
            let Ok(p) = bytecode::Program::parse(&c.bytes) else {
                return failed(error("the compiler emitted a program that doesn't parse"));
            };
            let imports: Vec<String> = (0..p.import_count).map(|i| String::from_utf8_lossy(p.import(i).signature).into_owned()).collect();
            let json = json!({
                "ok": true,
                "memory": c.memory.to_json(),
                "interface_hash": hex(&p.interface_hash),
                "imports": imports,
                "program_bytes": c.bytes.len(),
            });
            Compiled { ok: true, json: json.to_string(), bytes: c.bytes }
        }
    }
}

fn options(opts: &str) -> Result<compiler::Options, String> {
    if opts.trim().is_empty() {
        return Ok(compiler::Options::default());
    }
    let v = parse("opts", opts)?;
    let o = v.as_object().ok_or("opts: an object {limit_bytes?, code_in_ram?}")?;
    if let Some(k) = o.keys().find(|k| !matches!(k.as_str(), "limit_bytes" | "code_in_ram")) {
        return Err(format!("opts: no option `{k}` (limit_bytes, code_in_ram)"));
    }
    let limit_bytes = match o.get("limit_bytes") {
        None | Some(Json::Null) => None,
        Some(n) => Some(n.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| format!("opts: `limit_bytes` is {n}, not a u32"))?),
    };
    let code_in_ram = match o.get("code_in_ram") {
        None | Some(Json::Null) => false,
        Some(b) => b.as_bool().ok_or_else(|| format!("opts: `code_in_ram` is {b}, not a boolean"))?,
    };
    Ok(compiler::Options { limit_bytes, code_in_ram })
}

/// A program as text, one instruction per line: `{ok: true, text}` or
/// `{ok: false, error}`.
#[wasm_bindgen]
pub fn disassemble(bytes: &[u8]) -> String {
    match compiler::disassemble(bytes) {
        Ok(text) => json!({"ok": true, "text": text}),
        Err(e) => error(e),
    }
    .to_string()
}

/// The sample runs `samples` (JSON, lang/SPEC.md §5.3: `{runs: [{inputs,
/// chain?}]}`) of the program `bytes` compiled against `interface`, on the
/// VM with the sailing builtins: `compiler::samples::run_samples`'s report
/// `{ok, runs: [{run, ok, output, tagged, stack_high, steps, chained_from?}
/// | {run, ok: false, error: "R-…", span: {start, end} | null, help} |
/// {run, ok: false, invalid}]}`, or `{ok: false, error}`.
#[wasm_bindgen]
pub fn run_samples(bytes: &[u8], interface: &str, samples: &str) -> String {
    let result = (|| {
        let iface = parse("interface", interface)?;
        let samples = parse("samples", samples)?;
        // a program for another interface would not fit its inputs and output
        let parsed = compiler::schema::parse_interface(&iface).map_err(|d| format!("interface: {}", d.iter().map(|d| d.message.as_str()).collect::<Vec<_>>().join("; ")))?;
        let p = bytecode::Program::parse(bytes).map_err(|e| format!("the program: {e:?}"))?;
        if p.interface_hash != parsed.hash {
            return Err("the program was compiled against another interface (its interface hash differs)".to_string());
        }
        Ok(compiler::samples::run_samples(bytes, &iface, &samples, &sailing::BUILTINS))
    })();
    result.unwrap_or_else(error).to_string()
}

/// A map's entries, `[{leaf, span, aux}]` (engine_profile::layout).
fn map_entries(map: &[u8]) -> Json {
    map.get(4..)
        .unwrap_or_default()
        .chunks_exact(MAP_ENTRY_BYTES)
        .map(|e| {
            let at = |i: usize| u16::from_le_bytes([e[i], e[i + 1]]);
            json!({"leaf": at(0), "span": at(2), "aux": at(4)})
        })
        .collect()
}

/// `enums.bin`'s strings, in order.
fn enum_strings(enums: &[u8]) -> Json {
    let u16_at = |i: usize| enums.get(i..i + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]])) as usize;
    let u32_at = |i: usize| enums.get(i..i + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) as usize;
    (0..u16_at(0))
        .map(|k| {
            let (off, len) = (u32_at(4 + 8 * k), u16_at(8 + 8 * k));
            Json::String(enums.get(off..off + len).map_or_else(String::new, |s| String::from_utf8_lossy(s).into_owned()))
        })
        .collect()
}

/// The device tables of a program (engine_profile::layout): `interface`
/// (JSON) as it was compiled against, `program` the bytecode, `view_desc`
/// the bitwire descriptor of the published value (the view, or the state
/// without one), `event_desc` the author's events' (if any). `{ok: true,
/// layout: {<field>: u16, …}, gps_list, state_lists: [{list, leaf,
/// max_items, item_leaves}], interface_hash, view_map: [{leaf, span,
/// aux}], event_map: […] | null, enums: [string] | null, bytes: {layout,
/// view_map, event_map, enums} (base64; null where absent)}` or `{ok:
/// false, error}`. `layout` is the fixed part's fields; `gps_list` and
/// `state_lists` its list table's (`gps` list 0 and none without state
/// lists: a version 1 layout, which has no table).
#[wasm_bindgen]
pub fn layout(interface: &str, program: &[u8], view_desc: &[u8], event_desc: Option<Vec<u8>>) -> String {
    let result = (|| {
        let iface = parse("interface", interface)?;
        let t = layout::tables(&iface, program, view_desc, event_desc.as_deref())?;
        let fields: serde_json::Map<String, Json> = layout::layout_fields(&t.layout).into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
        let (gps_list, state_lists) = layout::layout_lists(&t.layout).ok_or("layout.bin's list table is cut short")?;
        let state_lists: Vec<Json> =
            state_lists.iter().map(|l| json!({"list": l.list, "leaf": l.leaf, "max_items": l.max_items, "item_leaves": l.item_leaves})).collect();
        let b64 = |b: &Option<Vec<u8>>| b.as_ref().map(|b| BASE64.encode(b));
        Ok::<_, String>(json!({
            "ok": true,
            "layout": fields,
            "gps_list": gps_list,
            "state_lists": state_lists,
            "interface_hash": hex(&t.layout[layout::HASH_AT..layout::LAYOUT_BYTES]),
            "view_map": map_entries(&t.view_map),
            "event_map": t.event_map.as_deref().map(map_entries),
            "enums": t.enums.as_deref().map(enum_strings),
            "bytes": {
                "layout": BASE64.encode(&t.layout),
                "view_map": BASE64.encode(&t.view_map),
                "event_map": b64(&t.event_map),
                "enums": b64(&t.enums),
            },
        }))
    })();
    result.unwrap_or_else(error).to_string()
}

fn manifest_json(m: &Manifest) -> Json {
    json!({
        "id": m.id,
        "name": m.name.as_str(),
        "version": m.version,
        "abi": m.abi,
        "content_hash": hex(&m.content_hash),
        "ui": {"web": hex(&m.ui.web), "watch": hex(&m.ui.watch)},
        "watch_len": m.watch_len,
    })
}

/// `[{name, bytes: Uint8Array}]` from JS.
fn files_from_js(files: &js_sys::Array) -> Result<Vec<(String, Vec<u8>)>, String> {
    files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let name = js_sys::Reflect::get(&f, &JsValue::from_str("name")).ok().and_then(|n| n.as_string());
            let bytes = js_sys::Reflect::get(&f, &JsValue::from_str("bytes")).ok().and_then(|b| b.dyn_into::<js_sys::Uint8Array>().ok());
            match (name, bytes) {
                (Some(name), Some(bytes)) => Ok((name, bytes.to_vec())),
                _ => Err(format!("files[{i}]: {{name: string, bytes: Uint8Array}}")),
            }
        })
        .collect()
}

/// A bundle: [`pack`]'s result.
#[wasm_bindgen]
pub struct Packed {
    ok: bool,
    json: String,
    bytes: Vec<u8>,
}

#[wasm_bindgen]
impl Packed {
    pub fn ok(&self) -> bool {
        self.ok
    }

    /// `{ok: true, manifest: {id, name, version, abi, content_hash, ui:
    /// {web, watch}, watch_len}, len}` or `{ok: false, error}`.
    pub fn json(&self) -> String {
        self.json.clone()
    }

    /// The bundle; empty if it wasn't made.
    pub fn bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }
}

/// The bundle of a package (`extreme-pack`'s): `manifest` is JSON `{name,
/// version}`, `files` `[{name, bytes}]` by their path in the bundle
/// (`engine.bin`, `layout.bin`, `view.desc`, `view.map`, `event.desc`,
/// `event.map`, `enums.bin`, `source.json`, `web/<name>`, `watch.js`; not
/// `manifest.bin`, which is made here: id, ABI, content and UI hashes).
#[wasm_bindgen]
pub fn pack(manifest: &str, files: js_sys::Array) -> Packed {
    let packed = files_from_js(&files).and_then(|files| pack_files(manifest, files));
    match packed {
        Ok((json, bytes)) => Packed { ok: true, json: json.to_string(), bytes },
        Err(e) => Packed { ok: false, json: error(e).to_string(), bytes: Vec::new() },
    }
}

/// [`pack`], natively.
pub fn pack_files(manifest: &str, mut files: Vec<(String, Vec<u8>)>) -> Result<(Json, Vec<u8>), String> {
    let m = parse("manifest", manifest)?;
    let m = m.as_object().ok_or("manifest: an object {name, version}")?;
    if let Some(k) = m.keys().find(|k| !matches!(k.as_str(), "name" | "version")) {
        return Err(format!("manifest: no field `{k}` (name, version)"));
    }
    let name = m.get("name").and_then(Json::as_str).ok_or("manifest: `name`, a string")?;
    let version = m.get("version").and_then(Json::as_u64).and_then(|v| u16::try_from(v).ok()).ok_or("manifest: `version`, a u16")?;
    let mut program = None;
    let mut web = Vec::new();
    let mut watch = Vec::new();
    for (path, contents) in &files {
        match path.as_str() {
            "manifest.bin" => return Err("manifest.bin: made by pack, not given".into()),
            "engine.bin" => program = Some(contents.as_slice()),
            "watch.js" if contents.is_empty() => return Err("watch.js is empty: leave it out for no watch bundle".into()),
            "watch.js" => watch = contents.clone(),
            p => match p.strip_prefix("web/") {
                Some("") => return Err("web/: a file needs a name".into()),
                Some(w) => web.push((w.to_string(), contents.clone())),
                None => {}
            },
        }
    }
    web.sort();
    let manifest = extreme_pack::manifest(name, version, program, &web, &watch)?;
    files.push(("manifest.bin".into(), postcard::to_stdvec(&manifest).map_err(|e| format!("manifest.bin: {e}"))?));
    let bytes = extreme_pack::archive(&files)?;
    Ok((json!({"ok": true, "manifest": manifest_json(&manifest), "len": bytes.len()}), bytes))
}

/// A bundle's contents: `{ok: true, manifest, files: [{name, len}],
/// source: <source.json's text> | null}` or `{ok: false, error}`.
#[wasm_bindgen]
pub fn unpack(bytes: &[u8]) -> String {
    let result = (|| {
        let archive = Archive::parse(bytes).map_err(|e| format!("not a bundle: {e:?}"))?;
        let manifest: Manifest = archive
            .file("manifest.bin")
            .ok_or("no manifest.bin")
            .and_then(|m| postcard::from_bytes(m).map_err(|_| "manifest.bin doesn't decode"))?;
        let files: Vec<Json> = archive.files().map(|(n, c)| json!({"name": n, "len": c.len()})).collect();
        let source = archive.file("source.json").map(|s| String::from_utf8_lossy(s).into_owned());
        Ok::<_, String>(json!({"ok": true, "manifest": manifest_json(&manifest), "files": files, "source": source}))
    })();
    result.unwrap_or_else(error).to_string()
}

/// The Kindle page's hash (`ui.web`, hex) of `files` `[{name, bytes}]`,
/// named as in `web/` (without the prefix); `""` if `files` isn't that.
#[wasm_bindgen]
pub fn web_hash(files: js_sys::Array) -> String {
    files_from_js(&files).map(web_hash_files).unwrap_or_default()
}

/// [`web_hash`], natively.
pub fn web_hash_files(mut files: Vec<(String, Vec<u8>)>) -> String {
    files.sort();
    hex(&extreme_pack::web_hash(&files))
}

/// The watch bundle's hash (`ui.watch`, hex); all zeros for none.
#[wasm_bindgen]
pub fn watch_hash(bytes: &[u8]) -> String {
    hex(&extreme_pack::watch_hash(bytes))
}

/// A package's id from its name.
#[wasm_bindgen]
pub fn package_id(name: &str) -> u32 {
    extreme_protocol::package_id(name)
}
