//! Builds every package and writes it out (README "Packages"):
//!
//!     cargo run -p extreme-pack                          # into target/packages/
//!     cargo run -p extreme-pack -- --out DIR
//!     cargo run -p extreme-pack -- --bundle              # also <name>.bundle
//!     cargo run -p extreme-pack -- --store-image PATH    # also the bundles region
//!     cargo run -p extreme-pack -- --check               # choose/choose.bundle is current
//!
//! Each package goes to `<out>/<name>/`: its files (`extreme_pack::Built::files`),
//! web files under `web/`. `--bundle` writes each as one archive,
//! `<out>/<name>.bundle`, and Choose's to `choose/choose.bundle` too (committed:
//! `extreme-packages` embeds it). `--store-image` writes an image of the C6's
//! bundles region (512 KiB) holding the app packages (not Choose, which the
//! firmware embeds), flashed at the start of the inactive app slot
//! (`ota_1` at 0x200000 while `ota_0` runs; `tgt-xiaoc6/README.md`):
//!
//!     probe-rs download --chip esp32c6 --binary-format bin --base-address 0x200000 PATH
//!
//! `--check` builds Choose only and fails if its bundle differs from the
//! committed one.

use std::{fs, path::Path, path::PathBuf, process::ExitCode};

use extreme_pack::{Built, CHOOSE, CHOOSE_BUNDLE, PACKAGES, Spec, build, root, store_image};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn write(out: &Path, built: &Built) -> std::io::Result<()> {
    let dir = out.join(built.spec.name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("web"))?;
    for (name, contents) in built.files() {
        fs::write(dir.join(name), contents)?;
    }
    Ok(())
}

fn built(spec: &Spec) -> Result<Built, ExitCode> {
    build(&root(), spec).map_err(|e| {
        eprintln!("{}: {e}", spec.name);
        ExitCode::FAILURE
    })
}

fn check() -> ExitCode {
    let choose = match built(&CHOOSE) {
        Ok(b) => b,
        Err(code) => return code,
    };
    let committed = root().join(CHOOSE_BUNDLE);
    if fs::read(&committed).ok() == Some(choose.bundle()) {
        return ExitCode::SUCCESS;
    }
    eprintln!("stale: {CHOOSE_BUNDLE}: run `cargo run -p extreme-pack -- --bundle`");
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(PathBuf::from);
    if args.iter().any(|a| a == "--check") {
        return check();
    }
    let out = value("--out").unwrap_or_else(|| root().join("target/packages"));
    let bundles = args.iter().any(|a| a == "--bundle");
    let image = value("--store-image");
    let mut apps = Vec::new();
    for spec in PACKAGES.iter().chain([&CHOOSE]) {
        let built = match built(spec) {
            Ok(b) => b,
            Err(code) => return code,
        };
        let failed = |what: &dyn std::fmt::Display, e: std::io::Error| {
            eprintln!("{}: writing {what}: {e}", spec.name);
            ExitCode::FAILURE
        };
        if let Err(e) = write(&out, &built) {
            return failed(&out.display(), e);
        }
        if bundles || image.is_some() {
            let bundle = built.bundle();
            if bundles {
                let path = out.join(format!("{}.bundle", spec.name));
                if let Err(e) = fs::write(&path, &bundle) {
                    return failed(&path.display(), e);
                }
                if spec.engine.is_none() {
                    let path = root().join(CHOOSE_BUNDLE);
                    if let Err(e) = fs::write(&path, &bundle) {
                        return failed(&path.display(), e);
                    }
                }
            }
            if spec.engine.is_some() {
                apps.push(bundle);
            }
        }
        let web: usize = built.web.iter().map(|(_, c)| c.len()).sum();
        let engine = match &built.engine {
            Some(e) => format!(
                "engine.bin {} B (stack {}, locals {}, output {} slots)",
                e.program.len(),
                e.memory.stack_slots,
                e.memory.locals,
                e.memory.output_slots
            ),
            None => "no engine".into(),
        };
        println!(
            "{:8} id {:08x} v{} abi {} hash {}  {engine}, web {} B in {} files (ui.web {}), watch.js {} B (ui.watch {})",
            built.spec.name,
            built.manifest.id,
            built.manifest.version,
            built.manifest.abi,
            hex(&built.manifest.content_hash),
            web,
            built.web.len(),
            hex(&built.manifest.ui.web),
            built.watch.len(),
            hex(&built.manifest.ui.watch)
        );
    }
    if let Some(path) = image {
        if let Err(e) = fs::write(&path, store_image(&apps)) {
            eprintln!("writing {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        println!("wrote {} ({} bundles)", path.display(), apps.len());
    }
    println!("wrote {}", out.display());
    ExitCode::SUCCESS
}
