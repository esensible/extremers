//! A package's engine running under wasm3 (`extreme-wasm3`), spoken to
//! through engine ABI v1 (`extreme_sdk::abi`).
//!
//! The module is parsed in place from the package's `'static` engine.wasm
//! (flash on the C6) and compiled lazily, a function on its first call; its
//! linear memory is exactly what the package declared. Dropping a
//! [`WasmEngine`] frees all of it.
//!
//! The engine's `now()` import reads the time the host hands each call, so
//! one call sees one time.

extern crate alloc;

use alloc::boxed::Box;
use core::{
    cell::Cell,
    ffi::{CStr, c_void},
};

use extreme_protocol::Package;
use extreme_sdk::{
    Location,
    abi::{self, names},
};
use extreme_wasm3::{Function, IM3ImportContext, IM3Runtime, OK, Wasm3};

/// wasm3's value stack per engine, bytes (allocated on the heap). The
/// engines' calls are shallow.
const VALUE_STACK: u32 = 4096;

/// Longest line an engine may log.
const LOG_MAX: usize = 120;

/// What went wrong: wasm3's message, or the host's.
pub type Error = &'static str;

/// What an `on_*` call said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Changed,
    Unchanged,
    /// The input was not one value of its type; nothing changed.
    Refused,
}

/// The host functions' view of the engine: `userdata` of its runtime.
struct Ctx {
    /// What `now()` returns during the current call.
    now: Cell<u64>,
    name: &'static str,
}

/// A running engine. See the module docs.
pub struct WasmEngine {
    // dropped before `ctx`, which its host functions point at
    wasm: Wasm3,
    ctx: Box<Ctx>,
    io: usize,
    io_cap: usize,
    on_location: Function,
    on_timer: Function,
    on_event: Function,
    view: Function,
    next_timer: Function,
}

// The runtime is only ever used by one task at a time (behind the host's
// mutex); nothing in it is tied to a thread.
unsafe impl Send for WasmEngine {}

/// `env.now() -> i64`.
unsafe extern "C" fn host_now(runtime: IM3Runtime, _: IM3ImportContext, sp: *mut u64, _: *mut c_void) -> *const c_void {
    let ctx = unsafe { &*(extreme_wasm3::userdata(runtime) as *const Ctx) };
    unsafe { *sp = ctx.now.get() };
    OK
}

/// `env.log(ptr i32, len i32)`.
unsafe extern "C" fn host_log(runtime: IM3Runtime, _: IM3ImportContext, sp: *mut u64, _: *mut c_void) -> *const c_void {
    let ctx = unsafe { &*(extreme_wasm3::userdata(runtime) as *const Ctx) };
    let (ptr, len) = unsafe { (*sp as u32 as usize, *sp.add(1) as u32 as usize) };
    let memory = unsafe { extreme_wasm3::memory(runtime) };
    match memory.get(ptr..ptr.saturating_add(len.min(LOG_MAX))) {
        Some(bytes) => info!(
            "engine {}: {}",
            ctx.name,
            core::str::from_utf8(bytes).unwrap_or("<not UTF-8>")
        ),
        None => warn!("engine {}: log outside its memory", ctx.name),
    }
    OK
}

impl WasmEngine {
    /// Instantiates `package`'s engine and runs its `init`, at `now`.
    pub fn new(package: &'static Package, now: u64) -> Result<Self, Error> {
        if package.abi != abi::VERSION {
            return Err("the package's engine ABI is not v1");
        }
        if package.engine.is_empty() {
            return Err("the package has no engine");
        }
        let ctx = Box::new(Ctx {
            now: Cell::new(now),
            name: package.name,
        });
        let userdata = &*ctx as *const Ctx as *mut c_void;
        // `ctx` is boxed and kept in the engine, dropped after `wasm`
        let mut wasm = unsafe { Wasm3::new(package.engine, VALUE_STACK, userdata) }?;
        wasm.link(names::MODULE, names::NOW, c"I()", host_now)?;
        wasm.link(names::MODULE, names::LOG, c"v(ii)", host_log)?;
        let find = |name: &CStr| wasm.function(name);
        let init = find(names::INIT)?;
        let mut engine = WasmEngine {
            on_location: find(names::ON_LOCATION)?,
            on_timer: find(names::ON_TIMER)?,
            on_event: find(names::ON_EVENT)?,
            view: find(names::VIEW)?,
            next_timer: find(names::NEXT_TIMER)?,
            wasm,
            ctx,
            io: 0,
            io_cap: 0,
        };
        let io: u64 = unsafe { engine.wasm.call(init, &[]) }?;
        let (io, cap) = abi::split_io_buffer(io);
        let (io, cap) = (io as usize, cap as usize);
        if io.checked_add(cap).is_none_or(|end| end > engine.wasm.memory().len()) {
            return Err("the engine's I/O buffer is outside its memory");
        }
        engine.io = io;
        engine.io_cap = cap;
        Ok(engine)
    }

    /// The engine's linear memory, bytes.
    pub fn memory_size(&mut self) -> usize {
        self.wasm.memory().len()
    }

    /// Writes `bytes` to the I/O buffer; its address and length, as the
    /// exports take them.
    fn input(&mut self, bytes: &[u8]) -> Result<(u32, u32), Error> {
        if bytes.len() > self.io_cap {
            return Err("input larger than the engine's I/O buffer");
        }
        self.wasm.memory()[self.io..self.io + bytes.len()].copy_from_slice(bytes);
        Ok((self.io as u32, bytes.len() as u32))
    }

    fn outcome(r: i32) -> Outcome {
        match r {
            abi::CHANGED => Outcome::Changed,
            abi::UNCHANGED => Outcome::Unchanged,
            _ => Outcome::Refused,
        }
    }

    /// Calls a `(ptr, len) -> i32` export with `bytes`.
    fn call_bytes(&mut self, f: Function, now: u64, bytes: &[u8]) -> Result<Outcome, Error> {
        self.ctx.now.set(now);
        let (ptr, len) = self.input(bytes)?;
        let r: i32 = unsafe {
            self.wasm.call(
                f,
                &[
                    &ptr as *const u32 as *const c_void,
                    &len as *const u32 as *const c_void,
                ],
            )
        }?;
        Ok(Self::outcome(r))
    }

    /// A GPS update. `Err` is a trap: the engine is broken.
    pub fn on_location(&mut self, now: u64, location: &Location) -> Result<Outcome, Error> {
        let mut buf = [0u8; 64];
        let bytes = postcard::to_slice(location, &mut buf).map_err(|_| "location does not encode")?;
        let f = self.on_location;
        self.call_bytes(f, now, bytes)
    }

    /// The timer the engine asked for, due at `at`.
    pub fn on_timer(&mut self, now: u64, at: u64) -> Result<Outcome, Error> {
        self.ctx.now.set(now);
        let f = self.on_timer;
        let r: i32 = unsafe { self.wasm.call(f, &[&at as *const u64 as *const c_void]) }?;
        Ok(Self::outcome(r))
    }

    /// A client event (the engine's own postcard). `Refused` covers bytes
    /// too long for its I/O buffer too.
    pub fn on_event(&mut self, now: u64, bytes: &[u8]) -> Result<Outcome, Error> {
        if bytes.len() > self.io_cap {
            return Ok(Outcome::Refused);
        }
        let f = self.on_event;
        self.call_bytes(f, now, bytes)
    }

    /// The view, postcard, into `out`; its length.
    pub fn view(&mut self, now: u64, out: &mut [u8]) -> Result<usize, Error> {
        self.ctx.now.set(now);
        let (ptr, cap) = (self.io as u32, self.io_cap as u32);
        let f = self.view;
        let len: i32 = unsafe {
            self.wasm.call(
                f,
                &[
                    &ptr as *const u32 as *const c_void,
                    &cap as *const u32 as *const c_void,
                ],
            )
        }?;
        if len < 0 || len as usize > out.len() || len as u32 > cap {
            return Err("the engine's view does not fit");
        }
        let len = len as usize;
        out[..len].copy_from_slice(&self.wasm.memory()[self.io..self.io + len]);
        Ok(len)
    }

    /// When the engine next wants [`on_timer`](Self::on_timer).
    pub fn next_timer(&mut self) -> Result<Option<u64>, Error> {
        let f = self.next_timer;
        let at: u64 = unsafe { self.wasm.call(f, &[]) }?;
        Ok((at != abi::NO_TIMER).then_some(at))
    }

    /// Bytes at the bottom of the engine's linear memory never written (the
    /// stack grows down towards address 0 and the memory starts zeroed), so
    /// how much of its stack it has not needed yet. Tests and logs.
    pub fn stack_unused(&mut self) -> usize {
        self.wasm.memory().iter().take_while(|b| **b == 0).count()
    }
}
