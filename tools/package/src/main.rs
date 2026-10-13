//! Builds every package and writes it out (README "Packages"):
//!
//!     cargo run -p extreme-pack                          # into target/packages/
//!     cargo run -p extreme-pack -- --out DIR
//!     cargo run -p extreme-pack -- --bundle              # also <name>.bundle
//!     cargo run -p extreme-pack -- --store-image PATH    # also the bundles region
//!     cargo run -p extreme-pack -- --store-image PATH --from a.bundle --from b.bundle
//!                                                        # the region from bundle files only
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
//! With `--from FILE` (repeatable) `--store-image` builds nothing: the image
//! holds exactly those bundle files (e.g. one exported from the studio), in
//! the order given, as seq 1 to n. Each must be an app bundle whose manifest
//! matches its files (`extreme_pack::bundle_manifest`); one line per bundle
//! (name, id, version, length) and the space left are printed. For
//! installing without BLE.
//!
//! `--check` builds Choose only and fails if its bundle differs from the
//! committed one.

use std::{fs, path::Path, path::PathBuf, process::ExitCode};

use extreme_pack::{Built, CHOOSE, CHOOSE_BUNDLE, PACKAGES, STORE_SIZE, Spec, build, bundle_manifest, root, store_image, store_used};

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

/// `--store-image image --from FILE…`: the image of exactly `files`, in order.
fn image_from(image: &Path, files: &[PathBuf]) -> ExitCode {
    let mut bundles = Vec::new();
    for path in files {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        };
        let manifest = match bundle_manifest(&bytes) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("{}: refused: {e}", path.display());
                return ExitCode::FAILURE;
            }
        };
        println!(
            "{:16} id {:08x} v{} {} B  {}",
            manifest.name.as_str(),
            manifest.id,
            manifest.version,
            bytes.len(),
            path.display()
        );
        bundles.push(bytes);
    }
    let used = store_used(&bundles);
    if used > STORE_SIZE {
        eprintln!("the bundles need {used} B of the region's {STORE_SIZE} B");
        return ExitCode::FAILURE;
    }
    if let Err(e) = fs::write(image, store_image(&bundles)) {
        eprintln!("writing {}: {e}", image.display());
        return ExitCode::FAILURE;
    }
    println!(
        "wrote {} ({} bundles, {used} B used, {} B free of {STORE_SIZE})",
        image.display(),
        bundles.len(),
        STORE_SIZE - used
    );
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(PathBuf::from);
    if args.iter().any(|a| a == "--check") {
        return check();
    }
    let from: Vec<PathBuf> = args.windows(2).filter(|w| w[0] == "--from").map(|w| PathBuf::from(&w[1])).collect();
    if args.iter().any(|a| a == "--from") {
        return match value("--store-image") {
            Some(image) if args.last().map(String::as_str) != Some("--from") => image_from(&image, &from),
            Some(_) => {
                eprintln!("--from needs a bundle file");
                ExitCode::FAILURE
            }
            None => {
                eprintln!("--from goes with --store-image PATH");
                ExitCode::FAILURE
            }
        };
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
