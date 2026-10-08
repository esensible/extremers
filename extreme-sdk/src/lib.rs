//! Writing an engine package (README "Packages").
//!
//! An *engine* is a pure state machine: it is fed GPS updates
//! ([`Engine::on_location`]), its own timer ([`Engine::on_timer`]) and client
//! events ([`Engine::on_event`]), and says whether its view changed. It does
//! no I/O and has no clock of its own, so it is unit tested natively on the
//! host like any Rust code. What clients see of it is its
//! [`Engine::View`]; what they send it is its [`Engine::Event`]: both are
//! postcard on the wire and derive postcard-schema's `Schema`, from which
//! `tools/bindgen` generates the package's own JS bindings.
//!
//! On the device an engine runs as WebAssembly under the host's wasm3
//! interpreter. [`export_engine!`] turns an `Engine` into a module speaking
//! [ABI v1](abi), and `tools/package` builds it (with the package's web and
//! watch bundles) into a package.
//!
//! Times are epoch milliseconds on the device's clock (uptime until the GPS
//! has given it the time), the one timeline every client syncs to.

#![no_std]

use postcard_schema::Schema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub mod abi;

// For the macro; not part of the API.
#[doc(hidden)]
pub use postcard;

/// A GPS position, in degrees. South and west are negative.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize, Schema)]
pub struct Fix {
    pub lat: f64,
    pub lon: f64,
}

/// Speed over ground in knots and course over ground in degrees true.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize, Schema)]
pub struct Velocity {
    pub speed: f64,
    pub heading: f64,
}

/// A GPS update, as the host passes it to [`Engine::on_location`] (postcard
/// across the ABI).
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize, Schema)]
pub struct Location {
    /// When, epoch ms: the GPS time when it has one, else the device's
    /// clock.
    pub timestamp: u64,
    pub fix: Option<Fix>,
    pub velocity: Option<Velocity>,
}

/// An engine. See the crate docs.
///
/// Every `on_*` returns whether the view changed, so clients must be told.
/// After every call the host asks [`next_timer`](Self::next_timer) when the
/// engine next wants [`on_timer`](Self::on_timer).
pub trait Engine: Default {
    /// What clients see: a typed value built from the state by
    /// [`view`](Self::view), leaving out internals. Absolute times in it are
    /// device times; every message also carries the device's time when it
    /// was sent, so clients count down against their own clocks.
    type View: Serialize + Schema;

    /// Events from clients. `()` for an engine that takes none.
    type Event: DeserializeOwned + Schema;

    /// The view of the current state.
    fn view(&self) -> Self::View;

    /// A GPS update.
    fn on_location(&mut self, location: &Location) -> bool;

    /// The timer [`next_timer`](Self::next_timer) asked for is due; `now` is
    /// the time it was due at.
    fn on_timer(&mut self, now: u64) -> bool;

    /// An event from a client, arriving at `now`.
    fn on_event(&mut self, now: u64, event: &Self::Event) -> bool;

    /// When the engine next wants [`on_timer`](Self::on_timer), `None` for
    /// never. A time already past fires at once.
    fn next_timer(&self) -> Option<u64> {
        None
    }
}

/// Decodes a client event: `None` unless `bytes` is exactly one `E`
/// (trailing bytes are refused too).
pub fn decode_event<E: DeserializeOwned>(bytes: &[u8]) -> Option<E> {
    match postcard::take_from_bytes(bytes) {
        Ok((event, [])) => Some(event),
        _ => None,
    }
}

/// Feeds `bytes` to `engine` as a client event, as the ABI does: `None` if
/// it is not exactly one `E::Event`, else whether the view changed.
pub fn raw_event<E: Engine>(engine: &mut E, now: u64, bytes: &[u8]) -> Option<bool> {
    let event = decode_event::<E::Event>(bytes)?;
    Some(engine.on_event(now, &event))
}

/// The engine's view as postcard into `out`; its length, `None` if it does
/// not fit.
pub fn encode_view<E: Engine>(engine: &E, out: &mut [u8]) -> Option<usize> {
    postcard::to_slice(&engine.view(), out).ok().map(|b| b.len())
}

/// Single-threaded module state for [`export_engine!`]: wasm32-unknown-unknown
/// has no threads, and the host never calls an export re-entrantly.
#[doc(hidden)]
pub struct Static<T>(pub core::cell::UnsafeCell<T>);

unsafe impl<T> Sync for Static<T> {}

impl<T> Static<T> {
    pub const fn new(value: T) -> Self {
        Self(core::cell::UnsafeCell::new(value))
    }

    /// # Safety
    /// No other reference to the value may be live (true of the exports,
    /// which never nest).
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get(&self) -> &mut T {
        unsafe { &mut *self.0.get() }
    }
}

/// Exports `$engine` (an [`Engine`]) as a WebAssembly module speaking
/// [ABI v1](abi). Put it at the root of the engine crate; it only expands
/// for `wasm32` (on the host the crate is an ordinary library, tested
/// natively). It also provides the module's panic handler (a trap).
///
/// ```ignore
/// extreme_sdk::export_engine!(Race);
/// ```
#[macro_export]
macro_rules! export_engine {
    ($engine:ty) => {
        #[cfg(target_arch = "wasm32")]
        mod __extreme_engine_abi {
            use $crate::{Engine as _, Static, abi};

            type E = $engine;

            #[panic_handler]
            fn panic(_: &core::panic::PanicInfo) -> ! {
                core::arch::wasm32::unreachable()
            }

            #[link(wasm_import_module = "env")]
            unsafe extern "C" {
                /// The device's clock, epoch ms.
                #[link_name = "now"]
                fn host_now() -> u64;
                /// A line for the device's log (UTF-8).
                #[link_name = "log"]
                fn host_log(ptr: *const u8, len: usize);
            }

            static ENGINE: Static<Option<E>> = Static::new(None);
            static IO: Static<[u8; abi::IO_CAP]> = Static::new([0; abi::IO_CAP]);

            fn engine() -> &'static mut E {
                unsafe { ENGINE.get() }.get_or_insert_with(E::default)
            }

            fn changed(changed: bool) -> i32 {
                if changed { abi::CHANGED } else { abi::UNCHANGED }
            }

            /// `len` bytes the host wrote at `ptr` (the I/O buffer).
            fn input<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
                if ptr.is_null() || len > abi::IO_CAP {
                    return &[];
                }
                unsafe { core::slice::from_raw_parts(ptr, len) }
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn init() -> u64 {
                unsafe { *ENGINE.get() = Some(E::default()) };
                let ptr = unsafe { IO.get() }.as_mut_ptr();
                let msg = "init";
                unsafe { host_log(msg.as_ptr(), msg.len()) };
                abi::io_buffer(ptr as u32, abi::IO_CAP as u32)
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn on_location(ptr: *const u8, len: usize) -> i32 {
                match $crate::decode_event::<$crate::Location>(input(ptr, len)) {
                    Some(location) => changed(engine().on_location(&location)),
                    None => abi::REFUSED,
                }
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn on_timer(now: u64) -> i32 {
                changed(engine().on_timer(now))
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn on_event(ptr: *const u8, len: usize) -> i32 {
                // copied out: the engine must not see the buffer change
                let mut event = [0u8; abi::IO_CAP];
                let bytes = input(ptr, len);
                event[..bytes.len()].copy_from_slice(bytes);
                let now = unsafe { host_now() };
                match $crate::raw_event(engine(), now, &event[..bytes.len()]) {
                    Some(c) => changed(c),
                    None => abi::REFUSED,
                }
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn view(out: *mut u8, cap: usize) -> i32 {
                if out.is_null() {
                    return abi::REFUSED;
                }
                let out = unsafe { core::slice::from_raw_parts_mut(out, cap) };
                match $crate::encode_view(engine(), out) {
                    Some(len) => len as i32,
                    None => abi::REFUSED,
                }
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn next_timer() -> u64 {
                engine().next_timer().unwrap_or(abi::NO_TIMER)
            }
        }
    };
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[derive(Default)]
    struct Counter(u32);

    impl Engine for Counter {
        type View = u32;
        type Event = u8;
        fn view(&self) -> u32 {
            self.0
        }
        fn on_location(&mut self, _: &Location) -> bool {
            false
        }
        fn on_timer(&mut self, _: u64) -> bool {
            false
        }
        fn on_event(&mut self, _: u64, by: &u8) -> bool {
            self.0 += *by as u32;
            *by != 0
        }
    }

    #[test]
    fn events_are_exact() {
        let mut c = Counter::default();
        assert_eq!(raw_event(&mut c, 0, &[3]), Some(true));
        assert_eq!(raw_event(&mut c, 0, &[0]), Some(false));
        assert_eq!(raw_event(&mut c, 0, &[]), None);
        assert_eq!(raw_event(&mut c, 0, &[1, 2]), None);
        let mut out = [0u8; 4];
        assert_eq!(encode_view(&c, &mut out), Some(1));
        assert_eq!(out[0], 3);
        assert_eq!(encode_view(&c, &mut out[..0]), None);
    }

    /// The location on the wire: a timestamp varint, then two options of
    /// little-endian f64s.
    #[test]
    fn location_on_the_wire() {
        let l = Location {
            timestamp: 300,
            fix: Some(Fix { lat: 1.0, lon: -1.0 }),
            velocity: None,
        };
        let bytes = postcard::to_stdvec(&l).unwrap();
        assert_eq!(&bytes[..3], &[0xac, 0x02, 1]);
        assert_eq!(bytes.len(), 2 + 1 + 16 + 1);
        assert!(postcard::to_stdvec(&Location {
            timestamp: u64::MAX,
            fix: Some(Fix::default()),
            velocity: Some(Velocity::default())
        })
        .unwrap()
        .len()
            <= abi::IO_CAP);
    }
}
