//! Builds every package and writes it out (README "Packages"):
//!
//!     cargo run -p extreme-pack                 # into target/packages/
//!     cargo run -p extreme-pack -- --out DIR
//!
//! Each package goes to `<out>/<name>/`: `manifest.bin`, `engine.wasm` (not
//! for Choose), `web/`, `watch.js` (if it has one) and `schema.json` (its
//! view's and events' schemas from tools/bindgen; the device never sees
//! them).

use std::{fs, path::PathBuf, process::ExitCode};

use extreme_pack::{Built, CHOOSE, PACKAGES, build, root};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn write(out: &PathBuf, built: &Built) -> std::io::Result<()> {
    let dir = out.join(built.spec.name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("web"))?;
    fs::write(dir.join("manifest.bin"), built.manifest_bytes())?;
    if !built.engine.is_empty() {
        fs::write(dir.join("engine.wasm"), &built.engine)?;
    }
    for (name, contents) in &built.web {
        fs::write(dir.join("web").join(name), contents)?;
    }
    if !built.watch.is_empty() {
        fs::write(dir.join("watch.js"), &built.watch)?;
    }
    // Choose's view is the host protocol's ChooseView
    let schema = match built.spec.engine {
        Some(_) => root().join(built.spec.dir).join("protocol-js/schema.json"),
        None => root().join("protocol-js/schema.json"),
    };
    if schema.is_file() {
        fs::copy(schema, dir.join("schema.json"))?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let out = match args.iter().position(|a| a == "--out") {
        Some(i) => PathBuf::from(&args[i + 1]),
        None => root().join("target/packages"),
    };
    let root = root();
    for spec in PACKAGES.iter().chain([&CHOOSE]) {
        let built = match build(&root, spec) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{}: {e}", spec.name);
                return ExitCode::FAILURE;
            }
        };
        if let Err(e) = write(&out, &built) {
            eprintln!("{}: writing {}: {e}", spec.name, out.display());
            return ExitCode::FAILURE;
        }
        let web: usize = built.web.iter().map(|(_, c)| c.len()).sum();
        println!(
            "{:8} id {:08x} v{} hash {}  engine.wasm {} B (memory {} B), web {} B in {} files (ui.web {}), watch.js {} B (ui.watch {})",
            built.spec.name,
            built.manifest.id,
            built.manifest.version,
            hex(&built.manifest.content_hash),
            built.engine.len(),
            built.memory,
            web,
            built.web.len(),
            hex(&built.manifest.ui.web),
            built.watch.len(),
            hex(&built.manifest.ui.watch)
        );
    }
    println!("wrote {}", out.display());
    ExitCode::SUCCESS
}
