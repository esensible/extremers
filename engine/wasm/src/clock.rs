//! The host's clock: an embassy-time driver whose time moves only when told
//! to ([`Clock::set`]), so a `common::Host` runs on virtual time.
//!
//! Each [`HostHandle`](crate::host::HostHandle) owns a [`Clock`] and makes
//! it the current one for the length of each call ([`Clock::enter`]): two
//! hosts in one module keep their own time. The current clock is
//! thread-local (the native tests run in parallel threads; the wasm module
//! has one thread). Outside any call the time is 0 and nothing is recorded.

use core::{cell::RefCell, task::Waker};

use embassy_time_driver::{Driver, TICK_HZ};

/// A virtual clock: its time in ticks, and the wakes futures asked for.
#[derive(Default)]
pub struct Clock {
    now: u64,
    wakes: Vec<(u64, Waker)>,
}

thread_local! {
    static CURRENT: RefCell<Clock> = RefCell::new(Clock::default());
}

impl Clock {
    /// Runs `f` with this clock as the current one.
    pub fn enter<R>(&mut self, f: impl FnOnce() -> R) -> R {
        CURRENT.with(|c| core::mem::swap(&mut *c.borrow_mut(), self));
        let r = f();
        CURRENT.with(|c| core::mem::swap(&mut *c.borrow_mut(), self));
        r
    }
}

/// The current clock's time, in ticks.
pub fn now() -> u64 {
    CURRENT.with(|c| c.borrow().now)
}

/// Moves the current clock to `ticks` (never back), and wakes every future
/// whose time has come.
pub fn set(ticks: u64) {
    let due: Vec<Waker> = CURRENT.with(|c| {
        let mut c = c.borrow_mut();
        c.now = c.now.max(ticks);
        let now = c.now;
        let (due, later) = core::mem::take(&mut c.wakes).into_iter().partition(|(at, _)| *at <= now);
        c.wakes = later;
        due.into_iter().map(|(_, w)| w).collect()
    });
    // outside the borrow: a waker may be anything
    due.into_iter().for_each(Waker::wake);
}

/// The earliest wake asked for, in ticks.
pub fn next_wake() -> Option<u64> {
    CURRENT.with(|c| c.borrow().wakes.iter().map(|(at, _)| *at).min())
}

/// Forgets the wakes asked for (before a future is polled again, which asks
/// again for what it still waits on).
pub fn clear_wakes() {
    CURRENT.with(|c| c.borrow_mut().wakes.clear());
}

/// Milliseconds as ticks.
pub fn ticks(ms: u64) -> u64 {
    ms.saturating_mul(TICK_HZ) / 1000
}

struct VirtualDriver;

impl Driver for VirtualDriver {
    fn now(&self) -> u64 {
        now()
    }

    fn schedule_wake(&self, at: u64, waker: &Waker) {
        CURRENT.with(|c| c.borrow_mut().wakes.push((at, waker.clone())));
    }
}

embassy_time_driver::time_driver_impl!(static DRIVER: VirtualDriver = VirtualDriver);
