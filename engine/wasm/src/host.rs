//! [`HostHandle`]: the real `common::Host` (what the C6 runs) over bundles
//! given as bytes, on a virtual clock ([`crate::clock`]).
//!
//! The host's timer task (`Host::run_timer`) is a future polled here, by
//! hand, after every call: it learns of the engine's new wake at once, and
//! fires it when [`HostHandle::advance`] moves the clock to it. Every change
//! the host broadcasts is kept as the envelope its websocket would have
//! sent (stamped with the device time when it happened), until
//! [`HostHandle::poll`] takes them.

use core::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll, Waker},
};
use std::{sync::Arc, task::Wake};

use common::{
    engine::{EngineMemory, LIMITS},
    host::{Host, Refused, Registry, StateMessage, Transport},
    nmea::{self, Fix, GpsUpdate, NMEAMessage, RingBuffer, Velocity},
    package::Package,
};
use embassy_futures::block_on;
use embassy_sync::pubsub::DynSubscriber;
use extreme_protocol::ENVELOPE_MAX;
use js_sys::Uint8Array;
use serde_json::{Value as Json, json};
use wasm_bindgen::prelude::*;

use crate::clock::{self, Clock};

/// The most polls of the timer task one call makes before giving up: it
/// settles in a few unless an engine keeps asking to be woken now.
const POLLS_MAX: usize = 10_000;

/// A host and what runs it. Fields drop in order: the timer task and the
/// subscription borrow the host; the host borrows the memory and bundles.
#[wasm_bindgen]
pub struct HostHandle {
    running: Option<Running>,
    clock: Clock,
    /// Envelopes not yet taken by [`poll`](Self::poll).
    outbox: Vec<Vec<u8>>,
    /// What [`new`](Self::new) made of each bundle.
    loaded: Json,
}

struct Running {
    timer: Pin<Box<dyn Future<Output = ()>>>,
    updates: DynSubscriber<'static, StateMessage>,
    host: &'static Host,
    woken: Arc<Flag>,
    /// Leaked for the host's lifetime (`'static` there), reclaimed on drop.
    memory: *mut EngineMemory,
    bundles: Vec<*mut [u8]>,
}

/// Set by the timer task's waker.
struct Flag(AtomicBool);

impl Wake for Flag {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Drop for HostHandle {
    fn drop(&mut self) {
        let Some(running) = self.running.take() else { return };
        let Running { timer, updates, host, memory, bundles, .. } = running;
        self.clock.enter(|| {
            drop(timer);
            drop(updates);
            // SAFETY: `host`, `memory` and each bundle came from
            // `Box::into_raw`/`Box::leak` in `new` and nothing borrows them
            // any more: the timer task and the subscription (the only
            // borrowers of the host) are gone, and the host (the only
            // borrower of the memory and the bundles) goes first.
            unsafe {
                drop(Box::from_raw(host as *const Host as *mut Host));
                drop(Box::from_raw(memory));
                for b in bundles {
                    drop(Box::from_raw(b));
                }
            }
        });
    }
}

/// Bytes read from a slice, for the NMEA tokeniser.
struct SliceReader<'a>(&'a [u8]);

impl nmea::AsyncReader for SliceReader<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        if self.0.is_empty() {
            return Err(());
        }
        let n = buf.len().min(self.0.len());
        buf[..n].copy_from_slice(&self.0[..n]);
        self.0 = &self.0[n..];
        Ok(n)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn refused(r: Refused) -> &'static str {
    match r {
        Refused::Undecodable => "Undecodable",
        Refused::NotInstalled => "NotInstalled",
        Refused::NotActive => "NotActive",
        Refused::Engine => "Engine",
        Refused::Ble => "Ble",
    }
}

#[wasm_bindgen]
impl HostHandle {
    /// [`from_bytes`](Self::from_bytes), from JS.
    #[wasm_bindgen(constructor)]
    pub fn new(bundles: Vec<Uint8Array>, choose: Option<Uint8Array>) -> HostHandle {
        HostHandle::from_bytes(bundles.iter().map(Uint8Array::to_vec).collect(), choose.map(|c| c.to_vec()))
    }

    /// [`take_envelopes`](Self::take_envelopes), from JS.
    pub fn poll(&mut self) -> Vec<Uint8Array> {
        self.take_envelopes().iter().map(|e| Uint8Array::from(e.as_slice())).collect()
    }
}

impl HostHandle {
    /// A host with the apps `bundles` and Choose `choose` (if given),
    /// starting as the device does ([`Registry::boot_package`]). A bundle
    /// that doesn't load or install is left out; [`loaded`](Self::loaded)
    /// says which and why. The first envelope is the state it starts in
    /// (what a websocket client gets on connecting).
    pub fn from_bytes(bundles: Vec<Vec<u8>>, choose: Option<Vec<u8>>) -> HostHandle {
        let mut clock = Clock::default();
        let (running, loaded) = clock.enter(|| {
            let mut leaked: Vec<*mut [u8]> = Vec::new();
            let mut scratch = vec![0u16; LIMITS.code_bytes as usize];
            let mut load = |bytes: Vec<u8>| -> Result<Package, String> {
                let raw = Box::into_raw(bytes.into_boxed_slice());
                leaked.push(raw);
                // SAFETY: leaked until the host is dropped (see `Drop`)
                let bytes: &'static [u8] = unsafe { &*raw };
                Package::load(bytes, &LIMITS, &mut scratch).map_err(|e| format!("invalid bundle: {e:?}"))
            };
            let mut report = Vec::new();
            let choose = match choose {
                None => None,
                Some(bytes) => match load(bytes) {
                    Ok(p) if p.engine.is_none() => {
                        report.push(json!({"bundle": "choose", "ok": true, "id": p.id, "name": p.name}));
                        Some(p)
                    }
                    Ok(p) => {
                        report.push(json!({"bundle": "choose", "ok": false, "error": format!("{} has an engine: Choose is a page alone", p.name)}));
                        None
                    }
                    Err(error) => {
                        report.push(json!({"bundle": "choose", "ok": false, "error": error}));
                        None
                    }
                },
            };
            let mut registry = Registry::new(choose, &[]);
            for (i, bytes) in bundles.into_iter().enumerate() {
                let installed = load(bytes).and_then(|p| match registry.install(p) {
                    Ok(()) => Ok(p),
                    Err(e) => Err(format!("{} not installed: {e:?}", p.name)),
                });
                report.push(match installed {
                    Ok(p) => json!({"bundle": i, "ok": true, "id": p.id, "name": p.name}),
                    Err(error) => json!({"bundle": i, "ok": false, "error": error}),
                });
            }
            let memory = Box::into_raw(Box::new(EngineMemory::new()));
            // SAFETY: leaked until the host is dropped (see `Drop`)
            let host: &'static Host = Box::leak(Box::new(Host::new(registry, unsafe { &mut *memory })));
            // The subscription is taken first so nothing published from
            // here on is missed; a host has room for six.
            let updates = host.subscribe().ok();
            let timer: Pin<Box<dyn Future<Output = ()>>> = Box::pin(async move {
                host.run_timer().await;
            });
            let running = updates.map(|updates| Running {
                timer,
                updates,
                host,
                woken: Arc::new(Flag(AtomicBool::new(false))),
                memory,
                bundles: leaked,
            });
            (running, Json::Array(report))
        });
        let mut handle = HostHandle { running, clock, outbox: Vec::new(), loaded };
        // the timer task learns of the first engine's wake; the clients of
        // the state the host started in
        let _ = handle.call(|r, outbox| {
            settle(r, outbox)?;
            let state = block_on(r.host.current_state());
            outbox.push(envelope(r.host, &state)?);
            Ok(())
        });
        handle
    }

    /// The envelopes the host sent since the last call, oldest first.
    pub fn take_envelopes(&mut self) -> Vec<Vec<u8>> {
        core::mem::take(&mut self.outbox)
    }

    /// Runs `f` on the running host with this handle's clock, then lets the
    /// timer task catch up.
    fn call<R>(&mut self, f: impl FnOnce(&mut Running, &mut Vec<Vec<u8>>) -> Result<R, String>) -> Result<R, String> {
        let HostHandle { running, clock, outbox, .. } = self;
        let Some(r) = running.as_mut() else {
            return Err("the host did not start".into());
        };
        clock.enter(|| {
            let out = f(r, outbox)?;
            settle(r, outbox)?;
            Ok(out)
        })
    }

    fn clock_now(&mut self) -> u64 {
        self.clock.enter(clock::now)
    }
}

#[wasm_bindgen]
impl HostHandle {
    /// What the constructor made of each bundle: `[{bundle: "choose" |
    /// index, ok: true, id, name} | {bundle, ok: false, error}]`.
    pub fn loaded(&self) -> String {
        self.loaded.to_string()
    }

    /// A client's message, a postcard `HostEvent` (as from the websocket):
    /// `{ok: true}` or `{ok: false, refused: "Undecodable" | "NotInstalled"
    /// | "NotActive" | "Engine" | "Ble"}`.
    pub fn event(&mut self, bytes: &[u8]) -> String {
        let result = self.call(|r, _| Ok(block_on(r.host.event(bytes, Transport::WebSocket))));
        match result {
            Ok(Ok(())) => json!({"ok": true}),
            Ok(Err(e)) => json!({"ok": false, "refused": refused(e)}),
            Err(error) => json!({"ok": false, "error": error}),
        }
        .to_string()
    }

    /// A GPS update, `{t?, lat?, lon?, speed?, heading?}` (`t` epoch ms, the
    /// receiver's time; `lat` and `lon` together, `speed` (knots) and
    /// `heading` (degrees true) together): `{ok: true}` or `{ok: false,
    /// error}`.
    pub fn gps(&mut self, update: &str) -> String {
        let result = gps_update(update).and_then(|u| self.call(|r, _| Ok(block_on(r.host.gps_update(u)))));
        ok_or_error(result.map(|()| json!({"ok": true})))
    }

    /// NMEA text: each RMC sentence in it with a valid checksum and a valid
    /// fix is a GPS update, as from the receiver: `{ok: true, updates: n}`.
    pub fn nmea(&mut self, text: &str) -> String {
        let result = self.call(|r, _| {
            let mut text = text.as_bytes().to_vec();
            if text.last() != Some(&b'\n') {
                text.push(b'\n');
            }
            let mut tokens = RingBuffer::<_, 128>::new(SliceReader(&text));
            let mut updates = 0;
            // `Err` once the text runs out
            while let Ok(NMEAMessage::GNRMC(rmc)) = block_on(nmea::next_message(&mut tokens)) {
                if let Some(u) = rmc.update() {
                    block_on(r.host.gps_update(u));
                    updates += 1;
                }
            }
            Ok(updates)
        });
        ok_or_error(result.map(|n| json!({"ok": true, "updates": n})))
    }

    /// Moves the clock `ms` forward, stopping at each timer the host has set
    /// on the way so it fires at its time: `{ok: true, now, fired}` (`now`
    /// the device time after, epoch ms; `fired` how many timers came due).
    pub fn advance(&mut self, ms: f64) -> String {
        if !(ms.is_finite() && ms >= 0.0) {
            return json!({"ok": false, "error": format!("advance by {ms} ms: a number of ms, at least 0")}).to_string();
        }
        let target = clock::ticks(ms as u64).saturating_add(self.clock_now());
        let result = self.call(|r, outbox| {
            let mut fired = 0u32;
            while let Some(at) = clock::next_wake().filter(|at| *at <= target) {
                clock::set(at);
                fired += 1;
                settle(r, outbox)?;
            }
            clock::set(target);
            Ok((fired, r.host.now()))
        });
        ok_or_error(result.map(|(fired, now)| json!({"ok": true, "now": now, "fired": fired})))
    }

    /// The current state's envelope, stamped now: what a websocket sends
    /// at each heartbeat. Empty if the host did not start.
    pub fn heartbeat(&mut self) -> Vec<u8> {
        self.call(|r, _| envelope(r.host, &block_on(r.host.current_state()))).unwrap_or_default()
    }

    /// `{active, name, ui: {web, watch}, view_hex, wake, now}`: the active
    /// package (0 Choose), its UI hashes (hex), its view (bitwire, hex;
    /// empty for Choose), when its wake is due and the time now (device
    /// clock, epoch ms; `wake` null without one).
    pub fn state(&mut self) -> String {
        let result = self.call(|r, _| {
            let s = block_on(r.host.current_state());
            Ok(json!({
                "active": s.active,
                "name": s.name,
                "ui": {"web": hex(&s.ui.web), "watch": hex(&s.ui.watch)},
                "view_hex": hex(&s.view),
                "wake": block_on(r.host.wake()),
                "now": r.host.now(),
            }))
        });
        match result {
            Ok(j) => j.to_string(),
            Err(error) => json!({"ok": false, "error": error}).to_string(),
        }
    }

    /// The device time, epoch ms (uptime until a GPS time sets the clock).
    pub fn now(&mut self) -> f64 {
        self.call(|r, _| Ok(r.host.now() as f64)).unwrap_or(0.0)
    }
}

/// Polls the timer task until it waits for something still to come, and
/// keeps what the host broadcast meanwhile.
fn settle(r: &mut Running, outbox: &mut Vec<Vec<u8>>) -> Result<(), String> {
    let waker = Waker::from(r.woken.clone());
    let mut cx = Context::from_waker(&waker);
    for _ in 0..POLLS_MAX {
        r.woken.0.store(false, Ordering::Relaxed);
        // it asks again for the wake it still waits on
        clock::clear_wakes();
        if let Poll::Ready(()) = r.timer.as_mut().poll(&mut cx) {
            return Err("the host's timer task ended".into());
        }
        drain(r, outbox)?;
        let due = clock::next_wake().is_some_and(|at| at <= clock::now());
        if !r.woken.0.load(Ordering::Relaxed) && !due {
            return Ok(());
        }
    }
    Err(format!("the host's timer task did not settle in {POLLS_MAX} polls: an engine waking itself at once, again and again?"))
}

/// What the host broadcast, as envelopes.
fn drain(r: &mut Running, outbox: &mut Vec<Vec<u8>>) -> Result<(), String> {
    while let Some(m) = r.updates.try_next_message_pure() {
        outbox.push(envelope(r.host, &m)?);
    }
    Ok(())
}

/// `message`'s envelope stamped now, as the websocket sends it.
fn envelope(host: &Host, message: &StateMessage) -> Result<Vec<u8>, String> {
    let mut out = [0u8; ENVELOPE_MAX];
    let n = host
        .encode_envelope(message, host.now(), host.ble_status_view(), &mut out)
        .map_err(|()| "an envelope over ENVELOPE_MAX".to_string())?;
    Ok(out[..n].to_vec())
}

fn ok_or_error(result: Result<Json, String>) -> String {
    result.unwrap_or_else(|error| json!({"ok": false, "error": error})).to_string()
}

/// `{t?, lat?, lon?, speed?, heading?}` as a [`GpsUpdate`].
fn gps_update(text: &str) -> Result<GpsUpdate, String> {
    let v: Json = serde_json::from_str(text).map_err(|e| format!("gps: {e}"))?;
    let o = v.as_object().ok_or("gps: an object {t?, lat?, lon?, speed?, heading?}")?;
    if let Some(k) = o.keys().find(|k| !matches!(k.as_str(), "t" | "lat" | "lon" | "speed" | "heading")) {
        return Err(format!("gps: no field `{k}` (t, lat, lon, speed, heading)"));
    }
    let num = |k: &str| -> Result<Option<f64>, String> {
        match o.get(k) {
            None | Some(Json::Null) => Ok(None),
            Some(n) => n.as_f64().filter(|f| f.is_finite()).map(Some).ok_or_else(|| format!("gps: `{k}` is {n}, not a number")),
        }
    };
    let timestamp = match o.get("t") {
        None | Some(Json::Null) => None,
        Some(t) => Some(t.as_u64().ok_or_else(|| format!("gps: `t` is {t}, not epoch ms"))?),
    };
    let fix = match (num("lat")?, num("lon")?) {
        (Some(lat), Some(lon)) => Some(Fix { lat, lon }),
        (None, None) => None,
        _ => return Err("gps: `lat` and `lon` go together".into()),
    };
    let velocity = match (num("speed")?, num("heading")?) {
        (Some(speed), Some(heading)) => Some(Velocity { speed, heading }),
        (None, None) => None,
        _ => return Err("gps: `speed` and `heading` go together".into()),
    };
    Ok(GpsUpdate { timestamp, fix, velocity })
}
