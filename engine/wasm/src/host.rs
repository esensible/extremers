//! [`HostHandle`]: the real `common::Host` (what the C6 runs) over bundles
//! given as bytes, on a virtual clock ([`crate::clock`]), with a `gps` track
//! as long as the C6's ([`crate::track`]).
//!
//! The host's timer task (`Host::run_timer`) is a future polled here, by
//! hand, after every call: it learns of the engine's new wake at once, and
//! fires it when [`HostHandle::advance`] moves the clock to it. Every change
//! the host broadcasts is kept as the envelope its websocket would have
//! sent (stamped with the device time when it happened), until
//! [`HostHandle::poll`] takes them.
//!
//! Every evaluation the host makes is logged, until [`HostHandle::log`]
//! takes them: after each input it is given and after each poll of the
//! timer task (which evaluates at most one wake per poll: an embassy timer
//! always yields once), the handle reads the active engine's count of
//! evaluations and its last outcome (`Host::inspect`, `VmEngine::last`),
//! and the state it holds ([`crate::inspect`]).

use core::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll, Waker},
};
use std::{collections::HashMap, sync::Arc, task::Wake};

use common::{
    engine::{EngineMemory, LIMITS, Rejected},
    host::{Host, Refused, Registry, StateMessage, Transport},
    nmea::{self, Fix, GpsUpdate, NMEAMessage, RingBuffer, Velocity},
    package::Package,
};
use embassy_futures::block_on;
use embassy_sync::pubsub::DynSubscriber;
use extreme_protocol::{ENVELOPE_MAX, PackageId};
use js_sys::Uint8Array;
use serde_json::{Value as Json, json};
use wasm_bindgen::prelude::*;

use crate::{
    clock::{self, Clock},
    inspect::Decoder,
    track::{DEVICE_FIXES, SharedTrack},
};

/// The most polls of the timer task one call makes before giving up: it
/// settles in a few unless an engine keeps asking to be woken now.
const POLLS_MAX: usize = 10_000;

/// A host and what runs it. Fields drop in order: the timer task and the
/// subscription borrow the host; the host borrows the memory, the bundles
/// and the track.
#[wasm_bindgen]
pub struct HostHandle {
    running: Option<Running>,
    clock: Clock,
    /// What the calls left for the caller.
    out: Out,
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
    /// The host's track (leaked, as the memory), and the handle's view of it.
    track_box: *mut SharedTrack,
    track: SharedTrack,
}

/// What the calls leave for the caller, and what the log needs to know.
#[derive(Default)]
struct Out {
    /// Envelopes not yet taken by [`poll`](HostHandle::poll).
    envelopes: Vec<Vec<u8>>,
    /// Evaluations not yet taken by [`log`](HostHandle::log).
    log: Vec<Json>,
    /// The active engine as last seen: its package and its evaluations.
    seen: Option<(PackageId, u32)>,
    /// How to read each package's state.
    decoders: HashMap<PackageId, Result<Decoder, String>>,
}

/// Why the host evaluated: what the log says.
enum Cause {
    /// A GPS update: the event as the engine got it.
    Gps(Json),
    /// The engine's wake, from the timer task.
    Wake,
    /// A client's event.
    Client,
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
        let Running { timer, updates, host, memory, bundles, track_box, .. } = running;
        self.clock.enter(|| {
            drop(timer);
            drop(updates);
            // SAFETY: `host`, `memory`, `track_box` and each bundle came
            // from `Box::into_raw`/`Box::leak` in `new` and nothing borrows
            // them any more: the timer task and the subscription (the only
            // borrowers of the host) are gone, and the host (the only
            // borrower of the memory, the track and the bundles) goes first.
            unsafe {
                drop(Box::from_raw(host as *const Host as *mut Host));
                drop(Box::from_raw(memory));
                drop(Box::from_raw(track_box));
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
    /// [`with_track_capacity`](Self::with_track_capacity), from JS:
    /// `new HostHandle(bundles, choose, trackCapacity?)` (default the C6's,
    /// [`DEVICE_FIXES`]).
    #[wasm_bindgen(constructor)]
    pub fn new(bundles: Vec<Uint8Array>, choose: Option<Uint8Array>, track_capacity: Option<u32>) -> HostHandle {
        HostHandle::with_track_capacity(
            bundles.iter().map(Uint8Array::to_vec).collect(),
            choose.map(|c| c.to_vec()),
            track_capacity.unwrap_or(DEVICE_FIXES),
        )
    }

    /// [`take_envelopes`](Self::take_envelopes), from JS.
    pub fn poll(&mut self) -> Vec<Uint8Array> {
        self.take_envelopes().iter().map(|e| Uint8Array::from(e.as_slice())).collect()
    }
}

impl HostHandle {
    /// [`with_track_capacity`](Self::with_track_capacity) with the C6's
    /// track, [`DEVICE_FIXES`].
    pub fn from_bytes(bundles: Vec<Vec<u8>>, choose: Option<Vec<u8>>) -> HostHandle {
        HostHandle::with_track_capacity(bundles, choose, DEVICE_FIXES)
    }

    /// A host with the apps `bundles` and Choose `choose` (if given),
    /// starting as the device does ([`Registry::boot_package`]), its `gps`
    /// the newest `track_capacity` fixes ([`crate::track`]). A bundle that
    /// doesn't load or install is left out; [`loaded`](Self::loaded) says
    /// which and why. The first envelope is the state it starts in (what a
    /// websocket client gets on connecting); the first log entry the
    /// engine's start.
    pub fn with_track_capacity(bundles: Vec<Vec<u8>>, choose: Option<Vec<u8>>, track_capacity: u32) -> HostHandle {
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
            let track = SharedTrack::new(track_capacity);
            let track_box = Box::into_raw(Box::new(track.clone()));
            // SAFETY: leaked until the host is dropped (see `Drop`)
            let host: &'static Host = Box::leak(Box::new(Host::with_track(registry, unsafe { &mut *memory }, unsafe { &mut *track_box })));
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
                track_box,
                track,
            });
            (running, Json::Array(report))
        });
        let mut handle = HostHandle { running, clock, out: Out::default(), loaded };
        // the engine's start (its first evaluation); the timer task learns
        // of its wake; the clients of the state the host started in
        let _ = handle.call(|r, out| {
            observe(r, out, &Cause::Wake);
            settle(r, out)?;
            let state = block_on(r.host.current_state());
            out.envelopes.push(envelope(r.host, &state)?);
            Ok(())
        });
        handle
    }

    /// The envelopes the host sent since the last call, oldest first.
    pub fn take_envelopes(&mut self) -> Vec<Vec<u8>> {
        core::mem::take(&mut self.out.envelopes)
    }

    /// The evaluations since the last call, oldest first ([`log`](Self::log)).
    pub fn take_log(&mut self) -> Vec<Json> {
        core::mem::take(&mut self.out.log)
    }

    /// Runs `f` on the running host with this handle's clock, then lets the
    /// timer task catch up.
    fn call<R>(&mut self, f: impl FnOnce(&mut Running, &mut Out) -> Result<R, String>) -> Result<R, String> {
        let HostHandle { running, clock, out, .. } = self;
        let Some(r) = running.as_mut() else {
            return Err("the host did not start".into());
        };
        clock.enter(|| {
            let result = f(r, out)?;
            settle(r, out)?;
            Ok(result)
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
        let result = self.call(|r, out| {
            let result = block_on(r.host.event(bytes, Transport::WebSocket));
            observe(r, out, &Cause::Client);
            Ok(result)
        });
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
        let result = gps_update(update).and_then(|u| self.call(|r, out| Ok(gps_to_host(r, out, u))));
        ok_or_error(result.map(|()| json!({"ok": true})))
    }

    /// NMEA text: each RMC sentence in it with a valid checksum and a valid
    /// fix is a GPS update, as from the receiver: `{ok: true, updates: n}`.
    pub fn nmea(&mut self, text: &str) -> String {
        let result = self.call(|r, out| {
            let mut text = text.as_bytes().to_vec();
            if text.last() != Some(&b'\n') {
                text.push(b'\n');
            }
            let mut tokens = RingBuffer::<_, 128>::new(SliceReader(&text));
            let mut updates = 0;
            // `Err` once the text runs out
            while let Ok(NMEAMessage::GNRMC(rmc)) = block_on(nmea::next_message(&mut tokens)) {
                if let Some(u) = rmc.update() {
                    gps_to_host(r, out, u);
                    // a wake the clock jump made due fires before the next
                    settle(r, out)?;
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
        let result = self.call(|r, out| {
            let mut fired = 0u32;
            while let Some(at) = clock::next_wake().filter(|at| *at <= target) {
                clock::set(at);
                fired += 1;
                settle(r, out)?;
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

    /// `{active, name, ui: {web, watch}, view_hex, wake, now, running,
    /// evaluations, track, track_capacity}`: the active package (0 Choose),
    /// its UI hashes (hex), its view (bitwire, hex; empty for Choose), when
    /// its wake is due and the time now (device clock, epoch ms; `wake` null
    /// without one); whether its engine runs (false for Choose, or an engine
    /// whose first evaluation failed) and how many evaluations it has made;
    /// the fixes in `gps` and the most it holds.
    pub fn state(&mut self) -> String {
        let result = self.call(|r, _| {
            let s = block_on(r.host.current_state());
            let evaluations = block_on(r.host.inspect(|_, engine, _| engine.map(|e| e.evaluations())));
            let track = r.track.lock();
            Ok(json!({
                "active": s.active,
                "name": s.name,
                "ui": {"web": hex(&s.ui.web), "watch": hex(&s.ui.watch)},
                "view_hex": hex(&s.view),
                "wake": block_on(r.host.wake()),
                "now": r.host.now(),
                "running": evaluations.is_some(),
                "evaluations": evaluations.unwrap_or(0),
                "track": track.len(),
                "track_capacity": track.capacity(),
            }))
        });
        match result {
            Ok(j) => j.to_string(),
            Err(error) => json!({"ok": false, "error": error}).to_string(),
        }
    }

    /// The active engine's state, decoded from the leaves it keeps with the
    /// state schema in its bundle's `source.json`: `{ok: true, state}`
    /// (`state` null before the first evaluation, or for Choose) or `{ok:
    /// false, error}` (no engine running, or no `source.json`).
    pub fn state_json(&mut self) -> String {
        let result = self.call(|r, out| {
            let Out { decoders, .. } = out;
            block_on(r.host.inspect(|package, engine, memory| {
                let (Some(package), Some(engine)) = (package, engine) else {
                    return Err("no engine is running (Choose is active, or the engine's first evaluation failed)".to_string());
                };
                let decoder = decoders.entry(package.id).or_insert_with(|| Decoder::of(&package));
                let decoder = decoder.as_ref().map_err(|e| format!("{}: {e}", package.name))?;
                Ok(engine.state(memory).map_or(Json::Null, |leaves| decoder.state(leaves, &|id| engine.state_list(memory, id))))
            }))
        });
        ok_or_error(result.map(|state| json!({"ok": true, "state": state})))
    }

    /// The evaluations the host made since the last call, oldest first, as
    /// JSON: `[{t, cause: "start" | "gps" | "wake" | "client", event?,
    /// accepted, changed, error?: {code, pc, span: {start, end} | null,
    /// help}, instructions?, stack_high?, state, view_hex, wake,
    /// unobserved?}]`. `t` the device time it was made at; `event` the
    /// engine's event (not for "client": the client's bytes are the
    /// caller's); `changed` whether the published value's bytes changed
    /// (an envelope went out); `error.code` an `R-…` run-time error (`pc`
    /// the instruction, `span` its bytes in engine.expr), or "undecodable"
    /// (a client's bytes that are not one of the engine's events) or
    /// "output" (an output that can't be published); `state` (plain JSON,
    /// null if it can't be read), `view_hex` and `wake` as after it.
    /// `unobserved: n`: n evaluations before this one were made between two
    /// looks (not expected; this one is the last of them).
    pub fn log(&mut self) -> String {
        Json::Array(self.take_log()).to_string()
    }

    /// The device time, epoch ms (uptime until a GPS time sets the clock).
    pub fn now(&mut self) -> f64 {
        self.call(|r, _| Ok(r.host.now() as f64)).unwrap_or(0.0)
    }
}

/// Polls the timer task until it waits for something still to come, and
/// keeps what the host broadcast meanwhile and the wakes it evaluated (at
/// most one per poll: the task waits on an embassy timer, which yields once
/// even when already due).
fn settle(r: &mut Running, out: &mut Out) -> Result<(), String> {
    let waker = Waker::from(r.woken.clone());
    let mut cx = Context::from_waker(&waker);
    for _ in 0..POLLS_MAX {
        r.woken.0.store(false, Ordering::Relaxed);
        // it asks again for the wake it still waits on
        clock::clear_wakes();
        if let Poll::Ready(()) = r.timer.as_mut().poll(&mut cx) {
            return Err("the host's timer task ended".into());
        }
        drain(r, out)?;
        observe(r, out, &Cause::Wake);
        let due = clock::next_wake().is_some_and(|at| at <= clock::now());
        if !r.woken.0.load(Ordering::Relaxed) && !due {
            return Ok(());
        }
    }
    Err(format!("the host's timer task did not settle in {POLLS_MAX} polls: an engine waking itself at once, again and again?"))
}

/// What the host broadcast, as envelopes.
fn drain(r: &mut Running, out: &mut Out) -> Result<(), String> {
    while let Some(m) = r.updates.try_next_message_pure() {
        out.envelopes.push(envelope(r.host, &m)?);
    }
    Ok(())
}

/// A GPS update to the host, logged with the event the engine got: the fix
/// as it went on the track (its `t` the track's), the velocity.
fn gps_to_host(r: &mut Running, out: &mut Out, update: GpsUpdate) {
    let fix = update.fix.map(|f| (f.lat, f.lon));
    let velocity = update.velocity.map(|v| (v.speed, v.heading));
    block_on(r.host.gps_update(update));
    let mut gps = serde_json::Map::new();
    if let Some((lat, lon)) = fix {
        let t = r.track.lock().last().map(|f| f.t);
        gps.insert("fix".into(), json!({"t": t, "lat": lat, "lon": lon}));
    }
    if let Some((speed, heading)) = velocity {
        gps.insert("velocity".into(), json!({"speed": speed, "heading": heading}));
    }
    observe(r, out, &Cause::Gps(json!({"gps": gps})));
}

/// Logs the evaluations the active engine made since the last look:
/// `cause`'s, except a new engine's first, which is its start.
fn observe(r: &Running, out: &mut Out, cause: &Cause) {
    let now = r.host.now();
    let Out { log, seen, decoders, .. } = out;
    let before = *seen;
    *seen = block_on(r.host.inspect(|package, engine, memory| {
        let (Some(package), Some(engine)) = (package, engine) else {
            return None;
        };
        let count = engine.evaluations();
        // a new engine: another package, or the same one started again
        let (fresh, from) = match before {
            Some((id, n)) if id == package.id && n <= count => (false, n),
            _ => (true, 0),
        };
        if count == from {
            return Some((package.id, count));
        }
        let decoder = decoders.entry(package.id).or_insert_with(|| Decoder::of(&package));
        let state = match (decoder.as_ref(), engine.state(memory)) {
            (Ok(d), Some(leaves)) => d.state(leaves, &|id| engine.state_list(memory, id)),
            _ => Json::Null,
        };
        let after = json!({
            "state": state,
            "view_hex": hex(engine.view(memory)),
            "wake": engine.wake().map(|w| w.max(0)),
        });
        let entry = |cause: &str, event: Option<Json>, outcome: Option<Result<common::engine::Evaluated, Rejected>>| {
            let mut e = json!({"t": now, "cause": cause});
            if let Some(event) = event {
                e["event"] = event;
            }
            match outcome {
                Some(Ok(evaluated)) => {
                    let stats = engine.stats();
                    e["accepted"] = json!(true);
                    e["changed"] = json!(evaluated.changed);
                    e["instructions"] = json!(stats.steps);
                    e["stack_high"] = json!(stats.stack_high);
                }
                Some(Err(rejected)) => {
                    e["accepted"] = json!(false);
                    e["changed"] = json!(false);
                    e["error"] = rejection(rejected, decoder.as_ref().ok());
                }
                // a start the engine made before this look's last one: it
                // started, so it was accepted, and its view was the first
                None => {
                    e["accepted"] = json!(true);
                    e["changed"] = json!(true);
                }
            }
            for (k, v) in after.as_object().into_iter().flatten() {
                e[k] = v.clone();
            }
            e
        };
        let mut rest = count - from;
        if fresh {
            rest -= 1;
            let outcome = if rest == 0 { engine.last() } else { None };
            log.push(entry("start", Some(json!({"gps": {}})), outcome));
        }
        if rest > 0 {
            let (name, event) = match cause {
                Cause::Gps(event) => ("gps", Some(event.clone())),
                Cause::Wake => ("wake", Some(json!({"wake": {}}))),
                Cause::Client => ("client", None),
            };
            let mut e = entry(name, event, engine.last());
            if rest > 1 {
                e["unobserved"] = json!(rest - 1);
            }
            log.push(e);
        }
        Some((package.id, count))
    }));
}

/// A rejected evaluation, for the log.
fn rejection(rejected: Rejected, decoder: Option<&Decoder>) -> Json {
    match rejected {
        Rejected::Failed(f) => {
            let code = f.code.as_str();
            json!({
                "code": code,
                "pc": f.pc,
                "span": decoder.and_then(|d| d.span(f.pc)).map(|(start, end)| json!({"start": start, "end": end})),
                "help": compiler::diag::runtime_help(code),
            })
        }
        Rejected::Undecodable => json!({
            "code": "undecodable",
            "pc": null,
            "span": null,
            "help": "the client's bytes are not one of the engine's events (they do not decode with its event.desc)",
        }),
        Rejected::Output => json!({
            "code": "output",
            "pc": null,
            "span": null,
            "help": "the output can't be published: the view does not encode with view.desc, or the state holds a string from outside the bundle",
        }),
    }
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
