//! Bare metal only: the libc wasm3 links against. Its
//! malloc/calloc/realloc/free (renamed by build.rs) go to the Rust global
//! allocator (esp-alloc on the C6) with an 8-byte size header, and are
//! counted ([`heap_use`]); the rest are the few string and math functions
//! it calls.

use alloc::alloc::{Layout, alloc, alloc_zeroed, dealloc, realloc as rust_realloc};
use core::ffi::{c_char, c_int, c_void};
use core::ptr::null_mut;
// portable-atomic: thumbv6m (the Pico) has no atomic read-modify-write
use portable_atomic::{AtomicUsize, Ordering::Relaxed};

const HEADER: usize = 8;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static BLOCKS: AtomicUsize = AtomicUsize::new(0);

/// wasm3's heap: (bytes now, peak bytes since boot, blocks now), size
/// headers included (not the allocator's own per-block overhead).
pub fn heap_use() -> (usize, usize, usize) {
    (CURRENT.load(Relaxed), PEAK.load(Relaxed), BLOCKS.load(Relaxed))
}

fn layout(size: usize) -> Option<Layout> {
    Layout::from_size_align(size.checked_add(HEADER)?, HEADER).ok()
}

fn grew(by: usize) {
    let now = CURRENT.fetch_add(by, Relaxed) + by;
    PEAK.fetch_max(now, Relaxed);
}

unsafe fn allocate(size: usize, zeroed: bool) -> *mut c_void {
    let Some(layout) = layout(size) else {
        return null_mut();
    };
    let p = unsafe { if zeroed { alloc_zeroed(layout) } else { alloc(layout) } };
    if p.is_null() {
        return null_mut();
    }
    unsafe { (p as *mut usize).write(size) };
    grew(layout.size());
    BLOCKS.fetch_add(1, Relaxed);
    unsafe { p.add(HEADER) as *mut c_void }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn m3host_malloc(size: usize) -> *mut c_void {
    unsafe { allocate(size, false) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn m3host_calloc(n: usize, size: usize) -> *mut c_void {
    match n.checked_mul(size) {
        Some(total) => unsafe { allocate(total, true) },
        None => null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn m3host_free(p: *mut c_void) {
    if p.is_null() {
        return;
    }
    unsafe {
        let base = (p as *mut u8).sub(HEADER);
        let Some(layout) = layout((base as *const usize).read()) else {
            return;
        };
        CURRENT.fetch_sub(layout.size(), Relaxed);
        BLOCKS.fetch_sub(1, Relaxed);
        dealloc(base, layout);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn m3host_realloc(p: *mut c_void, size: usize) -> *mut c_void {
    if p.is_null() {
        return unsafe { allocate(size, false) };
    }
    if size == 0 {
        unsafe { m3host_free(p) };
        return null_mut();
    }
    unsafe {
        let base = (p as *mut u8).sub(HEADER);
        let Some(old) = layout((base as *const usize).read()) else {
            return null_mut();
        };
        let Some(new) = layout(size) else {
            return null_mut();
        };
        let q = rust_realloc(base, old, new.size());
        if q.is_null() {
            return null_mut();
        }
        (q as *mut usize).write(size);
        CURRENT.fetch_sub(old.size(), Relaxed);
        grew(new.size());
        q.add(HEADER) as *mut c_void
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn m3host_abort() -> ! {
    panic!("wasm3 abort()")
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcmp(a: *const c_char, b: *const c_char) -> c_int {
    let mut i = 0;
    loop {
        let (x, y) = unsafe { (*a.add(i) as u8, *b.add(i) as u8) };
        if x != y || x == 0 {
            return x as c_int - y as c_int;
        }
        i += 1;
    }
}

/// newlib's FP_NAN, FP_INFINITE, FP_ZERO, FP_SUBNORMAL, FP_NORMAL (math.h;
/// clang reports itself as GCC 4.2, so newlib's isnan etc. call these).
fn fp_class(c: core::num::FpCategory) -> c_int {
    use core::num::FpCategory::*;
    match c {
        Nan => 0,
        Infinite => 1,
        Zero => 2,
        Subnormal => 3,
        Normal => 4,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __fpclassifyd(x: f64) -> c_int {
    fp_class(x.classify())
}

#[unsafe(no_mangle)]
pub extern "C" fn __fpclassifyf(x: f32) -> c_int {
    fp_class(x.classify())
}

macro_rules! libm_fns {
    ($($name:ident: $t:ty),* $(,)?) => {
        $(
            #[unsafe(no_mangle)]
            pub extern "C" fn $name(x: $t) -> $t {
                libm::$name(x)
            }
        )*
    };
}

libm_fns!(
    ceil: f64, ceilf: f32, floor: f64, floorf: f32, rint: f64, rintf: f32,
    sqrt: f64, sqrtf: f32, trunc: f64, truncf: f32,
);
