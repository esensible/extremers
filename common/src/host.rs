//! The package host: the registry of installed packages, the selection (the
//! *active* package) and its engine, the device clock and the engine's
//! timer, and the broadcast of every change to every client (README "Host
//! protocol").
//!
//! Transports ([`crate::http`], [`crate::ble`]) only move bytes; everything
//! that touches the selection or an engine goes through [`Host`].

extern crate alloc;

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
    Envelope, GetAsset, HostEvent, Package, PackageId, PackageInfo, REGISTRY_MAX, STATE_MAX,
    UiHashes, package_id,
};
use extreme_sdk::Location;

use crate::{
    ble_settings::{BleCommand, BleStatus, BleStatusView},
    config::{MAX_BLE_CONNECTIONS, MAX_WEB_SOCKETS},
    engine::{Outcome, WasmEngine},
    fmt::Dbg,
    nmea::GpsUpdate,
};

/// The packages a host can run: the built-in Choose (if installed) and the
/// apps. Packages are `'static` (the preinstalled ones are in flash).
pub struct Registry {
    choose: Option<&'static Package>,
    apps: heapless::Vec<&'static Package, { REGISTRY_MAX - 1 }>,
}

/// Why a package was not installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// Its engine speaks another ABI than v1, or it has none.
    Abi,
    /// Its id is not the hash of its name, or is Choose's.
    Id,
    /// The registry is full.
    Full,
}

impl Registry {
    /// `choose` (or none) and the `apps`; an app that cannot be installed
    /// is left out (logged).
    pub fn new(choose: Option<&'static Package>, apps: &'static [Package]) -> Self {
        let mut registry = Registry {
            choose,
            apps: heapless::Vec::new(),
        };
        for app in apps {
            if let Err(e) = registry.install(app) {
                error!("host: package {} not installed: {:?}", app.name, Dbg(&e));
            }
        }
        registry
    }

    /// Installs `package`, replacing an installed one with its id.
    pub fn install(&mut self, package: &'static Package) -> Result<(), InstallError> {
        if package.abi != extreme_sdk::abi::VERSION || package.engine.is_empty() {
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
    pub fn get(&self, id: PackageId) -> Option<&'static Package> {
        if id == CHOOSE {
            return self.choose;
        }
        self.apps.iter().copied().find(|p| p.id == id)
    }

    /// Where "back" goes: Choose if installed, else the first (the only)
    /// app; `None` with nothing installed at all.
    pub fn default_package(&self) -> Option<&'static Package> {
        self.choose.or_else(|| self.apps.first().copied())
    }

    /// What the host starts with: the only app when there is just one, or
    /// no Choose (single-app transparency); else Choose.
    pub fn boot_package(&self) -> Option<&'static Package> {
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
    /// The engine's view, postcard; empty for Choose, whose view each
    /// transport makes for itself (the BLE status only goes on the
    /// websocket).
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
    /// The engine did not take the bytes.
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

/// The active package.
struct Active {
    package: &'static Package,
    /// `None` for Choose, or if the engine failed to start.
    engine: Option<WasmEngine>,
    /// What the timer task was last told to wait for.
    timer: Option<u64>,
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
    /// ([`Registry::boot_package`]).
    pub fn new(registry: Registry) -> Self {
        let boot = registry.boot_package();
        let host = Host {
            active: Mutex::new(Active {
                package: boot.unwrap_or(&NOTHING),
                engine: None,
                timer: None,
            }),
            registry: BlockingMutex::new(RefCell::new(registry)),
            tick_offset: AtomicU64::new(0),
            timer_channel: PubSubChannel::new(),
            broadcast: PubSubChannel::new(),
            ble_status: BlockingMutex::new(RefCell::new(None)),
            ble_commands: Channel::new(),
        };
        if let Ok(mut active) = host.active.try_lock() {
            let package = active.package;
            host.start(&mut active, package);
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
    pub fn package(&self, id: PackageId) -> Option<&'static Package> {
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
            None => refused(ASSET_UNKNOWN_PACKAGE),
            Some(package) if package.ui.watch != request.hash => refused(ASSET_HASH_NOT_CURRENT),
            Some(package) => (
                AssetHeader {
                    status: ASSET_OK,
                    len: package.watch.len() as u32,
                },
                package.watch,
            ),
        }
    }

    /// Installs (or updates) `package` at run time; the clients' Choose
    /// lists change at once. Updating the active package restarts it.
    pub async fn install(&self, package: &'static Package) -> Result<(), InstallError> {
        let mut active = self.active.lock().await;
        self.registry.lock(|r| r.borrow_mut().install(package))?;
        info!("host: installed {} v{}", package.name, package.version);
        if active.package.id == package.id {
            self.start(&mut active, package);
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
        if active.package.id == id {
            let default = self.registry.lock(|r| r.borrow().default_package());
            self.start(&mut active, default.unwrap_or(&NOTHING));
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
                if package != active.package.id {
                    return Err(Refused::NotActive);
                }
                let now = self.now();
                let Some(engine) = active.engine.as_mut() else {
                    return Err(Refused::NotActive);
                };
                match engine.on_event(now, bytes) {
                    Ok(Outcome::Refused) => Err(Refused::Engine),
                    outcome => {
                        self.apply(&mut active, outcome);
                        Ok(())
                    }
                }
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
        if package.id != active.package.id {
            self.start(&mut active, package);
        }
        // sent either way: the client that asked learns where things are
        self.publish(&mut active);
        Ok(())
    }

    /// Feeds a GPS update to the engine. The first GPS time also sets the
    /// clock used by [`now`](Self::now).
    pub async fn gps_update(&self, update: GpsUpdate) {
        let timestamp = match update.timestamp {
            Some(timestamp) => {
                if self.tick_offset.load(Ordering::Relaxed) == 0 {
                    let uptime = Instant::now().as_millis();
                    self.tick_offset
                        .store(timestamp.saturating_sub(uptime), Ordering::Relaxed);
                    info!("clock set from GPS: {} ms since epoch", timestamp);
                }
                timestamp
            }
            None => self.now(),
        };
        let location = Location {
            timestamp,
            fix: update.fix,
            velocity: update.velocity,
        };
        let mut active = self.active.lock().await;
        let now = self.now();
        if let Some(engine) = active.engine.as_mut() {
            let outcome = engine.on_location(now, &location);
            self.apply(&mut active, outcome);
        }
    }

    // ---- outputs ----

    /// The current state.
    pub async fn current_state(&self) -> StateMessage {
        let mut active = self.active.lock().await;
        self.capture(&mut active)
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

    /// Delivers the engine's timer. Run exactly once, in its own task.
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

        // when the engine wants its timer, epoch ms
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
                    let now = self.now();
                    if let Some(engine) = active.engine.as_mut() {
                        let outcome = engine.on_timer(now, at);
                        self.apply(&mut active, outcome);
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
    /// first, so the two are never in memory together) and cancels the old
    /// timer. If the engine does not start the default package takes over.
    fn start(&self, active: &mut Active, package: &'static Package) {
        active.engine = None;
        active.package = package;
        if active.timer.take().is_some() {
            self.tell_timer(Timer::Cancel);
        }
        if package.engine.is_empty() {
            info!("host: {} active", package.name);
            return;
        }
        match WasmEngine::new(package, self.now()) {
            Ok(mut engine) => {
                info!(
                    "host: {} v{} active (engine.wasm {} B, memory {} B)",
                    package.name,
                    package.version,
                    package.engine.len(),
                    engine.memory_size()
                );
                active.engine = Some(engine);
                self.reschedule(active);
            }
            Err(e) => {
                error!("host: {}: engine does not start: {}", package.name, e);
                let default = self.registry.lock(|r| r.borrow().default_package());
                if let Some(default) = default
                    && default.id != package.id
                {
                    self.start(active, default);
                }
            }
        }
    }

    /// Acts on what an engine call said: tells the clients if the view
    /// changed and the timer task if the engine's timer moved. A trap (a
    /// bug in the package) stops the engine and returns to the default.
    fn apply(&self, active: &mut Active, outcome: Result<Outcome, &'static str>) {
        match outcome {
            Ok(Outcome::Changed) => {
                self.reschedule(active);
                self.publish(active);
            }
            Ok(_) => self.reschedule(active),
            Err(e) => {
                error!("host: {}: engine trapped: {}; back to the default", active.package.name, e);
                let default = self.registry.lock(|r| r.borrow().default_package());
                self.start(active, default.unwrap_or(&NOTHING));
                self.publish(active);
            }
        }
    }

    /// Asks the engine for its timer and tells the timer task if it moved.
    fn reschedule(&self, active: &mut Active) {
        let Some(engine) = active.engine.as_mut() else {
            return;
        };
        let next = match engine.next_timer() {
            Ok(next) => next,
            Err(e) => {
                error!("host: {}: next_timer trapped: {}", active.package.name, e);
                None
            }
        };
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
    fn capture(&self, active: &mut Active) -> StateMessage {
        let mut view = heapless::Vec::new();
        let now = self.now();
        if let Some(engine) = active.engine.as_mut() {
            let mut buf = [0u8; STATE_MAX];
            match engine.view(now, &mut buf) {
                // cannot fail: buf is STATE_MAX long
                Ok(len) => view.extend_from_slice(&buf[..len]).unwrap_or(()),
                Err(e) => error!("host: {}: view: {}", active.package.name, e),
            }
        }
        StateMessage {
            active: active.package.id,
            name: active.package.name,
            ui: active.package.ui,
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

/// Active on a host with nothing installed at all: no engine, no page.
static NOTHING: Package = Package {
    id: CHOOSE,
    name: "nothing",
    version: 0,
    abi: 0,
    content_hash: [0; 8],
    ui: UiHashes { web: [0; 8], watch: [0; 8] },
    memory: 0,
    engine: &[],
    web: &[],
    watch: &[],
};

#[cfg(test)]
mod tests;
