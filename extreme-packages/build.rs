//! Builds the preinstalled packages and Choose (tools/package) and writes
//! `packages.rs`, which src/lib.rs includes: every file copied into
//! `OUT_DIR` and `include_bytes!`ed from there (each target has its own
//! `OUT_DIR`, so one build cannot pull files from under another).

use std::{env, fmt::Write as _, fs, path::PathBuf};

use extreme_pack::{Built, CHOOSE, PACKAGES, build, inputs, root};

fn main() {
    let root = root();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", root.join("tools/package/src").display());

    let mut code = String::new();
    writeln!(code, "Packages {{\n    preinstalled: &[").unwrap();
    for spec in PACKAGES {
        for input in inputs(&root, spec) {
            println!("cargo:rerun-if-changed={}", input.display());
        }
        let built = build(&root, spec).unwrap_or_else(|e| panic!("package {}: {e}", spec.name));
        writeln!(code, "        {},", package(&out_dir, &built)).unwrap();
    }
    for input in inputs(&root, &CHOOSE) {
        println!("cargo:rerun-if-changed={}", input.display());
    }
    let choose = build(&root, &CHOOSE).unwrap_or_else(|e| panic!("Choose: {e}"));
    writeln!(code, "    ],\n    choose: {},\n}}", package(&out_dir, &choose)).unwrap();
    fs::write(out_dir.join("packages.rs"), code).unwrap();
}

/// A `Package` expression for `built`, its files copied into `out_dir`.
fn package(out_dir: &std::path::Path, built: &Built) -> String {
    let dir = out_dir.join(built.spec.name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("web")).unwrap();
    let file = |name: &str, contents: &[u8]| {
        let path = dir.join(name);
        fs::write(&path, contents).unwrap();
        format!("include_bytes!({:?})", path.display().to_string())
    };
    let m = &built.manifest;
    let mut web = String::new();
    for (name, contents) in &built.web {
        write!(web, "({name:?}, {}), ", file(&format!("web/{name}"), contents)).unwrap();
    }
    format!(
        "Package {{ id: {}, name: {:?}, version: {}, abi: {}, content_hash: {:?}, ui: UiHashes {{ web: {:?}, watch: {:?} }}, memory: {}, engine: {}, web: &[{web}], watch: {} }}",
        m.id,
        built.spec.name,
        m.version,
        m.abi,
        m.content_hash,
        m.ui.web,
        m.ui.watch,
        built.memory,
        file("engine.wasm", &built.engine),
        file("watch.js", &built.watch),
    )
}
