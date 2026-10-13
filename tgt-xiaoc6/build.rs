//! Feature `dev-race` embeds the bundle at `$EXTREMERS_DEV_BUNDLE`
//! (src/main.rs): rebuild when the variable or the file changes.

fn main() {
    println!("cargo:rerun-if-env-changed=EXTREMERS_DEV_BUNDLE");
    if let Ok(path) = std::env::var("EXTREMERS_DEV_BUNDLE") {
        println!("cargo:rerun-if-changed={path}");
    }
}
