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

    // Only inputs that exist: cargo treats a missing rerun-if-changed path as
    // changed, which would rerun this script on every build. index.html is
    // generated into dist/ by the rollup html plugin, so it is not an input.
    for input in [
        "src",
        "package.json",
        "package-lock.json",
        "rollup.config.js",
    ] {
        println!(
            "cargo:rerun-if-changed={}",
            client_dir.join(input).display()
        );
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

    check_templates_declared(&dist_dir);
    write_static_files(&dist_dir);
}

/// Fails the build if a bundle calls a Solid template that was never
/// declared. The minifier renames every declared `_tmpl$…` to a short name,
/// so the literal `_tmpl$` surviving in a bundle means a call with no
/// declaration: at runtime that is "ReferenceError: _tmpl$ is not defined"
/// and a blank page (seen in the tune client when babel-preset-solid and
/// preset-env ran in one pass).
fn check_templates_declared(dist_dir: &Path) {
    let Ok(entries) = fs::read_dir(dist_dir) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        if path.extension().is_some_and(|e| e == "js") {
            let js = fs::read_to_string(&path).unwrap_or_default();
            if js.contains("_tmpl$") {
                panic!(
                    "{} calls a Solid template (_tmpl$) that is never declared: the page would fail \
                     to render. Check that babel-preset-solid runs in its own babel pass.",
                    path.display()
                );
            }
        }
    }
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
            npm(
                client_dir,
                &["install", "--no-audit", "--no-fund"],
                "npm install",
            )?;
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
/// `include_bytes!`s each file, so the bytes never pass through generated
/// source.
///
/// The files are copied into `OUT_DIR` first. `npm run build` empties
/// `dist/` and uses hashed bundle names, and every target (host tests, each
/// firmware) has its own `OUT_DIR` and runs this script separately, so
/// pointing `include_bytes!` at `dist/` itself would break whichever target
/// built earlier.
fn write_static_files(dist_dir: &Path) {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let static_dir = out_dir.join("static");
    let _ = fs::remove_dir_all(&static_dir);
    fs::create_dir_all(&static_dir).expect("create OUT_DIR/static");

    let mut files: Vec<PathBuf> = fs::read_dir(dist_dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dist_dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();

    let mut out =
        fs::File::create(out_dir.join("static_files.rs")).expect("create static_files.rs");
    writeln!(out, "&[").unwrap();
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy();
        let copy = static_dir.join(&*name);
        fs::copy(&path, &copy).unwrap_or_else(|e| panic!("copy {}: {e}", path.display()));
        writeln!(
            out,
            "    ({:?}, include_bytes!({:?}).as_slice()),",
            name, copy
        )
        .unwrap();
    }
    writeln!(out, "]").unwrap();
}
