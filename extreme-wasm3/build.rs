//! Compiles the vendored wasm3 interpreter (wasm3/, v0.9.0) with the
//! settings in m3_extremers_config.h, for whatever the crate is built for.
//!
//! clang everywhere (env `WASM3_CC` overrides): wasm3 needs `musttail` for
//! guaranteed tail calls between its op handlers, or every executed op costs
//! a native stack frame (clang has it; gcc only from 15).
//!
//! - Bare metal (`target_os = "none"`, the C6): `-Os`, newlib's headers
//!   (Ubuntu libnewlib-dev, /usr/include/newlib; `WASM3_LIBC_INCLUDE`
//!   overrides), and wasm3's malloc/calloc/realloc/free/abort renamed (-D)
//!   onto src/libc.rs, which also has the few libc functions it calls; the
//!   firmware's own malloc/free (esp-radio's) are left alone. `llvm-ar-18`
//!   (env `WASM3_AR`) archives, since the host's `ar` cannot.
//! - Anywhere else (tgt-std, the tests): `-O2` (always, debug builds
//!   included: the stack budget assumes tail calls), the system's libc.

use std::{env, path::PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = manifest.join("wasm3");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=wasm3");
    println!("cargo:rerun-if-changed=m3_extremers_config.h");
    println!("cargo:rerun-if-env-changed=WASM3_CC");
    println!("cargo:rerun-if-env-changed=WASM3_AR");
    println!("cargo:rerun-if-env-changed=WASM3_LIBC_INCLUDE");

    let bare = env::var("CARGO_CFG_TARGET_OS").unwrap() == "none";

    let mut build = cc::Build::new();
    build
        .compiler(env::var("WASM3_CC").unwrap_or_else(|_| "clang".into()))
        .debug(false)
        .flag("-include")
        .flag(manifest.join("m3_extremers_config.h").to_str().unwrap())
        .flag("-ffunction-sections")
        .flag("-fdata-sections")
        .flag("-Wno-unused")
        .warnings(false);

    if bare {
        let libc = env::var("WASM3_LIBC_INCLUDE").unwrap_or_else(|_| "/usr/include/newlib".into());
        build
            .archiver(env::var("WASM3_AR").unwrap_or_else(|_| "llvm-ar-18".into()))
            // size first; -Os keeps clang's sibling calls (and musttail
            // guarantees them on the dispatch path)
            .opt_level_str("s")
            .flag("-isystem")
            .flag(&libc)
            .define("malloc", "m3host_malloc")
            .define("calloc", "m3host_calloc")
            .define("realloc", "m3host_realloc")
            .define("free", "m3host_free")
            .define("abort", "m3host_abort");
        // Cortex-M0 (the Pico): an op handler's arguments do not all fit
        // in registers (r0-r3), so musttail cannot be honoured and clang 18
        // crashes on it. Plain calls there: slower, and deeper native
        // stack per executed op (bounded by d_m3MaxNativeStack, a trap).
        if env::var("CARGO_CFG_TARGET_ARCH").unwrap() == "arm" {
            build.define("M3_HAS_TAIL_CALL", "0");
        }
    } else {
        build.opt_level(2);
    }

    for f in [
        "m3_bind.c",
        "m3_code.c",
        "m3_compile.c",
        "m3_core.c",
        "m3_env.c",
        "m3_exec.c",
        "m3_function.c",
        "m3_info.c",
        "m3_module.c",
        "m3_parse.c",
        "m3_validate.c",
    ] {
        build.file(src.join(f));
    }
    build.compile("wasm3");
}
