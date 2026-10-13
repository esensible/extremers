//! Memory use in the log: the heap (esp-alloc's statistics), the main
//! stack's high-water mark, and the engines' static buffers.
//!
//! The main stack is the RAM left after .data/.bss (the `.stack` section);
//! the main task, and with it the embassy executor and every task and
//! engine call, runs on it. [`paint_main_stack`] fills its unused part with
//! a pattern first thing in `main`; the high-water mark is the deepest word
//! overwritten since. [`memory_task`] repaints it once boot is done, so the
//! mark it logs from then on is that of steady running (HTTP, BLE, engine
//! switches and calls).

use core::ptr::{addr_of, read_volatile, write_volatile};

use embassy_time::{Duration, Timer};

unsafe extern "C" {
    /// Top of the main stack (highest address).
    static _stack_start_cpu0: u32;
    /// Bottom of the main stack (lowest address).
    static _stack_end_cpu0: u32;
}

const PATTERN: u32 = 0x5AA5_C33C;
/// Left alone at the bottom: esp-hal's stack guard word lives there.
const GUARD: usize = 256;

fn bounds() -> (usize, usize) {
    (
        addr_of!(_stack_end_cpu0) as usize,
        addr_of!(_stack_start_cpu0) as usize,
    )
}

/// Paints the main stack below the current frame (less 1 KiB of margin).
pub fn paint_main_stack() {
    let (bottom, _) = bounds();
    let sp: usize;
    unsafe { core::arch::asm!("mv {}, sp", out(reg) sp) };
    let mut a = bottom + GUARD;
    while a < ((sp - 1024) & !3) {
        unsafe { write_volatile(a as *mut u32, PATTERN) };
        a += 4;
    }
}

/// (deepest use of the main stack in bytes, its size in bytes)
pub fn main_stack_high_water() -> (usize, usize) {
    let (bottom, top) = bounds();
    let mut a = bottom + GUARD;
    while a < top && unsafe { read_volatile(a as *const u32) } == PATTERN {
        a += 4;
    }
    (top - a, top - bottom)
}

/// Logs the heap and the main stack, labelled.
pub fn log(label: &str) {
    let s = esp_alloc::HEAP.stats();
    let (stack_used, stack_size) = main_stack_high_water();
    defmt::info!(
        "memory {=str}: heap used {=usize} free {=usize} of {=usize}, min free {=usize}; main stack high water {=usize} of {=usize}; engine buffers {=usize} (static)",
        label,
        s.current_usage,
        s.size - s.current_usage,
        s.size,
        s.size - s.max_usage,
        stack_used,
        stack_size,
        core::mem::size_of::<common::engine::EngineMemory>()
    );
}

/// Every 10 s, the memory log; the main stack repainted once after boot,
/// and its high-water mark logged whenever it grows (with the log lines
/// around it saying what took it there).
#[embassy_executor::task]
pub async fn memory_task() -> ! {
    Timer::after(Duration::from_millis(2500)).await;
    let (boot, _) = main_stack_high_water();
    defmt::info!("memory: main stack high water at boot {=usize}; repainted, from here on after boot", boot);
    paint_main_stack();
    let mut last = 0;
    let mut n = 0u32;
    loop {
        Timer::after(Duration::from_millis(200)).await;
        let (used, _) = main_stack_high_water();
        if used > last {
            defmt::info!("memory: main stack high water grew: {=usize} -> {=usize}", last, used);
            last = used;
        }
        n += 1;
        if n % 50 == 0 {
            log("periodic");
        }
    }
}
