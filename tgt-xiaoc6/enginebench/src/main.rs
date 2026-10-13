//! The Race engine on the ESP32-C6. Not the firmware: a bench, flashed on
//! its own, that logs over defmt/RTT and then idles. (It compared Race
//! under wasm3 with Race on the VM until wasm3 left the firmware: GPS
//! evaluation 2,988 µs there against 1,653 µs here, heap +54 KB against 0.)
//!
//! engine/race/engine.expr as bytecode (in flash, run in place) on
//! `lang/vm`, with the sailing builtins, called as the host does
//! (engine/PROFILE.md §3): one `vm::run` per event (`{gps: …}`, `{wake:
//! {}}` or one of Race's own), the output's `state` copied into the state
//! buffer, its `wake` read as the timer, and its `view` read in the output
//! buffer.
//!
//! The events come as the firmware delivers them (no tick): a set line; GPS
//! updates approaching it; a sequence and bump events; then sequences of
//! one second, each ended by the wake at its gun. The wakes are scripted at
//! the times the engine asks for, and the bench checks that it asked for
//! exactly those. Times come from the SYSTIMER (µs); RAM from esp-alloc's
//! statistics; native stack by painting it. Same heap as the firmware (64
//! KiB + 64 KiB), same CPU clock, the firmware's release profile; but
//! nothing else running (no radio, no executor), so this is the engine
//! alone.

#![no_std]
#![no_main]

use core::ptr::{addr_of, read_volatile, write_volatile};

use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{clock::CpuClock, delay::Delay, main, ram, time::Instant};

esp_bootloader_esp_idf::esp_app_desc!();

defmt::timestamp!("{=u64:ms}", Instant::now().duration_since_epoch().as_millis());

include!(concat!(env!("OUT_DIR"), "/consts.rs"));

/// Timed evaluations of each kind.
const N: usize = 300;
const T0: u64 = 1_790_000_000_000;
/// The line's ends off Sydney Heads, ~185 m apart (engine/race's tests').
const STBD: (f64, f64) = (-33.85, 151.252);
const PORT: (f64, f64) = (-33.85, 151.250);

#[derive(Clone, Copy, PartialEq)]
enum Input {
    Fix { lat: f64, lon: f64, speed: f64, heading: f64 },
    Wake,
    LineStbd,
    LinePort,
    Bump { seconds: i32, tapped_at: u64 },
    Finish,
}

/// The `k`th fix of the GPS phase: 20-330 m south of the line's middle,
/// heading within 10° of north at 3-7 kn, so always pointing at the line
/// (the expensive path: bearings, the crossing, two distances).
fn approach(k: usize) -> Input {
    let lat = -33.8530 + (k % 100) as f64 * 0.00002;
    let lon = 151.251 + ((k * 7) % 11) as f64 * 0.00002 - 0.0001;
    let h = ((k * 13) % 21) as f64 - 10.0;
    Input::Fix { lat, lon, speed: 3.0 + (k % 9) as f64 * 0.5, heading: if h < 0.0 { h + 360.0 } else { h } }
}

// ---- measuring ----

unsafe extern "C" {
    static _stack_start_cpu0: u32;
    static _stack_end_cpu0: u32;
}

const PATTERN: u32 = 0x5AA5_C33C;
/// How far below the caller's frame [`stack_used`] paints.
const PAINT: usize = 48 * 1024;

/// Runs `f`, and the most native stack it used below this frame, bytes (the
/// region below is painted first; the deepest overwritten word counts).
#[inline(never)]
fn stack_used<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let sp: usize;
    unsafe { core::arch::asm!("mv {}, sp", out(reg) sp) };
    let bottom = addr_of!(_stack_end_cpu0) as usize + 256;
    let from = (sp - PAINT).max(bottom) & !3;
    let to = (sp - 64) & !3;
    let mut a = from;
    while a < to {
        unsafe { write_volatile(a as *mut u32, PATTERN) };
        a += 4;
    }
    let r = f();
    let mut a = from;
    while a < to && unsafe { read_volatile(a as *const u32) } == PATTERN {
        a += 4;
    }
    (r, sp - a)
}

fn main_stack_size() -> usize {
    addr_of!(_stack_start_cpu0) as usize - addr_of!(_stack_end_cpu0) as usize
}

fn heap() -> (usize, usize) {
    let s = esp_alloc::HEAP.stats();
    (s.current_usage, s.max_usage)
}

/// Per-evaluation times, µs.
struct Times {
    us: [u32; N + 1],
    n: usize,
}

impl Times {
    const fn new() -> Self {
        Times { us: [0; N + 1], n: 0 }
    }

    fn push(&mut self, us: u64) {
        if self.n < self.us.len() {
            self.us[self.n] = us as u32;
            self.n += 1;
        }
    }

    /// Logs min, median, max and mean over all but the first (which is
    /// logged on its own: wasm3 compiles a function on its first call).
    fn log(&mut self, what: &str) {
        let first = self.us[0];
        let rest = &mut self.us[1..self.n];
        rest.sort_unstable();
        let sum: u64 = rest.iter().map(|&u| u as u64).sum();
        defmt::info!(
            "time {=str}: n {=usize} min {=u32} median {=u32} max {=u32} mean {=u64} us (first {=u32} us)",
            what,
            rest.len(),
            rest[0],
            rest[rest.len() / 2],
            rest[rest.len() - 1],
            sum / rest.len() as u64,
            first
        );
    }
}

fn micros(since: Instant) -> u64 {
    since.elapsed().as_micros()
}

/// What clients would see, and the timer, in the terms the wasm3 engine's
/// view had (from when the two were compared): a line with both ends but
/// no crossing yet is time 0, no speed yet speed 0. The bench checks the
/// timer against the script.
#[derive(Clone, Copy, PartialEq, defmt::Format)]
struct Seen {
    phase: u8,
    start: i64,
    line: u8,
    time: i64,
    speed: f32,
    /// The timer it wants, or -1.
    wake: i64,
}

// ---- the engine ----

mod new {
    use vm::{HostFn, ListInput, Program, Value};

    use super::*;

    pub static RACE_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/race.bin"));

    // The leaves (lang/SPEC.md §7.3), records in field-name order, an
    // optional record (each variant is one) led by a presence leaf. build.rs
    // works out every offset (OUT_*, VW_*, EV_*, FIX_*) from the
    // interface. The inputs: state? (presence, then the state), event,
    // timestamp, gps; the output `{state, view, wake?}`.
    const _: () = assert!(BASE_STATE == 0 && BASE_EVENT == 1 + STATE && LIST_COUNT == 1);
    const _: () = assert!(
        OUTPUT_SLOTS == STATE + VIEW + 1
            && OUT_STATE + STATE <= OUTPUT_SLOTS
            && OUT_VIEW + VIEW <= OUTPUT_SLOTS
            && (OUT_VIEW >= OUT_STATE + STATE || OUT_STATE >= OUT_VIEW + VIEW)
            && (OUT_WAKE < OUT_STATE || OUT_WAKE >= OUT_STATE + STATE)
            && (OUT_WAKE < OUT_VIEW || OUT_WAKE >= OUT_VIEW + VIEW)
    );

    /// A fix as the track slot stores it (PROFILE.md §3).
    #[derive(Clone, Copy)]
    pub struct TrackFix {
        pub t: i64,
        pub lat: f64,
        pub lon: f64,
    }

    /// `gps`: the first `len` fixes, read in place; `alt` is absent.
    pub struct Track<'f> {
        pub fixes: &'f [TrackFix],
        pub len: u32,
    }

    impl<'a> ListInput<'a> for Track<'_> {
        fn len(&self) -> u32 {
            self.len
        }
        fn read(&self, item: u32, leaf: u16) -> Value<'a> {
            let f = &self.fixes[item as usize];
            match leaf as usize {
                FIX_LAT => Value::Float(f.lat),
                FIX_LON => Value::Float(f.lon),
                FIX_T => Value::Int(f.t),
                _ => Value::Nil,
            }
        }
    }

    pub struct Engine {
        pub p: Program<'static>,
        pub linked: [Option<&'static HostFn>; 8],
        /// The state (the last output's `state`), or none before the first
        /// evaluation.
        pub state: Option<[Value<'static>; STATE]>,
        /// The last output's `wake`.
        pub wake: Option<i64>,
        pub output: [Value<'static>; OUTPUT_SLOTS],
        pub stack: [Value<'static>; STACK_SLOTS],
        pub locals: [Value<'static>; LOCALS],
    }

    /// `vm::load` + `vm::link`; `scratch` is the validation buffer.
    pub fn load(scratch: &mut [u16]) -> Engine {
        let limits = vm::Limits { max_stack: STACK_SLOTS as u16, max_locals: LOCALS as u16 };
        let p = vm::load(RACE_BIN, scratch, &limits).unwrap();
        let mut linked = [None; 8];
        vm::link(&p, &sailing::BUILTINS, &mut linked).unwrap();
        Engine {
            p,
            linked,
            state: None,
            wake: None,
            output: [Value::Nil; OUTPUT_SLOTS],
            stack: [Value::Nil; STACK_SLOTS],
            locals: [Value::Nil; LOCALS],
        }
    }

    impl Engine {
        /// One event, as the host will do it: the inputs laid out, the
        /// program, the output's `state` kept as the state and its `wake` as
        /// the timer. A fix is appended to the track (`fixes`, `len`) first.
        /// The instructions it took.
        pub fn eval(&mut self, now: u64, input: Input, fixes: &mut [TrackFix], len: &mut u32) -> u32 {
            let mut inputs = [Value::Nil; INPUT_LEAVES];
            if let Some(s) = &self.state {
                inputs[BASE_STATE] = Value::Bool(true);
                inputs[BASE_STATE + 1..BASE_STATE + 1 + STATE].copy_from_slice(s);
            }
            let variant = match input {
                Input::Fix { lat, lon, speed, heading } => {
                    fixes[*len as usize] = TrackFix { t: now as i64, lat, lon };
                    *len += 1;
                    inputs[EV_GPS_FIX] = Value::Bool(true);
                    inputs[EV_GPS_FIX_LAT] = Value::Float(lat);
                    inputs[EV_GPS_FIX_LON] = Value::Float(lon);
                    inputs[EV_GPS_FIX_T] = Value::Int(now as i64);
                    inputs[EV_GPS_VELOCITY] = Value::Bool(true);
                    inputs[EV_GPS_VELOCITY_HEADING] = Value::Float(heading);
                    inputs[EV_GPS_VELOCITY_SPEED] = Value::Float(speed);
                    EV_GPS
                }
                Input::Wake => EV_WAKE,
                Input::LineStbd => EV_LINE_STBD,
                Input::LinePort => EV_LINE_PORT,
                Input::Bump { seconds, tapped_at } => {
                    inputs[EV_BUMP_SEQ_SECONDS] = Value::Int(seconds as i64);
                    inputs[EV_BUMP_SEQ_TAPPED_AT] = Value::Int(tapped_at as i64);
                    EV_BUMP_SEQ
                }
                Input::Finish => EV_RACE_FINISH,
            };
            inputs[variant] = Value::Bool(true);
            inputs[BASE_TIMESTAMP] = Value::Int(now as i64);
            let track = Track { fixes, len: *len };
            let lists: [Option<&dyn ListInput>; LIST_COUNT] = [Some(&track)];
            // (`Stats::steps` counts instructions)
            let instructions = match vm::run(&self.p, &self.linked, &inputs, &lists, &mut self.output, &mut self.stack, &mut self.locals) {
                Ok(stats) => stats.steps,
                Err(f) => panic!("vm: {} at {}", f.code.as_str(), f.pc),
            };
            let mut s = [Value::Nil; STATE];
            s.copy_from_slice(&self.output[OUT_STATE..OUT_STATE + STATE]);
            self.state = Some(s);
            self.wake = match self.output[OUT_WAKE] {
                Value::Int(w) => Some(w),
                Value::Nil => None,
                _ => panic!("the output's wake is an int or absent"),
            };
            instructions
        }

        /// The last output's view, read where the host reads it (in the
        /// output buffer), in the old view's terms.
        pub fn seen(&self) -> Seen {
            let v = &self.output[OUT_VIEW..OUT_VIEW + VIEW];
            let int = |v: Value| match v {
                Value::Int(i) => i,
                _ => 0,
            };
            let float = |v: Value| match v {
                Value::Float(f) => f,
                Value::Int(i) => i as f64,
                _ => f64::NAN,
            };
            let present = |i: usize| v[i] != Value::Nil;
            let set = |i: usize| match v[i] {
                Value::Bool(b) => b,
                _ => panic!("the view's line ends are bools"),
            };
            let (phase, start) = if present(VW_COUNTDOWN) {
                (1, int(v[VW_COUNTDOWN_GUN]))
            } else if present(VW_RACING) {
                (2, int(v[VW_RACING_GUN]))
            } else {
                assert!(present(VW_IDLE));
                (0, 0)
            };
            let line = match (set(VW_STBD), set(VW_PORT)) {
                (false, false) => 0,
                (true, false) => 1,
                (false, true) => 2,
                (true, true) => 3,
            };
            let racing = phase == 2;
            Seen {
                phase,
                start,
                // the old view has no line while racing; the new one isn't shown then
                line: if racing { 0 } else { line },
                time: if present(VW_CROSSING) { int(v[VW_CROSSING]) } else { 0 },
                speed: if present(VW_SPEED) { float(v[VW_SPEED]) as f32 } else { 0.0 },
                wake: self.wake.unwrap_or(-1),
            }
        }
    }
}

// ---- the bench ----

/// Evaluations: setup, N GPS updates, a sequence, N bumps, a finish, then N
/// (bump of 1 s, wake 1 s later) pairs.
const EVALUATIONS: usize = 5 + N + 1 + N + 1 + 2 * N;
const GPS_FROM: usize = 5;
const BUMP_FROM: usize = GPS_FROM + N + 1;

/// 0 GPS update (line set), 1 bump in a sequence, 2 wake (the gun), 3 other.
fn kind(i: usize, input: Input) -> usize {
    match input {
        Input::Wake => 2,
        _ if (GPS_FROM..GPS_FROM + N).contains(&i) => 0,
        _ if (BUMP_FROM..BUMP_FROM + N).contains(&i) => 1,
        _ => 3,
    }
}

#[main]
fn main() -> ! {
    rtt_target::rtt_init_defmt!(rtt_target::ChannelMode::BlockIfFull, 4096);
    let _peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 64 * 1024);
    let delay = Delay::new();
    delay.delay_millis(1500);

    let (heap0, _) = heap();
    defmt::info!(
        "enginebench: CPU {=u32} MHz; heap {=usize} B, {=usize} used at start; main stack {=usize} B",
        esp_hal::clock::cpu_clock().as_mhz(),
        esp_alloc::HEAP.stats().size,
        heap0,
        main_stack_size()
    );
    defmt::info!(
        "code: race bytecode {=usize} B; compiler: stack {=usize} + locals {=usize} + output {=usize} slots x 16 = {=usize} B; Value is {=usize} B",
        new::RACE_BIN.len(),
        STACK_SLOTS,
        LOCALS,
        OUTPUT_SLOTS,
        COMPILER_RAM_BYTES,
        core::mem::size_of::<vm::Value>()
    );

    // The script, with the wakes where the engines will ask for them (each
    // evaluation checks both engines asked for exactly the scripted one).
    let mut script: [(u64, Input); EVALUATIONS] = [(0, Input::Wake); EVALUATIONS];
    let mut t = T0;
    let mut k = 0;
    let mut add = |dt: u64, input: Input, script: &mut [(u64, Input)]| {
        t += dt;
        script[k] = (t, input);
        k += 1;
        t
    };
    add(0, Input::Fix { lat: STBD.0, lon: STBD.1, speed: 4.0, heading: 90.0 }, &mut script);
    add(500, Input::LineStbd, &mut script);
    add(500, Input::Fix { lat: PORT.0, lon: PORT.1, speed: 4.0, heading: 270.0 }, &mut script);
    add(500, Input::LinePort, &mut script);
    add(500, Input::Fix { lat: -33.853, lon: 151.251, speed: 5.0, heading: 0.0 }, &mut script);
    let mut last = T0;
    for i in 0..N {
        // irregular, as a receiver delivers them: 0.2-1.9 s, now and then 12 s
        let dt = if i % 37 == 36 { 12_000 } else { 200 + ((i * 577) % 1700) as u64 };
        last = add(dt, approach(i), &mut script);
    }
    // a sequence: the gun 10 minutes from a tap 100 ms before the event
    let tap = add(500, Input::Bump { seconds: 600, tapped_at: last + 400 }, &mut script);
    for i in 0..N {
        add(500, Input::Bump { seconds: if i % 2 == 0 { 1 } else { -1 }, tapped_at: tap + 500 * i as u64 }, &mut script);
    }
    last = add(500, Input::Finish, &mut script);
    for _ in 0..N {
        // a sequence of one second from a tap at the event, then its gun
        let tap = add(700, Input::Bump { seconds: 1, tapped_at: last + 700 }, &mut script);
        last = add(1000, Input::Wake, &mut script);
        assert!(last == tap + 1000);
    }
    assert!(k == EVALUATIONS);
    // Before each scripted wake, the engine must have asked for exactly it:
    // `Some(true)` if `seen` asks for the wake scripted next.
    let asks_for_next_wake = |i: usize, seen: &Seen| match script.get(i + 1) {
        Some(&(at, Input::Wake)) => Some(seen.wake == at as i64),
        _ => None,
    };

    {
        use new::TrackFix;
        let (h0, _) = heap();
        let mut scratch = [0u16; 8192];
        let ((mut e, load_us), load_stack) = stack_used(|| {
            let t = Instant::now();
            let e = new::load(&mut scratch);
            (e, micros(t))
        });
        let (h1, _) = heap();
        defmt::info!(
            "new load: vm::load (parse, validate) + vm::link {=u64} us; native stack {=usize} B; heap +{=usize} B; validation scratch {=usize} B (2 per code byte, only during load; {=usize} code bytes)",
            load_us,
            load_stack,
            h1 - h0,
            2 * e.p.code.len(),
            e.p.code.len()
        );
        let mut fixes = [TrackFix { t: 0, lat: 0.0, lon: 0.0 }; N + 8];
        let mut len = 0u32;
        let mut times = [Times::new(), Times::new(), Times::new()];
        let mut stack = [0usize; 3];
        let mut instructions = [0u32; 3];
        let mut seen_kind = [0usize; 3];
        let mut timers_off = 0;
        for (i, &(now, input)) in script.iter().enumerate() {
            let kind = kind(i, input);
            let us = if kind < 3 && seen_kind[kind] == 10 {
                let ((us, counts), used) = stack_used(|| {
                    let t = Instant::now();
                    let counts = e.eval(now, input, &mut fixes, &mut len);
                    (micros(t), counts)
                });
                stack[kind] = used;
                instructions[kind] = counts;
                us
            } else {
                let t = Instant::now();
                e.eval(now, input, &mut fixes, &mut len);
                micros(t)
            };
            if kind < 3 {
                seen_kind[kind] += 1;
                times[kind].push(us);
            }
            let seen = e.seen();
            if asks_for_next_wake(i, &seen) == Some(false) {
                timers_off += 1;
            }
        }
        times[0].log("new GPS update, line set (program + state copy)");
        times[1].log("new bump event (program + state copy)");
        times[2].log("new wake, the gun (program + state copy)");
        defmt::info!(
            "new instructions per evaluation: GPS update {=u32}, bump {=u32}, wake {=u32}",
            instructions[0],
            instructions[1],
            instructions[2]
        );
        defmt::info!("new native stack: GPS update {=usize} B, bump {=usize} B, wake {=usize} B", stack[0], stack[1], stack[2]);
        let (h2, _) = heap();
        let program_slots = STACK_SLOTS + LOCALS + OUTPUT_SLOTS + STATE + INPUT_LEAVES;
        defmt::info!(
            "new RAM: heap +{=usize} B; the host's buffers: stack {=usize} + locals {=usize} + output {=usize} + state {=usize} + inputs {=usize} slots x {=usize} B = {=usize} B; linked {=usize} B",
            h2 - h0,
            STACK_SLOTS,
            LOCALS,
            OUTPUT_SLOTS,
            STATE,
            INPUT_LEAVES,
            core::mem::size_of::<vm::Value>(),
            program_slots * core::mem::size_of::<vm::Value>(),
            core::mem::size_of_val(&e.linked)
        );
        defmt::info!("new timer: {=usize} evaluations asked for a timer other than the scripted wake", timers_off);
    }

    defmt::info!("enginebench: done");
    loop {
        delay.delay_millis(1000);
    }
}
