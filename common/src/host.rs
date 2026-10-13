//! The package host: the registry of installed packages, the selection (the
//! *active* package) and its engine, the device clock and the engine's
//! timer, the GPS track, and the broadcast of every change to every client
//! (README "Host protocol").
//!
//! Transports ([`crate::http`], [`crate::ble`]) only move bytes; everything
//! that touches the selection or an engine goes through [`Host`].

use core::{cell::RefCell, sync::atomic::Ordering};

use embassy_sync::{
    blocking_mutex::{Mutex as BlockingMutex, raw::CriticalSectionRawMutex},
    channel::Channel,
    mutex::Mutex,
    pubsub::{DynSubscriber, Error as PubSubError, PubSubChannel},
};
use embassy_time::{Duration, Instant, with_timeout};
use portable_atomic::AtomicU64;

use extreme_protocol::{
    ASSET_HASH_NOT_CURRENT, ASSET_OK, ASSET_UNKNOWN_PACKAGE, AssetHeader, CHOOSE, ChooseView,
    Envelope, GetAsset, HostEvent, PackageId, PackageInfo, REGISTRY_MAX, STATE_MAX, UiHashes,
    package_id,
};

use crate::{
    ble_settings::{BleCommand, BleStatus, BleStatusView},
    config::{MAX_BLE_CONNECTIONS, MAX_WEB_SOCKETS, TRACK_FIXES},
    engine::{EngineMemory, Evaluated, Event, Rejected, VmEngine},
    fmt::Dbg,
    nmea::GpsUpdate,
    package::Package,
    track::RamTrack,
};

/// The packages a host can run: the built-in Choose (if installed) and the
/// apps, validated bundles (`'static`: in flash on the C6).
pub struct Registry {
    choose: Option<Package>,
    apps: heapless::Vec<Package, { REGISTRY_MAX - 1 }>,
}

/// Why a package was not installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// It has no engine (only Choose may be a page alone).
    Abi,
    /// Its id is not the hash of its name, or is Choose's.
    Id,
    /// The registry is full.
    Full,
}

impl Registry {
    /// `choose` (or none) and the `apps`; an app that cannot be installed
    /// is left out (logged).
    pub fn new(choose: Option<Package>, apps: &[Package]) -> Self {
        let mut registry = Registry {
            choose,
            apps: heapless::Vec::new(),
        };
        for app in apps {
            if let Err(e) = registry.install(*app) {
                error!("host: package {} not installed: {:?}", app.name, Dbg(&e));
            }
        }
        registry
    }

    /// Installs `package`, replacing an installed one with its id.
    pub fn install(&mut self, package: Package) -> Result<(), InstallError> {
        if package.engine.is_none() {
            return Err(InstallError::Abi);
        }
        if package.id == CHOOSE || package.id != package_id(package.name) {
            return Err(InstallError::Id);
        }
        match self.apps.iter_mut().find(|p| p.id == package.id) {
            Some(slot) => *slot = package,
            None => self.apps.push(package).map_err(|_| InstallError::Full)?,
        }
        Ok(())
    }

    /// Removes the app `id`; whether it was installed.
    pub fn remove(&mut self, id: PackageId) -> bool {
        let before = self.apps.len();
        self.apps.retain(|p| p.id != id);
        self.apps.len() != before
    }

    /// The installed package `id` (Choose is 0).
    pub fn get(&self, id: PackageId) -> Option<Package> {
        if id == CHOOSE {
            return self.choose;
        }
        self.apps.iter().copied().find(|p| p.id == id)
    }

    /// Where "back" goes: Choose if installed, else the first (the only)
    /// app; `None` with nothing installed at all.
    pub fn default_package(&self) -> Option<Package> {
        self.choose.or_else(|| self.apps.first().copied())
    }

    /// What the host starts with: the only app when there is just one, or
    /// no Choose (single-app transparency); else Choose.
    pub fn boot_package(&self) -> Option<Package> {
        if self.apps.len() == 1 || self.choose.is_none() {
            self.apps.first().copied().or(self.choose)
        } else {
            self.choose
        }
    }

    /// Choose's list: every app, in order.
    pub fn infos(&self) -> heapless::Vec<PackageInfo, REGISTRY_MAX> {
        self.apps
            .iter()
            .map(|p| PackageInfo {
                id: p.id,
                name: p.manifest().name,
                version: p.version,
            })
            .collect()
    }
}

/// A change for the clients: the active package and its view. Each
/// transport wraps it in an [`Envelope`] stamped with the time of sending
/// ([`Host::encode_envelope`]).
#[derive(Clone, Debug)]
pub struct StateMessage {
    pub active: PackageId,
    /// The active package's name, for logs.
    pub name: &'static str,
    pub ui: UiHashes,
    /// The engine's view, bitwire (its `view.desc`); empty for Choose,
    /// whose view each transport makes for itself (the BLE status only
    /// goes on the websocket).
    pub view: heapless::Vec<u8, STATE_MAX>,
}

/// Which transport an event came over: BLE takes no `Ble` commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    WebSocket,
    Ble,
}

/// Why an event was refused (logged by the transport; BLE answers a write
/// request with Value Not Allowed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not exactly one `HostEvent`.
    Undecodable,
    /// `Select` of a package that is not installed.
    NotInstalled,
    /// An engine event for a package that is not the active one (a client
    /// that has not caught up yet), or for Choose.
    NotActive,
    /// The engine did not take the bytes: not one of its events, outside
    /// its declaration, or its evaluation failed.
    Engine,
    /// A `Ble` command over BLE, or on a host without BLE, or while one is
    /// still queued.
    Ble,
}

/// What the engine timer task is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timer {
    At(u64),
    Cancel,
}

/// The active package, and what its engine runs on.
struct Active {
    /// `None` with nothing installed at all (no engine, no page).
    package: Option<Package>,
    /// `None` for Choose, or if the engine failed to start.
    engine: Option<VmEngine>,
    /// What the timer task was last told to wait for.
    timer: Option<u64>,
    /// The engine's buffers, lent to every call.
    memory: &'static mut EngineMemory,
    /// `gps`: every fix since boot (the newest [`TRACK_FIXES`]), whichever
    /// package is active.
    track: RamTrack<TRACK_FIXES>,
}

impl Active {
    fn id(&self) -> PackageId {
        self.package.map_or(CHOOSE, |p| p.id)
    }

    fn name(&self) -> &'static str {
        self.package.map_or("nothing", |p| p.name)
    }
}

pub struct Host {
    active: Mutex<CriticalSectionRawMutex, Active>,
    registry: BlockingMutex<CriticalSectionRawMutex, RefCell<Registry>>,
    /// Added to uptime to get epoch milliseconds; 0 until the first GPS time.
    tick_offset: AtomicU64,
    /// Timer instructions for [`run_timer`](Self::run_timer). Only ever
    /// published with `active` locked, so they arrive in order; only the
    /// latest matters, hence capacity 1.
    timer_channel: PubSubChannel<CriticalSectionRawMutex, Timer, 1, 1, 1>,
    /// Changes for the connected clients, one subscriber per possible
    /// client on every transport. Each message is complete, so a slow
    /// client that misses one loses nothing.
    broadcast: PubSubChannel<
        CriticalSectionRawMutex,
        StateMessage,
        1,
        { MAX_WEB_SOCKETS + MAX_BLE_CONNECTIONS },
        1,
    >,
    /// The BLE transport's pairing state and bonds, for Choose's view on
    /// the websocket; `None` on a target without BLE.
    ble_status: BlockingMutex<CriticalSectionRawMutex, RefCell<Option<BleStatus>>>,
    /// BLE commands from the web clients to the BLE transport.
    ble_commands: Channel<CriticalSectionRawMutex, BleCommand, 2>,
}

impl Host {
    /// A host running `registry`, starting with its boot package
    /// ([`Registry::boot_package`]); its engines run in `memory`.
    pub fn new(registry: Registry, memory: &'static mut EngineMemory) -> Self {
        let boot = registry.boot_package();
        let host = Host {
            active: Mutex::new(Active {
                package: boot,
                engine: None,
                timer: None,
                memory,
                track: RamTrack::new(),
            }),
            registry: BlockingMutex::new(RefCell::new(registry)),
            tick_offset: AtomicU64::new(0),
            timer_channel: PubSubChannel::new(),
            broadcast: PubSubChannel::new(),
            ble_status: BlockingMutex::new(RefCell::new(None)),
            ble_commands: Channel::new(),
        };
        if let Ok(mut active) = host.active.try_lock() {
            host.start(&mut active, boot);
        }
        host
    }

    /// Current time in epoch milliseconds, the units of GPS timestamps.
    /// Until the first GPS time arrives this is just uptime.
    ///
    /// Known limitation: the clock jumps forward when the first GPS time
    /// arrives, so a timer set before then (its deadline is in uptime
    /// units) fires immediately afterwards. In practice a start sequence is
    /// never begun before the GPS has a fix.
    pub fn now(&self) -> u64 {
        Instant::now().as_millis() + self.tick_offset.load(Ordering::Relaxed)
    }

    // ---- packages ----

    /// The installed package `id`.
    pub fn package(&self, id: PackageId) -> Option<Package> {
        self.registry.lock(|r| r.borrow().get(id))
    }

    /// Where "back" goes ([`Registry::default_package`]); [`CHOOSE`] with
    /// nothing installed.
    pub fn default_id(&self) -> PackageId {
        self.registry
            .lock(|r| r.borrow().default_package())
            .map_or(CHOOSE, |p| p.id)
    }

    /// The answer to a BLE client's [`GetAsset`] (BLE.md "Watch bundles"):
    /// the header, and the bytes to stream after it (in flash, `'static`;
    /// empty unless the status is [`ASSET_OK`]). Only the package's
    /// current watch bundle is served: a request for another hash (an
    /// envelope the client saw before an update) gets
    /// [`ASSET_HASH_NOT_CURRENT`].
    pub fn asset(&self, request: &GetAsset) -> (AssetHeader, &'static [u8]) {
        let refused = |status| (AssetHeader { status, len: 0 }, &[][..]);
        match self.package(request.package) {
            // not installed, or nothing to fetch
            None => refused(ASSET_UNKNOWN_PACKAGE),
            Some(package) if package.watch().is_empty() => refused(ASSET_UNKNOWN_PACKAGE),
            Some(package) if package.ui.watch != request.hash => refused(ASSET_HASH_NOT_CURRENT),
            Some(package) => (
                AssetHeader {
                    status: ASSET_OK,
                    len: package.watch().len() as u32,
                },
                package.watch(),
            ),
        }
    }

    /// Installs (or updates) `package` at run time; the clients' Choose
    /// lists change at once. Updating the active package restarts it.
    pub async fn install(&self, package: Package) -> Result<(), InstallError> {
        let mut active = self.active.lock().await;
        self.registry.lock(|r| r.borrow_mut().install(package))?;
        info!("host: installed {} v{}", package.name, package.version);
        if active.id() == package.id {
            self.start(&mut active, Some(package));
        }
        self.publish(&mut active);
        Ok(())
    }

    /// Removes the app `id`; if it was active, back to the default.
    pub async fn remove(&self, id: PackageId) -> bool {
        let mut active = self.active.lock().await;
        if !self.registry.lock(|r| r.borrow_mut().remove(id)) {
            return false;
        }
        if active.id() == id {
            let default = self.registry.lock(|r| r.borrow().default_package());
            self.start(&mut active, default);
        }
        self.publish(&mut active);
        true
    }

    // ---- inputs ----

    /// A client's message: a postcard [`HostEvent`] (README "Host
    /// protocol").
    pub async fn event(&self, bytes: &[u8], transport: Transport) -> Result<(), Refused> {
        let event = match postcard::take_from_bytes::<HostEvent>(bytes) {
            Ok((event, [])) => event,
            _ => return Err(Refused::Undecodable),
        };
        match event {
            HostEvent::Select(id) => self.select(Some(id)).await,
            HostEvent::SelectDefault => self.select(None).await,
            HostEvent::Engine { package, bytes } => {
                let mut active = self.active.lock().await;
                if package != active.id() || active.engine.is_none() {
                    return Err(Refused::NotActive);
                }
                let result = self.evaluate(&mut active, Event::Client(bytes));
                result.map(|_| ()).map_err(|_| Refused::Engine)
            }
            HostEvent::Ble(command) => {
                if transport == Transport::Ble {
                    return Err(Refused::Ble);
                }
                self.ble_command(command).map_err(|()| Refused::Ble)
            }
        }
    }

    /// Makes `id` active (`None`: the default). The active package again
    /// changes nothing (a fresh engine would lose a race in progress to a
    /// stale tap).
    async fn select(&self, id: Option<PackageId>) -> Result<(), Refused> {
        let mut active = self.active.lock().await;
        let package = self.registry.lock(|r| {
            let r = r.borrow();
            match id {
                Some(id) => r.get(id),
                None => r.default_package(),
            }
        });
        let Some(package) = package else {
            return Err(Refused::NotInstalled);
        };
        if package.id != active.id() {
            self.start(&mut active, Some(package));
        }
        // sent either way: the client that asked learns where things are
        self.publish(&mut active);
        Ok(())
    }

    /// A GPS update: a fix goes on the track (whichever package is
    /// active), then the engine evaluates `{gps: {fix?, velocity?}}`, the
    /// fix being the track's last. The first GPS time also sets the clock
    /// used by [`now`](Self::now).
    pub async fn gps_update(&self, update: GpsUpdate) {
        if let Some(timestamp) = update.timestamp
            && self.tick_offset.load(Ordering::Relaxed) == 0
        {
            let uptime = Instant::now().as_millis();
            self.tick_offset
                .store(timestamp.saturating_sub(uptime), Ordering::Relaxed);
            info!("clock set from GPS: {} ms since epoch", timestamp);
        }
        let mut active = self.active.lock().await;
        let now = self.now() as i64;
        let fix = update.fix.map(|f| (active.track.append(now, f.lat, f.lon), f.lat, f.lon));
        let velocity = update.velocity.map(|v| (v.speed, v.heading));
        if active.engine.is_some() {
            // a rejection is logged by the engine, and changes nothing
            let _ = self.evaluate(&mut active, Event::Gps { fix, velocity });
        }
    }

    // ---- outputs ----

    /// The current state.
    pub async fn current_state(&self) -> StateMessage {
        let active = self.active.lock().await;
        self.capture(&active)
    }

    /// Subscribes to changes. Subscribe before reading
    /// [`current_state`](Self::current_state) so no change is missed.
    pub fn subscribe(&self) -> Result<DynSubscriber<'_, StateMessage>, PubSubError> {
        self.broadcast.dyn_subscriber()
    }

    /// Writes the [`Envelope`] of `message` stamped `timestamp` into `out`;
    /// for Choose its view is made here, with `ble` (the websocket's BLE
    /// status; `None` over BLE). Its length; `Err` if `out` is too small.
    pub fn encode_envelope(
        &self,
        message: &StateMessage,
        timestamp: u64,
        ble: Option<BleStatusView>,
        out: &mut [u8],
    ) -> Result<usize, ()> {
        let mut choose = [0u8; STATE_MAX];
        let state: &[u8] = if message.active == CHOOSE {
            let view = ChooseView {
                packages: self.registry.lock(|r| r.borrow().infos()),
                ble,
            };
            postcard::to_slice(&view, &mut choose).map_err(|_| ())?
        } else {
            &message.view
        };
        let envelope = Envelope {
            timestamp,
            active: message.active,
            ui: message.ui,
            state,
        };
        Ok(postcard::to_slice(&envelope, out).map_err(|_| ())?.len())
    }

    // ---- the engine timer ----

    /// Delivers the engine's wake. Run exactly once, in its own task.
    pub async fn run_timer(&self) -> ! {
        let mut instructions = loop {
            match self.timer_channel.dyn_subscriber() {
                Ok(subscriber) => break subscriber,
                Err(e) => {
                    error!("timer: cannot subscribe ({:?}), retrying", Dbg(&e));
                    embassy_time::Timer::after(Duration::from_secs(10)).await;
                }
            }
        };

        // when the engine wants its wake, epoch ms
        let mut pending: Option<u64> = None;
        loop {
            let Some(at) = pending else {
                pending = next_pending(instructions.next_message_pure().await);
                continue;
            };

            let delay = at.saturating_sub(self.now());
            debug!("timer: due in {} ms", delay);
            match with_timeout(Duration::from_millis(delay), instructions.next_message_pure()).await {
                Ok(instruction) => pending = next_pending(instruction),
                Err(_) => {
                    let mut active = self.active.lock().await;
                    // instructions are published under the lock, so one
                    // that arrived while we waited for it replaces this
                    if let Some(instruction) = instructions.try_next_message_pure() {
                        pending = next_pending(instruction);
                        continue;
                    }
                    pending = None;
                    // stale if the engine has since moved it (or another
                    // package started)
                    if active.timer != Some(at) {
                        continue;
                    }
                    debug!("timer: firing for {}", at);
                    active.timer = None;
                    if active.engine.is_some() {
                        let _ = self.evaluate(&mut active, Event::Wake);
                    }
                }
            }
        }
    }

    // ---- BLE ----

    /// A BLE command from a web client (BLE.md "Pairing a watch"). `Err`
    /// if this target has no BLE transport or the command queue is full.
    pub fn ble_command(&self, command: BleCommand) -> Result<(), ()> {
        if self.ble_status.lock(|s| s.borrow().is_none()) {
            return Err(());
        }
        self.ble_commands.try_send(command).map_err(|_| ())
    }

    /// The next BLE command, for the BLE transport.
    pub async fn next_ble_command(&self) -> BleCommand {
        self.ble_commands.receive().await
    }

    /// Publishes the BLE transport's status (which also marks this target
    /// as having one), and sends every client the current state with it
    /// straight away rather than at the next heartbeat.
    pub async fn set_ble_status(&self, status: BleStatus) {
        self.ble_status.lock(|s| *s.borrow_mut() = Some(status));
        let mut active = self.active.lock().await;
        self.publish(&mut active);
    }

    /// The BLE status for Choose's view on the websocket, as of now (the
    /// pairing window's time left is relative to the moment of sending);
    /// `None` without BLE.
    pub fn ble_status_view(&self) -> Option<BleStatusView> {
        self.ble_status
            .lock(|s| s.borrow().as_ref().map(|s| s.view(Instant::now())))
    }

    // ---- internals, called with `active` locked ----

    /// Makes `package` active with a fresh engine (the old one is dropped
    /// first: they share the buffers) and cancels the old timer. If the
    /// engine does not start the default package takes over.
    fn start(&self, active: &mut Active, package: Option<Package>) {
        active.engine = None;
        active.package = package;
        if active.timer.take().is_some() {
            self.tell_timer(Timer::Cancel);
        }
        let Some(package) = package else {
            info!("host: nothing installed");
            return;
        };
        if package.engine.is_none() {
            info!("host: {} active", package.name);
            return;
        }
        let now = self.now() as i64;
        let started = Instant::now();
        match VmEngine::start(&package, now, active.memory, &active.track) {
            Ok(engine) => {
                let (stack, locals, inputs, outputs) = engine.slots();
                let stats = engine.stats();
                info!(
                    "host: {} v{} active: started in {} us (code + constants {} B; slots: stack {}, locals {}, inputs {}, outputs {}); first evaluation {} instructions, stack high {}",
                    package.name,
                    package.version,
                    started.elapsed().as_micros(),
                    engine.program_bytes(),
                    stack,
                    locals,
                    inputs,
                    outputs,
                    stats.steps,
                    stats.stack_high
                );
                active.engine = Some(engine);
                self.reschedule(active);
            }
            Err(e) => {
                error!("host: {}: engine does not start: {:?}", package.name, Dbg(&e));
                let default = self.registry.lock(|r| r.borrow().default_package());
                if let Some(default) = default
                    && default.id != package.id
                {
                    self.start(active, Some(default));
                }
            }
        }
    }

    /// Evaluates `event` on the active engine (the caller checked there is
    /// one). If it was evaluated: the timer task is told if the wake moved,
    /// and the clients if the view changed. If not, nothing changed.
    fn evaluate(&self, active: &mut Active, event: Event) -> Result<Evaluated, Rejected> {
        let now = self.now() as i64;
        let Active { engine, memory, track, .. } = active;
        let Some(engine) = engine.as_mut() else {
            return Err(Rejected::Undecodable);
        };
        let evaluated = engine.evaluate(memory, track, now, event)?;
        let stats = engine.stats();
        debug!(
            "engine {}: {} instructions, stack high {}",
            active.name(),
            stats.steps,
            stats.stack_high
        );
        self.reschedule(active);
        if evaluated.changed {
            self.publish(active);
        }
        Ok(evaluated)
    }

    /// Tells the timer task if the engine's wake moved.
    fn reschedule(&self, active: &mut Active) {
        let Some(engine) = active.engine.as_ref() else {
            return;
        };
        let next = engine.wake().map(|at| at.max(0) as u64);
        if next != active.timer {
            active.timer = next;
            self.tell_timer(match next {
                Some(at) => Timer::At(at),
                None => Timer::Cancel,
            });
        }
    }

    fn tell_timer(&self, timer: Timer) {
        debug!("timer: {:?}", Dbg(&timer));
        self.timer_channel.immediate_publisher().publish_immediate(timer);
    }

    /// The state as clients see it now.
    fn capture(&self, active: &Active) -> StateMessage {
        let mut view = heapless::Vec::new();
        if let Some(engine) = active.engine.as_ref() {
            // cannot fail: a view is at most STATE_MAX bytes
            view.extend_from_slice(engine.view(active.memory)).unwrap_or(());
        }
        StateMessage {
            active: active.id(),
            name: active.name(),
            ui: active.package.map_or(UiHashes::default(), |p| p.ui),
            view,
        }
    }

    /// Sends every client the current state.
    fn publish(&self, active: &mut Active) {
        let message = self.capture(active);
        self.broadcast.immediate_publisher().publish_immediate(message);
    }
}

/// The pending timer after `instruction`.
fn next_pending(instruction: Timer) -> Option<u64> {
    match instruction {
        Timer::At(at) => Some(at),
        Timer::Cancel => None,
    }
}

#[cfg(test)]
mod tests;
