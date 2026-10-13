//! Every export, called as plain Rust, on Race: engine/race's sources, and
//! `target/packages/race.bundle` with its inputs `target/packages/race/`
//! (`cargo run -p extreme-pack -- --out target/packages --bundle` at the
//! root writes both).
//!
//! The golden files the studio's node tests hold the wasm build to are
//! written here: `fixtures/race.samples.expected.json` (Race's samples run)
//! and `fixtures/race.layout.json` (Race's tables). `UPDATE=1 cargo test`
//! rewrites them; otherwise a stale one fails.

use std::{fs, path::Path};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use engine_wasm::*;
use extreme_protocol::Manifest;
use serde_json::{Value as Json, json};

const RACE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../race");
const PACKAGES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/packages");
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn race_file(name: &str) -> String {
    read(&format!("{RACE}/{name}"))
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

fn race_interface() -> String {
    let r = json(&interface(&race_file("state.schema.json"), Some(race_file("view.schema.json")), Some(race_file("event.schema.json"))));
    assert_eq!(r["ok"], true, "{r:#}");
    r["interface"].to_string()
}

fn race_program() -> Vec<u8> {
    let c = compile(&race_file("engine.expr"), &race_interface(), "");
    assert!(c.ok(), "{}", c.json());
    c.bytes()
}

fn race_bundle() -> Vec<u8> {
    let path = format!("{PACKAGES}/race.bundle");
    fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}: build it with `cargo run -p extreme-pack -- --out target/packages --bundle`"))
}

fn bundle_file(name: &str) -> Vec<u8> {
    let bytes = race_bundle();
    let archive = bundle::Archive::parse(&bytes).unwrap();
    archive.file(name).unwrap_or_else(|| panic!("race.bundle has no {name}")).to_vec()
}

/// `want` is `fixtures/<name>`'s JSON; `UPDATE=1` writes it.
fn golden(name: &str, want: &Json) {
    let path = format!("{FIXTURES}/{name}");
    let text = serde_json::to_string_pretty(want).unwrap() + "\n";
    if std::env::var_os("UPDATE").is_some() {
        fs::create_dir_all(FIXTURES).unwrap();
        fs::write(&path, &text).unwrap();
    }
    let got = fs::read_to_string(&path).unwrap_or_default();
    assert!(got == text, "fixtures/{name} is stale: run `UPDATE=1 cargo test`");
}

#[test]
fn version_names_the_spec_and_the_bytecode() {
    let v = json(&version());
    assert_eq!(v["engine_wasm"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["lang_spec"], "draft 4");
    assert_eq!(v["bytecode_version"], 1);
    assert!(v["source_sha"].is_null() || v["source_sha"].is_string(), "{v}");
}

#[test]
fn builtins_are_the_profiles() {
    assert_eq!(json(&builtins()), engine_profile::declarations());
}

/// The interface the studio makes from Race's schemas is the committed one.
#[test]
fn race_interface_is_interface_json() {
    assert_eq!(json(&race_interface()), json(&race_file("interface.json")));
}

#[test]
fn interface_errors() {
    // not JSON: which file, where
    let r = json(&interface("{", None, None));
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().unwrap().starts_with("state.schema.json: EOF while parsing"), "{r}");
    // the profile's diagnostics
    let r = json(&interface(r#"{"type": "object"}"#, None, Some(r#"{"type": "object"}"#.into())));
    assert_eq!(r["ok"], false);
    assert_eq!(r["diagnostics"][0]["rule"], "E-SCHEMA", "{r}");
    assert_eq!(r["diagnostics"][0]["span"], Json::Null, "{r}");
}

/// Race compiles to the very program its bundle carries.
#[test]
fn race_compiles_to_its_bundles_program() {
    let c = compile(&race_file("engine.expr"), &race_interface(), "{}");
    assert!(c.ok());
    assert_eq!(c.bytes(), bundle_file("engine.bin"));
    let j = json(&c.json());
    assert_eq!(j["ok"], true);
    assert_eq!(j["memory"]["stack_slots"], 30, "{j}");
    assert_eq!(j["program_bytes"], c.bytes().len());
    assert_eq!(j["interface_hash"].as_str().unwrap().len(), 16);
    let imports: Vec<&str> = j["imports"].as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect();
    assert!(!imports.is_empty() && imports.iter().all(|s| s.contains('(')), "{imports:?}");
}

#[test]
fn compile_errors_have_spans() {
    let source = "{state: state,\n view: nope}";
    let c = compile(source, &race_interface(), "");
    assert!(!c.ok());
    assert!(c.bytes().is_empty());
    let j = json(&c.json());
    let d = &j["diagnostics"][0];
    assert!(d["rule"].as_str().unwrap().starts_with("E-"), "{j}");
    assert_eq!(d["severity"], "error");
    assert!(d["span"]["start"]["line"].is_u64() && d["span"]["start"]["offset"].is_u64(), "{d}");
    // options: the limit, and nonsense
    let j = json(&compile(&race_file("engine.expr"), &race_interface(), r#"{"limit_bytes": 64}"#).json());
    assert_eq!(j["diagnostics"][0]["rule"], "E-LIMIT", "{j}");
    let j = json(&compile(&race_file("engine.expr"), &race_interface(), r#"{"fast": true}"#).json());
    assert_eq!(j["error"], "opts: no option `fast` (limit_bytes, code_in_ram)");
    let j = json(&compile("1", "[", "").json());
    assert_eq!(j["ok"], false);
    assert!(j["error"].as_str().unwrap().starts_with("interface: "), "{j}");
}

#[test]
fn disassembly() {
    let d = json(&disassemble(&race_program()));
    assert_eq!(d["ok"], true);
    assert!(d["text"].as_str().unwrap().starts_with("max_stack 30 "), "{d}");
    assert_eq!(json(&disassemble(b"nope"))["ok"], false);
}

/// Race's samples, run as the studio runs them: the golden report.
#[test]
fn race_samples() {
    let report = json(&run_samples(&race_program(), &race_interface(), &race_file("samples.json")));
    assert_eq!(report["ok"], true, "{report:#}");
    // the same as compiler::samples with the sailing builtins
    let direct = compiler::samples::run_samples(&race_program(), &json(&race_interface()), &json(&race_file("samples.json")), &sailing::BUILTINS);
    assert_eq!(report, direct);
    golden("race.samples.expected.json", &report);
}

#[test]
fn samples_refuse_a_program_for_another_interface() {
    let state = r#"{"type": "object", "properties": {"n": {"type": "integer"}}}"#;
    let other = json(&interface(state, None, None))["interface"].to_string();
    let r = json(&run_samples(&race_program(), &other, r#"{"runs": []}"#));
    assert_eq!(r["error"], "the program was compiled against another interface (its interface hash differs)");
    let r = json(&run_samples(&race_program(), &race_interface(), "{}"));
    assert_eq!((r["ok"].clone(), r["invalid"].clone()), (json!(false), json!("samples need a `runs` list")));
}

/// Race's tables, as the studio makes them: the bundle's bytes, and the
/// golden JSON.
#[test]
fn race_layout() {
    let view = fs::read(format!("{RACE}/view.desc")).unwrap();
    let event = fs::read(format!("{RACE}/event.desc")).unwrap();
    let l = json(&layout(&race_interface(), &race_program(), &view, Some(event)));
    assert_eq!(l["ok"], true, "{l:#}");
    let b = |k: &str| BASE64.decode(l["bytes"][k].as_str().unwrap()).unwrap();
    assert_eq!(b("layout"), bundle_file("layout.bin"));
    assert_eq!(b("view_map"), bundle_file("view.map"));
    assert_eq!(b("event_map"), bundle_file("event.map"));
    assert_eq!(l["bytes"]["enums"], Json::Null, "Race has no enums");
    assert_eq!(l["layout"]["version"], 1);
    assert_eq!(l["layout"]["gps_max_items"], engine_profile::GPS_MAX_ITEMS);
    assert_eq!(l["view_map"].as_array().unwrap().len() as u32, u32::from_le_bytes(b("view_map")[..4].try_into().unwrap()));
    golden("race.layout.json", &l);
    // the program against the wrong descriptor
    let r = json(&layout(&race_interface(), &race_program(), b"nope", None));
    assert_eq!(r["ok"], false);
}

/// `target/packages/race/` (what `--out` writes), as pack takes it.
fn race_inputs() -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) {
        for e in fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            let name = format!("{prefix}{}", path.file_name().unwrap().to_str().unwrap());
            if path.is_dir() {
                walk(&path, &format!("{name}/"), out);
            } else if name != "manifest.bin" {
                out.push((name, fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(Path::new(&format!("{PACKAGES}/race")), "", &mut out);
    out
}

/// What the studio packs is extreme-pack's bundle, byte for byte.
#[test]
fn pack_is_extreme_packs_bundle() {
    let (j, bytes) = pack_files(r#"{"name": "race", "version": 1}"#, race_inputs()).unwrap();
    assert!(bytes == race_bundle(), "pack(race inputs) != target/packages/race.bundle");
    assert_eq!(j["len"], bytes.len());
    let m: Manifest = postcard::from_bytes(&bundle_file("manifest.bin")).unwrap();
    assert_eq!(j["manifest"]["id"], m.id);
    assert_eq!(j["manifest"]["id"], package_id("race"));
    assert_eq!(j["manifest"]["abi"], 2);
    // and unpack reads it back
    let u = json(&unpack(&bytes));
    assert_eq!(u["ok"], true);
    assert_eq!(u["manifest"], j["manifest"]);
    let names: Vec<&str> = u["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"manifest.bin") && names.contains(&"web/index.html") && names.contains(&"watch.js"), "{names:?}");
    assert_eq!(json(u["source"].as_str().unwrap())["engine"], race_file("engine.expr"));
    // the UI hashes on their own
    let web: Vec<(String, Vec<u8>)> = race_inputs().into_iter().filter_map(|(n, c)| n.strip_prefix("web/").map(|w| (w.to_string(), c))).collect();
    assert_eq!(web_hash_files(web.into_iter().rev().collect()), j["manifest"]["ui"]["web"]);
    assert_eq!(watch_hash(&bundle_file("watch.js")), j["manifest"]["ui"]["watch"]);
    assert_eq!(watch_hash(b""), "0000000000000000");
}

#[test]
fn pack_errors() {
    let race = r#"{"name": "race", "version": 1}"#;
    let with = |extra: (&str, &[u8])| {
        let mut files = race_inputs();
        files.push((extra.0.to_string(), extra.1.to_vec()));
        pack_files(race, files).unwrap_err()
    };
    assert_eq!(with(("engine.bin", b"x")), "engine.bin: a file name given twice");
    assert_eq!(with(("manifest.bin", b"x")), "manifest.bin: made by pack, not given");
    assert_eq!(with(("web/", b"x")), "web/: a file needs a name");
    assert_eq!(pack_files(r#"{"name": "a name of seventeen", "version": 1}"#, vec![]).unwrap_err(), "a name of seventeen: name longer than 16 bytes");
    assert_eq!(pack_files(r#"{"name": "race"}"#, vec![]).unwrap_err(), "manifest: `version`, a u16");
    assert!(json(&unpack(b"XBND"))["error"].as_str().unwrap().starts_with("not a bundle"));
}
