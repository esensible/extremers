//! The wasm3 interpreter (v0.9.0, the `wasm3/` submodule, built by build.rs) and
//! the few parts of its API the package host uses (`common::engine`).
//!
//! A [`Wasm3`] is one wasm3 environment and runtime holding at most one
//! module, parsed in place from `'static` bytes (on the C6 they stay in
//! flash; wasm3 keeps pointers into them) and compiled lazily, a function on
//! its first call. Dropping it frees everything wasm3 allocated.
//!
//! On bare metal wasm3's heap goes to the Rust global allocator and is
//! counted ([`heap_use`]); see src/libc.rs.

#![no_std]

#[cfg(target_os = "none")]
extern crate alloc;

#[cfg(target_os = "none")]
mod libc;
#[cfg(target_os = "none")]
pub use libc::heap_use;

use core::{
    ffi::{CStr, c_char, c_void},
    ptr::{null, null_mut},
};

/// NULL on success, else a static C string.
pub type M3Result = *const c_char;
pub type IM3Environment = *mut c_void;
pub type IM3Runtime = *mut c_void;
pub type IM3Module = *mut c_void;
pub type IM3Function = *mut c_void;
pub type IM3ImportContext = *mut c_void;

/// A host function as wasm3 calls it: `sp` holds the results first (one
/// 64-bit slot each), then the arguments; `mem` is the linear memory. Return
/// [`OK`] or a static C string (a trap).
pub type RawCall =
    unsafe extern "C" fn(IM3Runtime, IM3ImportContext, *mut u64, *mut c_void) -> *const c_void;

/// A raw call's "no error".
pub const OK: *const c_void = null();

unsafe extern "C" {
    fn m3_NewEnvironment() -> IM3Environment;
    fn m3_FreeEnvironment(env: IM3Environment);
    fn m3_NewRuntime(env: IM3Environment, stack_bytes: u32, userdata: *mut c_void) -> IM3Runtime;
    fn m3_FreeRuntime(runtime: IM3Runtime);
    fn m3_ParseModule(env: IM3Environment, module: *mut IM3Module, bytes: *const u8, len: u32) -> M3Result;
    fn m3_FreeModule(module: IM3Module);
    fn m3_LoadModule(runtime: IM3Runtime, module: IM3Module) -> M3Result;
    fn m3_LinkRawFunction(
        module: IM3Module,
        module_name: *const c_char,
        function_name: *const c_char,
        signature: *const c_char,
        function: RawCall,
    ) -> M3Result;
    fn m3_FindFunction(function: *mut IM3Function, runtime: IM3Runtime, name: *const c_char) -> M3Result;
    fn m3_Call(function: IM3Function, argc: u32, argptrs: *const *const c_void) -> M3Result;
    fn m3_GetResults(function: IM3Function, retc: u32, retptrs: *const *const c_void) -> M3Result;
    fn m3_GetRetCount(function: IM3Function) -> u32;
    fn m3_GetMemory(runtime: IM3Runtime, size: *mut u32, index: u32) -> *mut u8;
    fn m3_GetUserData(runtime: IM3Runtime) -> *mut c_void;

    static m3Err_functionLookupFailed: M3Result;
    /// "[trap] program called abort"
    pub static m3Err_trapAbort: M3Result;
}

/// What went wrong: wasm3's message (terse; verbose messages are off).
pub type Error = &'static str;

/// `Ok` for NULL, else wasm3's message.
fn check(r: M3Result) -> Result<(), Error> {
    if r.is_null() {
        Ok(())
    } else {
        Err(unsafe { CStr::from_ptr(r) }.to_str().unwrap_or("<wasm3 error>"))
    }
}

/// An exported function, valid as long as the [`Wasm3`] it came from.
#[derive(Clone, Copy)]
pub struct Function(IM3Function);

/// One wasm3 environment and runtime with one module loaded.
pub struct Wasm3 {
    env: IM3Environment,
    runtime: IM3Runtime,
    module: IM3Module,
}

impl Wasm3 {
    /// Parses `wasm` in place (it must outlive the result, hence `'static`)
    /// and loads it into a new runtime with a value stack of `stack_bytes`
    /// and `userdata` for the host functions ([`userdata`]).
    ///
    /// # Safety
    /// `userdata` is handed to the host functions as it is: it must stay
    /// valid, for whatever they read through it, as long as the result.
    pub unsafe fn new(wasm: &'static [u8], stack_bytes: u32, userdata: *mut c_void) -> Result<Self, Error> {
        let env = unsafe { m3_NewEnvironment() };
        if env.is_null() {
            return Err("m3_NewEnvironment: out of memory");
        }
        let runtime = unsafe { m3_NewRuntime(env, stack_bytes, userdata) };
        if runtime.is_null() {
            unsafe { m3_FreeEnvironment(env) };
            return Err("m3_NewRuntime: out of memory");
        }
        // from here on, dropping `this` frees the runtime and environment
        let mut this = Wasm3 {
            env,
            runtime,
            module: null_mut(),
        };
        let mut module: IM3Module = null_mut();
        check(unsafe { m3_ParseModule(env, &mut module, wasm.as_ptr(), wasm.len() as u32) })?;
        if let Err(e) = check(unsafe { m3_LoadModule(runtime, module) }) {
            // not taken over by the runtime
            unsafe { m3_FreeModule(module) };
            return Err(e);
        }
        this.module = module;
        Ok(this)
    }

    /// Provides the import `module_name.name` with `signature` (wasm3's
    /// notation, e.g. `"I()"`). `Ok(false)` if the module does not import it.
    pub fn link(&mut self, module_name: &CStr, name: &CStr, signature: &CStr, f: RawCall) -> Result<bool, Error> {
        let r = unsafe {
            m3_LinkRawFunction(self.module, module_name.as_ptr(), name.as_ptr(), signature.as_ptr(), f)
        };
        if r == unsafe { m3Err_functionLookupFailed } {
            return Ok(false);
        }
        check(r).map(|()| true)
    }

    /// The exported function `name` (compiled now; its callees on first call).
    pub fn function(&self, name: &CStr) -> Result<Function, Error> {
        let mut f: IM3Function = null_mut();
        check(unsafe { m3_FindFunction(&mut f, self.runtime, name.as_ptr()) })?;
        Ok(Function(f))
    }

    /// Calls `f` with `args`, pointers to each argument's value (`u32`/`i32`
    /// for i32, `u64`/`i64` for i64, ...), and returns its result if it has
    /// one (`T` the result's type, `()` for none). `Err` on a trap.
    ///
    /// # Safety
    /// `args` must match `f`'s parameters in number and type, and `T` its
    /// result.
    pub unsafe fn call<T: Copy + Default>(&mut self, f: Function, args: &[*const c_void]) -> Result<T, Error> {
        check(unsafe { m3_Call(f.0, args.len() as u32, args.as_ptr()) })?;
        let mut value = T::default();
        if unsafe { m3_GetRetCount(f.0) } > 0 {
            let ptrs = [&mut value as *mut T as *const c_void];
            check(unsafe { m3_GetResults(f.0, 1, ptrs.as_ptr()) })?;
        }
        Ok(value)
    }

    /// The module's linear memory.
    pub fn memory(&mut self) -> &mut [u8] {
        // a live runtime of ours
        unsafe { memory(self.runtime) }
    }
}

impl Drop for Wasm3 {
    fn drop(&mut self) {
        unsafe {
            // frees the loaded module too
            m3_FreeRuntime(self.runtime);
            m3_FreeEnvironment(self.env);
        }
    }
}

/// The `userdata` a runtime was made with, for a host function.
///
/// # Safety
/// `runtime` must be a live runtime (the one a host function is called with).
pub unsafe fn userdata(runtime: IM3Runtime) -> *mut c_void {
    unsafe { m3_GetUserData(runtime) }
}

/// A runtime's linear memory, for a host function (empty if it has none).
///
/// # Safety
/// `runtime` must be a live runtime, and the slice must not outlive a
/// call into it (which may move the memory).
pub unsafe fn memory<'a>(runtime: IM3Runtime) -> &'a mut [u8] {
    let mut size = 0u32;
    let base = unsafe { m3_GetMemory(runtime, &mut size, 0) };
    if base.is_null() {
        return &mut [];
    }
    unsafe { core::slice::from_raw_parts_mut(base, size as usize) }
}
