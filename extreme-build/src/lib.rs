//! `build.rs` support for engines that embed a web UI.
//!
//! An engine crate keeps its UI in `client-js/`, built by `npm run build`
//! into `client-js/dist/`. [`embed_client_js`] runs that build and writes a
//! `static_files.rs` into `OUT_DIR` that `extreme_traits::static_files!()`
//! includes.

use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

/// Build `client-js/` and embed `client-js/dist/*` as the crate's static files.
///
/// Reruns when the client sources change. `node_modules` is installed on
/// first use. If `npm` is not available, a previously built `dist/` is used
/// with a warning so that the Rust side can still be worked on; with no
/// `dist/` either, the build fails with instructions.
pub fn embed_client_js() {
    let crate_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let client_dir = crate_dir.join("client-js");
    let dist_dir = client_dir.join("dist");

    for input in ["src", "index.html", "package.json", "package-lock.json", "rollup.config.js"] {
        println!("cargo:rerun-if-changed={}", client_dir.join(input).display());
    }

    match build_client(&client_dir) {
        Ok(()) => {}
        Err(NpmError::NotFound) if has_files(&dist_dir) => {
            println!(
                "cargo:warning=npm not found; using the existing {} which may be stale",
                dist_dir.display()
            );
        }
        Err(NpmError::NotFound) => panic!(
            "npm is required to build the web UI in {}. Install Node.js, or run \
             `npm ci && npm run build` there on a machine that has it.",
            client_dir.display()
        ),
        Err(NpmError::Failed(what)) => panic!("`{what}` failed in {}", client_dir.display()),
    }

    write_static_files(&dist_dir);
}

enum NpmError {
    NotFound,
    Failed(&'static str),
}

fn npm(client_dir: &Path, args: &[&str], what: &'static str) -> Result<(), NpmError> {
    let status = Command::new("npm")
        .args(args)
        .current_dir(client_dir)
        .status()
        .map_err(|_| NpmError::NotFound)?;
    if status.success() {
        Ok(())
    } else {
        Err(NpmError::Failed(what))
    }
}

fn build_client(client_dir: &Path) -> Result<(), NpmError> {
    if !client_dir.join("node_modules").is_dir() {
        if client_dir.join("package-lock.json").is_file() {
            npm(client_dir, &["ci", "--no-audit", "--no-fund"], "npm ci")?;
        } else {
            npm(client_dir, &["install", "--no-audit", "--no-fund"], "npm install")?;
        }
    }
    npm(client_dir, &["run", "build"], "npm run build")
}

fn has_files(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|entries| entries.flatten().any(|e| e.path().is_file()))
        .unwrap_or(false)
}

/// Writes `static_files.rs`: a `&'static [(&str, &[u8])]` expression that
/// `include_bytes!`s each file in `dist/`, so the bytes never pass through
/// the generated source.
fn write_static_files(dist_dir: &Path) {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let mut out = fs::File::create(out_dir.join("static_files.rs")).expect("create static_files.rs");

    let mut files: Vec<PathBuf> = fs::read_dir(dist_dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dist_dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();

    writeln!(out, "&[").unwrap();
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy();
        writeln!(
            out,
            "    ({:?}, include_bytes!({:?}).as_slice()),",
            name,
            path.canonicalize().expect("canonicalize dist file")
        )
        .unwrap();
    }
    writeln!(out, "]").unwrap();
}
