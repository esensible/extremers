/* wasm3 build configuration for the extremers package host, force-included
 * (-include) ahead of every wasm3 source by build.rs, on every target (the
 * C6, and the host for tgt-std and the tests). Mirrors wasm3's own embedded
 * defaults (m3_config_platforms.h, ARDUINO/ESP32 branch) where it has one.
 */
#ifndef m3_extremers_config_h
#define m3_extremers_config_h

/* floats on: the engines work in f64 */
#define d_m3HasFloat                1
/* terse errors; the d_m3Log* switches (m3_config.h) all default off, so
 * no printf */
#define d_m3VerboseErrorMessages    0
/* packages are built by tools/package; the host trusts its own registry */
#define d_m3EnableValidation        0
/* embedded defaults: no cascaded opcode table, small constant table */
#define d_m3CascadedOpcodes         0
#define d_m3MaxConstantTableSize    64
/* per-function compile-time slot limit (ESP32 default 256) */
#define d_m3MaxFunctionStackHeight  256
/* code pages are heap blocks of a multiple of this many bytes */
#define d_m3CodePageAlignSize       1024
/* trap instead of overflowing the caller's stack: wasm calls may use at
 * most this much native stack below the host call (the C6's main stack,
 * which the embassy executor and so every engine call runs on, is ~77 KB) */
#define d_m3MaxNativeStack          (24 * 1024)
/* no fixed heap: malloc/calloc/realloc/free go to the Rust global
 * allocator on the firmware (src/libc.rs), the system's elsewhere */
#define d_m3FixedHeap               0

#endif
