//! The package host: the registry of installed packages, the selection (the
//! *active* package) and its engine, the device clock and the engine's
//! timer, the GPS track, and the broadcast of every change to every client
//! (README "Host protocol").
//!
//! Transports ([`crate::http`], [`crate::ble`]) only move bytes; everything
//! that touches the selection or an engine goes through [`Host`].

use core::{
    cell::{Cell, RefCell},
    sync::atomic::{AtomicU8, Ordering},
};

use embassy_sync::{
    blocking_mutex::{Mutex as BlockingMutex, raw::CriticalSectionRawMutex},
    channel::Channel,
    mutex::Mutex,
    pubsub::{DynSubscriber, Error as PubSubError, PubSubChannel},
    signal::Signal,
};
use embassy_time::{Duration, Instant, with_timeout};
use portable_atomic::{AtomicU32, AtomicU64};

use extreme_protocol::{
    ASSET_HASH_NOT_CURRENT, ASSET_OK, ASSET_UNKNOWN_PACKAGE, AssetHeader, BundleResponse, CHOOSE, ChooseView,
    Envelope, GetAsset, HostEvent, PackageId, PackageInfo, REGISTRY_MAX, STATE_MAX, UiHashes,
    package_id,
};

use crate::{
    ble_bundles::{BleClient, BundleAnswer, Session},
    ble_settings::{BleCommand, BleStatus, BleStatusView},
    config::{MAX_BLE_CONNECTIONS, MAX_WEB_SOCKETS, TRACK_FIXES},
    engine::{EngineMemory, Evaluated, Event, LIMITS, Rejected, VmEngine},
    firmware::Firmware,
    fmt::{Dbg, Disp, Hex},
    nmea::GpsUpdate,
    package::Package,
    store::{Bundles, Error as StoreError, Space, Uploading},
    track::{RamTrack, Track, TrackFix, TrackLog},
};

/// The packages a host can run: the built-in Choose (if installed) and the
/// apps, validated bundles (`'static`: in flash on the C6).
pub struct Registry {
    choose: Option<Package>,
    apps: heapless::Vec<Package, { REGISTRY_MAX - 1 }>,
    /// The package to boot with, if one was chosen ([`set_boot`](Self::set_boot)).
    boot: Option<PackageId>,
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
    /// Its manifest's hashes aren't its files' (`bundle::hash::check`):
    /// checked when it is uploaded ([`Host::bundle_commit`]).
    Invalid(bundle::Invalid),
}

impl Registry {
    /// `choose` (or none) and the `apps`; an app that cannot be installed
    /// is left out (logged).
    pub fn new(choose: Option<Package>, apps: &[Package]) -> Self {
        let mut registry = Registry {
            choose,
            apps: heapless::Vec::new(),
            boot: None,
        };
        for app in apps {
            if let Err(e) = registry.install(*app) {
                error!("host: package {} not installed: {:?}", app.name, Dbg(&e));
            }
        }
        registry
    }

    /// Whether [`install`](Self::install) would take `package`.
    pub fn check(&self, package: &Package) -> Result<(), InstallError> {
        if package.engine.is_none() {
            return Err(InstallError::Abi);
        }
        if package.id == CHOOSE || package.id != package_id(package.name) {
            return Err(InstallError::Id);
        }
        if self.apps.is_full() && !self.apps.iter().any(|p| p.id == package.id) {
            return Err(InstallError::Full);
        }
        Ok(())
    }

    /// Installs `package`, replacing an installed one with its id.
    pub fn install(&mut self, package: Package) -> Result<(), InstallError> {
        self.check(&package)?;
        match self.apps.iter_mut().find(|p| p.id == package.id) {
            Some(slot) => *slot = package,
            None => self.apps.push(package).map_err(|_| InstallError::Full)?,
        }
        Ok(())
    }

    /// Chooses the package to boot with ([`CHOOSE`] for Choose; `None`:
    /// the rule of [`boot_package`](Self::boot_package)). On the C6 it is
    /// kept in the settings (`KEY_BOOT`) and set from there at boot.
    pub fn set_boot(&mut self, id: Option<PackageId>) {
        self.boot = id;
    }

    /// The package chosen to boot with, if any (installed or not).
    pub fn boot(&self) -> Option<PackageId> {
        self.boot
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

    /// What the host starts with: the package chosen with
    /// [`set_boot`](Self::set_boot) if it is installed; else the only app
    /// when there is just one, or no Choose (single-app transparency); else
    /// Choose.
    pub fn boot_package(&self) -> Option<Package> {
        if let Some(chosen) = self.boot.and_then(|id| self.get(id)) {
            return Some(chosen);
        }
        if self.apps.len() == 1 || self.choose.is_none() {
            self.apps.first().copied().or(self.choose)
        } else {
            self.choose
        }
    }

    /// Every package: Choose (if installed), then the apps in order.
    pub fn all(&self) -> heapless::Vec<Package, REGISTRY_MAX> {
        self.choose.iter().chain(self.apps.iter()).copied().collect()
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

/// Which transport an event came over: BLE takes no `Ble` commands, the
/// websocket no `Bundle` requests.
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
    /// A command its transport doesn't carry: a `Ble` command over BLE, or
    /// on a host without BLE, or while one is still queued; a `Bundle`
    /// request other than through [`Host::ble_event`] (the websocket).
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
    /// `gps`: every fix since boot, whichever package is active.
    track: HostTrack,
}

/// The host's track: the target's (in flash on the C6), or the newest
/// [`TRACK_FIXES`] in RAM.
enum HostTrack {
    Ram(RamTrack<TRACK_FIXES>),
    Given(&'static mut (dyn TrackLog + Send)),
}

impl Track for HostTrack {
    fn len(&self) -> u32 {
        match self {
            HostTrack::Ram(t) => t.len(),
            HostTrack::Given(t) => t.len(),
        }
    }

    fn get(&self, i: u32) -> Option<TrackFix> {
        match self {
            HostTrack::Ram(t) => t.get(i),
            HostTrack::Given(t) => t.get(i),
        }
    }
}

impl TrackLog for HostTrack {
    fn append(&mut self, now: i64, lat: f64, lon: f64) -> i64 {
        match self {
            HostTrack::Ram(t) => t.append(now, lat, lon),
            HostTrack::Given(t) => t.append(now, lat, lon),
        }
    }
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
    pub(crate) registry: BlockingMutex<CriticalSectionRawMutex, RefCell<Registry>>,
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
    /// The bundle store the registry's apps live in; `None` on a host
    /// without one (the registry is then all there is).
    store: Mutex<CriticalSectionRawMutex, Option<&'static mut (dyn Bundles + Send)>>,
    /// Readers of bundle bytes outside the `active` lock ([`lease`](Self::lease)).
    leases: AtomicU32,
    /// The boot package [`activate`](Self::activate) chose last, for the
    /// target to keep ([`next_boot_change`](Self::next_boot_change)).
    boot_changes: Signal<CriticalSectionRawMutex, PackageId>,
    /// The upload in progress as BLE bundle requests see it: its connection
    /// and its last activity ([`crate::ble_bundles`]).
    pub(crate) upload: BlockingMutex<CriticalSectionRawMutex, Cell<Option<Session>>>,
    /// The running firmware; `None` until the target says
    /// ([`set_identity`](Self::set_identity)), and on a host that doesn't.
    identity: BlockingMutex<CriticalSectionRawMutex, Cell<Option<Identity>>>,
    /// Where a firmware update's image goes (the data slot); `None` on a
    /// host that takes no updates.
    firmware: Mutex<CriticalSectionRawMutex, Option<&'static mut (dyn Firmware + Send)>>,
    /// What may be written to the data slot ([`Data`]).
    data: AtomicU8,
    /// What the target is to do for the firmware
    /// ([`next_firmware_action`](Self::next_firmware_action)).
    firmware_actions: Signal<CriticalSectionRawMutex, FirmwareAction>,
    /// The target's answer to a [`FirmwareAction::Switch`]: whether the next
    /// boot runs the new image ([`firmware_switched`](Self::firmware_switched)).
    firmware_done: Signal<CriticalSectionRawMutex, bool>,
}

/// What may be written to the data slot (the store and the track live
/// there, and a firmware update's image goes there).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Data {
    /// Anything: bundle uploads and deletes, the track, an update.
    Writable = 0,
    /// Nothing: the firmware runs on trial after an update
    /// ([`Host::set_pending`]) and the slot holds the image it came from,
    /// which the target boots again if this one fails; until
    /// [`Host::confirm`].
    Unconfirmed = 1,
    /// An update's image is being written over it: the store and the track
    /// are gone until the reboot.
    Updating = 2,
}

/// What the target does for the firmware ([`Host::next_firmware_action`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirmwareAction {
    /// An image with this digest is in the data slot, checked: boot it next
    /// (on trial), answer with [`Host::firmware_switched`], then restart.
    Switch { digest: [u8; 32] },
    /// An update ended without one (refused, aborted, dropped): restart into
    /// the running slot, so the store and the track come back.
    Restart,
}

/// The running firmware, as the target found it at boot
/// ([`Host::set_identity`]): what a `Firmware` request is answered with
/// ([`Host::firmware_info`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The image's appended digest (its last 32 bytes, the SHA-256 of the
    /// rest; `extreme_protocol::firmware`): which firmware this is.
    pub sha256: [u8; 32],
    /// The app slot it runs from: n for `ota_n`.
    pub slot: u8,
    /// The image's length, bytes.
    pub len: u32,
    /// The largest image an update can bring: the data slot's length (0
    /// without one).
    pub max_len: u32,
}

/// A reader of bundle bytes ([`Host::lease`]); dropping it ends the read.
pub struct Lease<'h>(&'h AtomicU32);

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// How often [`Host::bundle_begin`] looks again while a reader holds a
/// lease, and for how long.
const LEASE_POLL: Duration = Duration::from_millis(50);
const LEASE_WAIT: Duration = Duration::from_secs(10);

impl Host {
    /// A host running `registry`, starting with its boot package
    /// ([`Registry::boot_package`]); its engines run in `memory`, with a
    /// track in RAM.
    pub fn new(registry: Registry, memory: &'static mut EngineMemory) -> Self {
        Self::with(registry, memory, HostTrack::Ram(RamTrack::new()), None, None)
    }

    /// [`new`](Self::new), with `track` as the engines' `gps`.
    pub fn with_track(registry: Registry, memory: &'static mut EngineMemory, track: &'static mut (dyn TrackLog + Send)) -> Self {
        Self::with(registry, memory, HostTrack::Given(track), None, None)
    }

    /// [`new`](Self::new), with `track` (if any) as the engines' `gps`,
    /// `store` (if any): the bundle store the registry's apps were read
    /// from ([`BundleStore::open`](crate::store::BundleStore::open)), which
    /// installs and deletes go to, and `firmware` (if any): where an update's
    /// image goes ([`firmware_begin`](Self::firmware_begin)), over the same
    /// flash as the store and the track.
    pub fn with_storage(
        registry: Registry,
        memory: &'static mut EngineMemory,
        track: Option<&'static mut (dyn TrackLog + Send)>,
        store: Option<&'static mut (dyn Bundles + Send)>,
        firmware: Option<&'static mut (dyn Firmware + Send)>,
    ) -> Self {
        let track = match track {
            Some(track) => HostTrack::Given(track),
            None => HostTrack::Ram(RamTrack::new()),
        };
        Self::with(registry, memory, track, store, firmware)
    }

    fn with(
        registry: Registry,
        memory: &'static mut EngineMemory,
        track: HostTrack,
        store: Option<&'static mut (dyn Bundles + Send)>,
        firmware: Option<&'static mut (dyn Firmware + Send)>,
    ) -> Self {
        let boot = registry.boot_package();
        let host = Host {
            active: Mutex::new(Active {
                package: boot,
                engine: None,
                timer: None,
                memory,
                track,
            }),
            registry: BlockingMutex::new(RefCell::new(registry)),
            tick_offset: AtomicU64::new(0),
            timer_channel: PubSubChannel::new(),
            broadcast: PubSubChannel::new(),
            ble_status: BlockingMutex::new(RefCell::new(None)),
            ble_commands: Channel::new(),
            store: Mutex::new(store),
            leases: AtomicU32::new(0),
            boot_changes: Signal::new(),
            upload: BlockingMutex::new(Cell::new(None)),
            identity: BlockingMutex::new(Cell::new(None)),
            firmware: Mutex::new(firmware),
            data: AtomicU8::new(Data::Writable as u8),
            firmware_actions: Signal::new(),
            firmware_done: Signal::new(),
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

    /// Installs (or updates) `package` at run time, in the registry only
    /// (not the store: see [`bundle_commit`](Self::bundle_commit)); the
    /// clients' Choose lists change at once. Updating the active package
    /// restarts it, with its state if the interface is unchanged.
    pub async fn install(&self, package: Package) -> Result<(), InstallError> {
        let mut active = self.active.lock().await;
        self.registry.lock(|r| r.borrow_mut().install(package))?;
        self.installed(&mut active, package);
        Ok(())
    }

    /// Removes the app `id` from the registry (not the store: see
    /// [`bundle_delete`](Self::bundle_delete)); if it was active, back to
    /// the default.
    pub async fn remove(&self, id: PackageId) -> bool {
        let mut active = self.active.lock().await;
        if !self.registry.lock(|r| r.borrow_mut().remove(id)) {
            return false;
        }
        self.removed(&mut active, id);
        true
    }

    /// Makes `id` active (as [`HostEvent::Select`]) and the package to boot
    /// with ([`Registry::set_boot`]; [`CHOOSE`] for Choose), which the
    /// target keeps ([`next_boot_change`](Self::next_boot_change)).
    pub async fn activate(&self, id: PackageId) -> Result<(), Refused> {
        self.select(Some(id)).await?;
        self.registry.lock(|r| r.borrow_mut().set_boot(Some(id)));
        self.boot_changes.signal(id);
        info!("host: {} is the boot package", id);
        Ok(())
    }

    /// The next boot package chosen by [`activate`](Self::activate), for
    /// the target to keep across reboots (only the latest matters).
    pub async fn next_boot_change(&self) -> PackageId {
        self.boot_changes.wait().await
    }

    // ---- the bundle store ----

    /// A reader of bundle bytes outside the `active` lock (an HTTP file
    /// response, a BLE transfer): until it is dropped, the sectors of a
    /// bundle deleted meanwhile are not erased. Taken *before* the package
    /// is looked up, so a package found is either still installed or
    /// covered.
    pub fn lease(&self) -> Lease<'_> {
        self.leases.fetch_add(1, Ordering::AcqRel);
        Lease(&self.leases)
    }

    /// Readers holding a [`lease`](Self::lease) now.
    pub fn leases(&self) -> u32 {
        self.leases.load(Ordering::Acquire)
    }

    /// Starts an upload of bundle `id`, `len` bytes, SHA-256 `sha256`, into
    /// the store ([`Bundles::begin`]). If the sectors it needs held a
    /// bundle deleted since boot that something may still be reading, it
    /// waits for the readers ([`lease`](Self::lease)) to go, looking every
    /// 50 ms; [`StoreError::Busy`] if they are still there after 10 s.
    /// Refused for Choose's id, without a store, for a new id with the
    /// registry full, and ([`StoreError::Busy`]) unless the data slot is
    /// writable (a firmware on trial, or an update under way).
    pub async fn bundle_begin(&self, id: PackageId, len: u32, sha256: &[u8; 32]) -> Result<(), StoreError> {
        if id == CHOOSE {
            return Err(StoreError::Install(InstallError::Id));
        }
        if self.data() != Data::Writable {
            return Err(StoreError::Busy);
        }
        let full = self.registry.lock(|r| {
            let r = r.borrow();
            r.apps.is_full() && r.get(id).is_none()
        });
        if full {
            return Err(StoreError::Install(InstallError::Full));
        }
        let deadline = Instant::now() + LEASE_WAIT;
        loop {
            let begun = match self.store.lock().await.as_deref_mut() {
                Some(store) => store.begin(id, len, sha256, self.leases() == 0),
                None => return Err(StoreError::NotFound),
            };
            match begun {
                Err(StoreError::Leased) if Instant::now() < deadline => embassy_time::Timer::after(LEASE_POLL).await,
                Err(StoreError::Leased) => {
                    warn!("host: upload refused: a deleted bundle's reader held on for {} s", LEASE_WAIT.as_secs());
                    return Err(StoreError::Busy);
                }
                other => return other,
            }
        }
    }

    /// The upload's next bytes ([`Bundles::write`]); the bytes written so
    /// far.
    pub async fn bundle_write(&self, offset: u32, bytes: &[u8]) -> Result<u32, StoreError> {
        match self.store.lock().await.as_deref_mut() {
            Some(store) => store.write(offset, bytes),
            None => Err(StoreError::NoUpload),
        }
    }

    /// Ends the upload ([`Bundles::commit`]): once it is validated, its
    /// manifest's hashes are its files' ([`InstallError::Invalid`]) and the
    /// registry takes it, it is installed, and if it replaces the active
    /// package that restarts (keeping its state if the interface is
    /// unchanged); every client is told.
    pub async fn bundle_commit(&self) -> Result<Package, StoreError> {
        let mut active = self.active.lock().await;
        let mut store = self.store.lock().await;
        let Some(store) = store.as_deref_mut() else {
            return Err(StoreError::NoUpload);
        };
        let accept = &mut |p: &Package| {
            bundle::hash::check(&p.archive, &p.manifest()).map_err(InstallError::Invalid)?;
            self.registry.lock(|r| r.borrow().check(p))
        };
        let package = store.commit(&LIMITS, &mut active.memory.scratch, accept)?;
        // checked, so it can't fail; the store has it either way, for the
        // next boot
        if let Err(e) = self.registry.lock(|r| r.borrow_mut().install(package)) {
            error!("host: {} committed but not installed: {:?}", package.name, Dbg(&e));
        }
        self.installed(&mut active, package);
        Ok(package)
    }

    /// Drops the upload in progress; whether there was one.
    pub async fn bundle_abort(&self) -> bool {
        self.store.lock().await.as_deref_mut().is_some_and(|s| s.abort())
    }

    /// Deletes the bundle `id` from the store and the registry; if it was
    /// active, back to the default. [`StoreError::Busy`] unless the data
    /// slot is writable (as [`bundle_begin`](Self::bundle_begin)).
    pub async fn bundle_delete(&self, id: PackageId) -> Result<(), StoreError> {
        if self.data() != Data::Writable {
            return Err(StoreError::Busy);
        }
        let mut active = self.active.lock().await;
        match self.store.lock().await.as_deref_mut() {
            Some(store) => store.delete(id)?,
            None => return Err(StoreError::NotFound),
        }
        if self.registry.lock(|r| r.borrow_mut().remove(id)) {
            self.removed(&mut active, id);
        }
        info!("host: bundle {} deleted", id);
        Ok(())
    }

    /// The store's free space; `None` without a store.
    pub async fn bundle_space(&self) -> Option<Space> {
        self.store.lock().await.as_deref().map(|s| s.space())
    }

    /// The upload in progress, if any.
    pub async fn bundle_uploading(&self) -> Option<Uploading> {
        self.store.lock().await.as_deref().and_then(|s| s.uploading())
    }

    // ---- the firmware ----

    /// The running firmware, as the target found it at boot.
    pub fn set_identity(&self, identity: Identity) {
        self.identity.lock(|i| i.set(Some(identity)));
    }

    /// The answer to a `Firmware` request: the running firmware; `None` on
    /// a host that hasn't said which ([`set_identity`](Self::set_identity)).
    pub fn firmware_info(&self) -> Option<BundleResponse> {
        let identity = self.identity.lock(|i| i.get())?;
        Some(BundleResponse::Firmware {
            sha256: identity.sha256,
            slot: identity.slot,
            pending: self.pending(),
            max_len: identity.max_len,
        })
    }

    fn data(&self) -> Data {
        match self.data.load(Ordering::Acquire) {
            0 => Data::Writable,
            1 => Data::Unconfirmed,
            _ => Data::Updating,
        }
    }

    /// The firmware runs on trial, just updated: nothing is written to the
    /// data slot (it holds the image the update came from) until
    /// [`confirm`](Self::confirm). Set by the target at boot, before any
    /// client is served.
    pub fn set_pending(&self) {
        self.data.store(Data::Unconfirmed as u8, Ordering::Release);
    }

    /// Whether the firmware runs on trial ([`set_pending`](Self::set_pending)).
    pub fn pending(&self) -> bool {
        self.data() == Data::Unconfirmed
    }

    /// The firmware on trial is confirmed: the data slot is writable again
    /// (the target opened the track there first: [`set_track`](Self::set_track)).
    pub fn confirm(&self) {
        let _ = self.data.compare_exchange(Data::Unconfirmed as u8, Data::Writable as u8, Ordering::AcqRel, Ordering::Acquire);
    }

    /// `track` becomes the engines' `gps` from now on (the fixes so far are
    /// left behind): the target's, in flash, once the data slot may be
    /// written.
    pub async fn set_track(&self, track: &'static mut (dyn TrackLog + Send)) {
        self.active.lock().await.track = HostTrack::Given(track);
    }

    /// Starts a firmware update: an image of `len` bytes whose appended
    /// digest is `digest` ([`Firmware::begin`]). The first one takes the
    /// data slot over for good (until the reboot that follows, whichever
    /// way the update ends): once no reader holds a [`lease`](Self::lease)
    /// (waiting as [`bundle_begin`](Self::bundle_begin) does), every bundle
    /// leaves the registry (the active one gives way to Choose), the track
    /// goes to RAM and the store is dropped; then the image is written over
    /// them. Another begin while that holds starts the image afresh.
    /// [`StoreError::Busy`] while the firmware is on trial, and
    /// [`StoreError::NotFound`] on a host that takes no updates.
    pub async fn firmware_begin(&self, len: u32, digest: &[u8; 32]) -> Result<(), StoreError> {
        match self.data() {
            Data::Unconfirmed => return Err(StoreError::Busy),
            Data::Updating => {
                return match self.firmware.lock().await.as_deref_mut() {
                    Some(sink) => sink.begin(len, digest),
                    None => Err(StoreError::NotFound),
                };
            }
            Data::Writable => {}
        }
        let deadline = Instant::now() + LEASE_WAIT;
        while self.leases() > 0 {
            if Instant::now() >= deadline {
                warn!("host: firmware update refused: a bundle's reader held on for {} s", LEASE_WAIT.as_secs());
                return Err(StoreError::Busy);
            }
            embassy_time::Timer::after(LEASE_POLL).await;
        }
        let mut active = self.active.lock().await;
        let mut firmware = self.firmware.lock().await;
        let Some(sink) = firmware.as_deref_mut() else {
            return Err(StoreError::NotFound);
        };
        // nothing in flash is touched before the image is known to fit
        sink.begin(len, digest)?;
        let removed = self.registry.lock(|r| {
            let mut r = r.borrow_mut();
            let n = r.apps.len();
            r.apps.clear();
            n
        });
        if active.id() != CHOOSE {
            let default = self.registry.lock(|r| r.borrow().default_package());
            self.start(&mut active, default);
        }
        self.publish(&mut active);
        active.track = HostTrack::Ram(RamTrack::new());
        // a bundle upload in progress goes with it
        let _ = self.store.lock().await.take();
        self.data.store(Data::Updating as u8, Ordering::Release);
        info!(
            "host: firmware update begun: {} B, sha256 {}; {} bundles out of the registry, the track in RAM, the store dropped until the reboot",
            len,
            Disp(&Hex(digest)),
            removed
        );
        Ok(())
    }

    /// The image's next bytes ([`Firmware::write`]); the bytes written so far.
    pub async fn firmware_write(&self, offset: u32, bytes: &[u8]) -> Result<u32, StoreError> {
        match self.firmware.lock().await.as_deref_mut() {
            Some(sink) if self.data() == Data::Updating => sink.write(offset, bytes),
            _ => Err(StoreError::NoUpload),
        }
    }

    /// Ends the update ([`Firmware::finish`]): the image checked, the
    /// target is told to boot it ([`FirmwareAction::Switch`]) and answers
    /// whether it will; its digest then, and the target restarts shortly.
    /// [`StoreError::Length`] before every byte is in (the upload goes on);
    /// any other refusal ends it, and the target restarts into the running
    /// firmware ([`FirmwareAction::Restart`]), as it does when it couldn't
    /// switch ([`StoreError::Flash`]).
    pub async fn firmware_commit(&self) -> Result<[u8; 32], StoreError> {
        let finished = match self.firmware.lock().await.as_deref_mut() {
            Some(sink) if self.data() == Data::Updating => sink.finish(),
            _ => return Err(StoreError::NoUpload),
        };
        let digest = match finished {
            Ok(digest) => digest,
            Err(StoreError::Length) => return Err(StoreError::Length),
            Err(e) => {
                warn!("host: firmware image refused: {:?}; restarting", Dbg(&e));
                self.firmware_actions.signal(FirmwareAction::Restart);
                return Err(e);
            }
        };
        self.firmware_done.reset();
        self.firmware_actions.signal(FirmwareAction::Switch { digest });
        if self.firmware_done.wait().await {
            info!("host: firmware {} checked; the next boot runs it", Disp(&Hex(&digest)));
            Ok(digest)
        } else {
            error!("host: firmware {} checked, but the switch to it failed; restarting", Disp(&Hex(&digest)));
            Err(StoreError::Flash)
        }
    }

    /// Drops the update in progress (the target restarts into the running
    /// firmware: [`FirmwareAction::Restart`]); whether there was one.
    pub async fn firmware_abort(&self) -> bool {
        let aborted = match self.firmware.lock().await.as_deref_mut() {
            Some(sink) if self.data() == Data::Updating => sink.abort(),
            _ => false,
        };
        if self.data() == Data::Updating {
            info!("host: firmware update dropped; restarting");
            self.firmware_actions.signal(FirmwareAction::Restart);
        }
        aborted
    }

    /// The firmware update in progress, if any.
    pub async fn firmware_uploading(&self) -> Option<Uploading> {
        if self.data() != Data::Updating {
            return None;
        }
        self.firmware.lock().await.as_deref().and_then(|s| s.uploading())
    }

    /// What the target is to do for the firmware next; only the latest
    /// matters.
    pub async fn next_firmware_action(&self) -> FirmwareAction {
        self.firmware_actions.wait().await
    }

    /// The target's answer to [`FirmwareAction::Switch`]: whether the next
    /// boot runs the new image.
    pub fn firmware_switched(&self, ok: bool) {
        self.firmware_done.signal(ok);
    }

    // ---- inputs ----

    /// A client's message: a postcard [`HostEvent`] (README "Host
    /// protocol"). A `Bundle` request is refused here
    /// ([`Refused::Ble`]): it needs [`ble_event`](Self::ble_event)'s
    /// answer.
    pub async fn event(&self, bytes: &[u8], transport: Transport) -> Result<(), Refused> {
        self.dispatch(bytes, transport, None).await.map(|_| ())
    }

    /// A BLE client's message ([`event`](Self::event)) from `client`; a
    /// `Bundle` request is answered with what to send back
    /// ([`crate::ble_bundles`], BLE.md "Bundles").
    pub async fn ble_event(&self, bytes: &[u8], client: BleClient) -> Result<Option<BundleAnswer<'_>>, Refused> {
        self.dispatch(bytes, Transport::Ble, Some(client)).await
    }

    async fn dispatch(&self, bytes: &[u8], transport: Transport, client: Option<BleClient>) -> Result<Option<BundleAnswer<'_>>, Refused> {
        let event = match postcard::take_from_bytes::<HostEvent>(bytes) {
            Ok((event, [])) => event,
            _ => return Err(Refused::Undecodable),
        };
        let done = match event {
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
            HostEvent::Bundle(request) => {
                let Some(client) = client.filter(|_| transport == Transport::Ble) else {
                    return Err(Refused::Ble);
                };
                return Ok(Some(self.bundle_request(request, client).await));
            }
        };
        done.map(|()| None)
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

    /// When the active engine's `wake` is due (device clock, epoch ms): what
    /// the timer task was last told to wait for; `None` without one.
    pub async fn wake(&self) -> Option<u64> {
        self.active.lock().await.timer
    }

    /// Reads the active package, its engine (`None` for Choose, or one that
    /// did not start) and the engine's memory, changing nothing: for a host
    /// that shows what the engine holds (the studio's, engine/wasm).
    pub async fn inspect<R>(&self, f: impl FnOnce(Option<Package>, Option<&VmEngine>, &EngineMemory) -> R) -> R {
        let active = self.active.lock().await;
        f(active.package, active.engine.as_ref(), &*active.memory)
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

    /// After `package` went into the registry: if it replaces the active
    /// package, that restarts (with its state if the interface is
    /// unchanged); every client is told (the Choose list changed).
    fn installed(&self, active: &mut Active, package: Package) {
        info!("host: installed {} v{}", package.name, package.version);
        if active.id() == package.id {
            let previous = active.engine.take();
            self.start_from(active, Some(package), previous);
        }
        self.publish(active);
    }

    /// After the app `id` left the registry: if it was active, back to the
    /// default; every client is told.
    fn removed(&self, active: &mut Active, id: PackageId) {
        if active.id() == id {
            let default = self.registry.lock(|r| r.borrow().default_package());
            self.start(active, default);
        }
        self.publish(active);
    }

    /// Makes `package` active with a fresh engine (the old one is dropped
    /// first: they share the buffers) and cancels the old timer. If the
    /// engine does not start the default package takes over.
    fn start(&self, active: &mut Active, package: Option<Package>) {
        self.start_from(active, package, None);
    }

    /// [`start`](Self::start); with `previous`, the engine of the package
    /// `package` replaces, its state is kept if the interface is unchanged
    /// ([`VmEngine::resume`]).
    fn start_from(&self, active: &mut Active, package: Option<Package>, previous: Option<VmEngine>) {
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
        let engine = match previous {
            Some(previous) => VmEngine::resume(previous, &package, now, active.memory, &active.track),
            None => VmEngine::start(&package, now, active.memory, &active.track),
        };
        match engine {
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
        let began = embassy_time::Instant::now();
        let evaluated = engine.evaluate(memory, track, now, event)?;
        let took = began.elapsed().as_micros();
        let stats = engine.stats();
        let vm_us = engine.vm_micros();
        debug!(
            "engine {}: {} instructions in {} us ({} us in the VM), stack high {}, {}",
            active.name(),
            stats.steps,
            took,
            vm_us,
            stats.stack_high,
            if evaluated.changed { "view changed" } else { "view unchanged" }
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
