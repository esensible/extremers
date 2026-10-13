#![no_std]
#![no_main]

use static_cell::{ConstStaticCell, StaticCell};

use embassy_executor::Spawner;
use embassy_net::{Ipv4Cidr, Stack, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};

use edge_nal_embassy::{Tcp, TcpBuffers};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    Async,
    clock::CpuClock,
    gpio::{Level, Output, OutputConfig},
    ram,
    rng::Rng,
    timer::timg::TimerGroup,
    uart::{Config as UartConfig, RxConfig, Uart, UartRx},
};
use esp_radio::{
    ble::controller::BleConnector,
    wifi::{
        ControllerConfig, CountryInfo, Interface, OperatingClass, PowerSaveMode, WifiController,
    },
};
use trouble_host::prelude::ExternalController;

esp_bootloader_esp_idf::esp_app_desc!();

mod memory;
mod network_tasks;
#[cfg(feature = "selftest")]
mod selftest;
#[allow(dead_code)] // shared with src/bin/{provision,dump}.rs; each uses a subset
mod storage;
#[allow(dead_code)] // shared with src/bin/dump.rs
mod xip;

use crate::network_tasks::{
    access_point_config, choose_access_point_credentials, dhcp_task, dns_task, net_task,
    wifi_task,
};
use common::{
    ble::{BondStore, CONNECTIONS_MAX, Diagnostics, serve_ble},
    ble_settings::{BondRecord, GattClientRecord, static_random_address},
    config::{
        AP_IP, AP_PREFIX_LEN, GPS_BAUD, GPS_MAX_ITEMS, HTTP_PORT, MAX_BLE_BONDS, MAX_WEB_SOCKETS,
        SOCKET_BUFFER_SIZE,
    },
    engine::{EngineMemory, LIMITS},
    host::{Host, Registry},
    nmea::AsyncReader,
    package::Package,
    store::{self, Apps, BundleStore, Bundles, Item},
    tasks::{read_gps, serve_http},
    track::{FlashTrack, TrackFlash, TrackLog},
};
use esp_storage::FlashStorage;

/// The engines' buffers (common::engine), static: not the heap, not the
/// main stack.
static ENGINE_MEMORY: ConstStaticCell<EngineMemory> = ConstStaticCell::new(EngineMemory::new());

/// The BLE controller, with 20 HCI command slots.
type BleController = ExternalController<BleConnector<'static>, 20>;

/// Access point transmit power ceiling, 0.25 dBm units: 60 = 15 dBm.
const WIFI_TX_POWER_QDBM: i8 = 60;

// Every log line carries the uptime (probe-rs shows it as seconds with ms).
// esp-hal's clock, not embassy-time's, so it works before esp_rtos::start.
defmt::timestamp!(
    "{=u64:ms}",
    esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_millis()
);

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // first, so the high-water mark covers everything after it
    memory::paint_main_stack();
    // Logs go over RTT as defmt, read with probe-rs (`probe-rs run` or
    // `probe-rs attach`). 4 KiB covers boot when attaching late.
    rtt_target::rtt_init_defmt!(rtt_target::ChannelMode::NoBlockSkip, 4096);

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // esp-radio needs a heap; WiFi + BLE coexistence wants the larger
    // second one the esp-hal coex example uses
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 64 * 1024);

    // start the scheduler (also provides the embassy time driver)
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // XIAO ESP32-C6 RF switch (Seeed wiki, "Getting Started"): GPIO3 LOW
    // powers the switch and GPIO14 LOW selects the onboard ceramic antenna
    // (HIGH = external U.FL). esp-hal and esp-radio know nothing about it;
    // with GPIO3 floating the antenna path is effectively unpowered and the
    // access point is barely visible (seen on hilux's XIAO C6). Held for the
    // life of main, before WiFi starts.
    let _rf_switch_power = Output::new(peripherals.GPIO3, Level::Low, OutputConfig::default());
    Timer::after(Duration::from_millis(100)).await;
    let _rf_antenna_select = Output::new(peripherals.GPIO14, Level::Low, OutputConfig::default());
    defmt::info!("rf switch: powered (GPIO3 low), onboard antenna (GPIO14 low)");

    // Two more handles on the flash, for the data slot's regions (xip.rs):
    // each its own FlashStorage, over flash disjoint from the settings
    // partition's and from each other's.
    // SAFETY: esp-storage runs every operation in its own critical section,
    // so they never interleave; none assumes it is the only writer of
    // anything outside its own region.
    // Made first, before any flash operation of the app's: each handle's
    // size comes from the chip's ID (esp-storage reads it when a handle is
    // made), and the data slot's handles once read a size too small for it
    // when made after the settings had been read (`OutOfBounds`). The
    // repo's esp-storage now keeps the first size detected for all.
    let (tracks_flash, bundles_flash) = unsafe {
        (
            FlashStorage::new(peripherals.FLASH.clone_unchecked()),
            FlashStorage::new(peripherals.FLASH.clone_unchecked()),
        )
    };
    defmt::info!(
        "flash: capacity {=usize:#x} (tracks handle), {=usize:#x} (bundles handle)",
        embedded_storage::nor_flash::ReadNorFlash::capacity(&tracks_flash),
        embedded_storage::nor_flash::ReadNorFlash::capacity(&bundles_flash)
    );
    // Settings partition (read only here; see storage.rs), then the access
    // point's credentials from it, or the compiled-in defaults.
    storage::init(peripherals.FLASH).await;
    choose_access_point_credentials().await;
    let (tracks, bundles) = match data_regions(tracks_flash, bundles_flash) {
        Some((tracks, bundles)) => (Some(tracks), Some(bundles)),
        None => {
            defmt::warn!("data slot unusable: the track in RAM (lost at a reboot), no bundles but Choose");
            (None, None)
        }
    };
    let track = tracks.and_then(flash_track);

    memory::log("before the host");
    let engine_memory = ENGINE_MEMORY.take();
    let choose = load("choose", extreme_packages::CHOOSE, engine_memory);
    let (store, apps) = match bundles {
        Some(bundles) => {
            let (store, apps) = bundle_store(bundles, engine_memory);
            (Some(store), apps)
        }
        None => (None, Default::default()),
    };
    let mut registry = Registry::new(choose, &apps);
    let boot = storage::boot_id().await;
    registry.set_boot(boot);
    defmt::info!(
        "boot package: {=str}; starting {=str}",
        match boot {
            None => "none kept (the rule: the only app, else Choose)",
            Some(0) => "Choose",
            Some(_) => "a bundle",
        },
        registry.boot_package().map_or("nothing", |p| p.name)
    );
    if let Some(id) = boot {
        defmt::info!("boot package id {=u32:#010x}{=str}", id, if registry.get(id).is_some() { "" } else { " (not installed)" });
    }
    static HOST: StaticCell<Host> = StaticCell::new();
    let runtime: &'static Host = HOST.init(Host::with_storage(
        registry,
        engine_memory,
        track.map(|t| t as &'static mut (dyn TrackLog + Send)),
        store.map(|s| s as &'static mut (dyn Bundles + Send)),
    ));
    memory::log("host up");
    spawner.spawn(boot_package_task(runtime).unwrap());

    // BLE (BLE.md) shares the radio with WiFi (esp-radio `coex`); the
    // controller's connection limit matches the host's.
    let ble_config = esp_radio::ble::Config::default().with_max_connections(CONNECTIONS_MAX as u16);
    let connector = BleConnector::new(peripherals.BT, ble_config).unwrap();
    spawner.spawn(ble_task(ExternalController::new(connector), runtime, ble_address()).unwrap());
    #[cfg(feature = "dev-pairing")]
    spawner.spawn(dev_pairing::task(runtime).unwrap());

    // initialize wifi controller as an access point
    let controller_config = ControllerConfig::default()
        .with_country_info(
            CountryInfo::from(*b"AU").with_operating_class(OperatingClass::Repr(0x21)),
        )
        .with_initial_config(access_point_config());
    let mut controller = WifiController::new(peripherals.WIFI, controller_config).unwrap();
    controller.set_power_saving(PowerSaveMode::None).unwrap();
    // esp-radio starts the radio at 5 dBm (its documented default; ESP-IDF's
    // is 20 dBm), which a Kindle sees as a weak network. 15 dBm, under the
    // ~16 dBm esp-radio warns has broken authentication on some hardware.
    match controller.set_max_tx_power(WIFI_TX_POWER_QDBM) {
        Ok(()) => defmt::info!(
            "wifi: max tx power {=i8} (0.25 dBm units)",
            WIFI_TX_POWER_QDBM
        ),
        Err(e) => defmt::warn!(
            "wifi: set_max_tx_power failed: {:?}; still at the 5 dBm default",
            defmt::Debug2Format(&e)
        ),
    }
    let device = Interface::access_point();

    let config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(AP_IP, AP_PREFIX_LEN),
        gateway: Some(AP_IP),
        dns_servers: [AP_IP].into_iter().collect(),
    });

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    static RESOURCES: StaticCell<StackResources<{ MAX_WEB_SOCKETS + 5 }>> = StaticCell::new();
    let (stack, runner) =
        embassy_net::new(device, config, RESOURCES.init(StackResources::new()), seed);

    spawner.spawn(wifi_task(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());
    spawner.spawn(dhcp_task(stack).unwrap());
    spawner.spawn(dns_task(stack).unwrap());

    spawner.spawn(httpd_task(stack, runtime).unwrap());
    spawner.spawn(timer_task(runtime).unwrap());

    // The GPS's EN pin (Adafruit Ultimate GPS v2) is wired to D0 = GPIO0:
    // high = GPS on, low = GPS off (for power saving later). Held for the
    // life of the program.
    let _gps_enable = Output::new(peripherals.GPIO0, Level::High, OutputConfig::default());

    let (tx_pin, rx_pin) = (peripherals.GPIO16, peripherals.GPIO17);
    let config = UartConfig::default()
        .with_baudrate(GPS_BAUD)
        .with_rx(RxConfig::default());
    let uart0 = Uart::new(peripherals.UART0, config)
        .unwrap()
        .with_tx(tx_pin)
        .with_rx(rx_pin)
        .into_async();
    let (uart_rx, _uart_tx) = uart0.split();
    spawner.spawn(gps_task(uart_rx, runtime).unwrap());

    memory::log("boot, all tasks spawned");
    spawner.spawn(memory::memory_task().unwrap());
    #[cfg(feature = "selftest")]
    spawner.spawn(selftest::selftest_task(runtime).unwrap());

    // idle loop, blink LED
    let mut led = Output::new(peripherals.GPIO15, Level::Low, OutputConfig::default());
    led.set_high();
    loop {
        Timer::after(Duration::from_millis(100)).await;
        led.toggle();
    }
}

type Tracks = FlashTrack<xip::XipFlash>;
type Store = BundleStore<xip::XipFlash>;

/// The data slot's two regions (storage.rs, xip.rs), tracks and bundles,
/// each written through its own `flash` handle and read in place: maps the
/// slot and checks that reads in place see what the flash holds. `None`
/// (logged) if any of that fails: the host then keeps the track in RAM,
/// gone at a reboot, and has no bundles but Choose.
fn data_regions(tracks: FlashStorage<'static>, bundles: FlashStorage<'static>) -> Option<(xip::XipFlash, xip::XipFlash)> {
    let Some(slot) = storage::data_slot() else {
        defmt::warn!("data slot: none (see the storage lines above)");
        return None;
    };
    if !xip::map(&slot) {
        return None;
    }
    let (Some(mut tracks), Some(bundles)) = (
        xip::XipFlash::new(tracks, slot.tracks()),
        xip::XipFlash::new(bundles, slot.bundles()),
    ) else {
        defmt::warn!("data slot: a region is not within the mapping");
        return None;
    };
    tracks.self_check(&slot).then_some((tracks, bundles))
}

/// The bundle store in the data slot's bundles region (common::store):
/// its boot scan, every bundle in it logged, and the ones that validate
/// (in `memory`'s scratch) for the registry.
fn bundle_store(region: xip::XipFlash, memory: &mut EngineMemory) -> (&'static mut Store, Apps) {
    let bytes = region.bytes();
    let started = esp_hal::time::Instant::now();
    let (store, apps) = BundleStore::open(region, &LIMITS, &mut memory.scratch);
    let us = started.elapsed().as_micros();
    let space = store.space();
    let extents = store.entries().filter(|i| matches!(i, Item::Extent { .. })).count();
    defmt::info!(
        "store: bundles region at {=usize:#010x}, {=usize} B: {=usize} extents, {=usize} bundles validated in {=u64} us; {=u32} B free, the largest bundle that fits {=u32} B",
        bytes.as_ptr() as usize,
        bytes.len(),
        extents,
        apps.len(),
        us,
        space.free_bytes,
        space.largest_free
    );
    for item in store.entries() {
        let Item::Extent { first, record: r } = item else {
            continue;
        };
        let state = store::state_name(r.state);
        // the package, if it is this extent's and it validated
        let at = bytes.as_ptr() as usize + store::archive_offset(first);
        match apps.iter().find(|p| p.archive.bytes().as_ptr() as usize == at) {
            Some(p) => defmt::info!(
                "store: sectors {=u16}+{=u16}: {=str}, seq {=u32}, id {=u32:#010x}: {=str} v{=u16}, {=u32} B",
                first,
                r.sectors,
                state,
                r.seq,
                r.id,
                p.name,
                p.version,
                r.len
            ),
            None => defmt::info!(
                "store: sectors {=u16}+{=u16}: {=str}, seq {=u32}, id {=u32:#010x}, {=u32} B{=str}",
                first,
                r.sectors,
                state,
                r.seq,
                r.id,
                r.len,
                if r.state == store::COMMITTED { " (not loaded: see above)" } else { "" }
            ),
        }
    }
    static STORE: StaticCell<Store> = StaticCell::new();
    (STORE.init(store), apps)
}

/// Keeps the boot package `Host::activate` chooses in the settings.
#[embassy_executor::task]
async fn boot_package_task(runtime: &'static Host) -> ! {
    loop {
        let id = runtime.next_boot_change().await;
        if storage::store_boot_id(id).await {
            defmt::info!("settings: boot package {=u32:#010x} kept", id);
        } else {
            defmt::warn!("settings: boot package {=u32:#010x} not kept", id);
        }
    }
}

/// The engines' `gps` in the data slot's tracks region (common::track,
/// xip.rs): opens the ring (a new run from its head). `None` (logged) if
/// that fails: the host then keeps the track in RAM, gone at a reboot.
fn flash_track(region: xip::XipFlash) -> Option<&'static mut Tracks> {
    let fallback = "track: in RAM instead (lost at a reboot)";
    let range = region.range();
    let started = esp_hal::time::Instant::now();
    let track = match FlashTrack::open(region, GPS_MAX_ITEMS) {
        Ok(track) => track,
        Err(e) => {
            defmt::error!("track: opening the ring failed: {:?}; {=str}", defmt::Debug2Format(&e), fallback);
            return None;
        }
    };
    let us = started.elapsed().as_micros();
    let scan = track.boot_scan();
    defmt::info!(
        "track: flash {=u32:#08x}..{=u32:#08x}, {=u32} records; scan {:?} in {=u64} us: head {=u32}, oldest {=u32}, {=u32} held{=str}; this boot's run starts at {=u32}",
        range.start,
        range.end,
        track.capacity(),
        defmt::Debug2Format(&scan.kind),
        us,
        scan.head,
        scan.oldest,
        scan.held,
        match scan.keep {
            Some(0) => "; the head's sector erased",
            Some(_) => "; the head's sector rewritten up to the head",
            None => "",
        },
        track.run_start()
    );
    static TRACK: StaticCell<Tracks> = StaticCell::new();
    Some(TRACK.init(track))
}

/// The bundle `bytes` (in flash), validated, timed; `None` (logged) if it
/// is not valid.
fn load(what: &str, bytes: &'static [u8], memory: &mut EngineMemory) -> Option<Package> {
    let started = esp_hal::time::Instant::now();
    let loaded = Package::load(bytes, &LIMITS, &mut memory.scratch);
    let us = started.elapsed().as_micros();
    match loaded {
        Ok(package) => {
            match package.engine {
                Some(engine) => defmt::info!(
                    "bundle {=str}: {=str} v{=u16}, {=usize} B, loaded (validated) in {=u64} us; code {=usize} B, constants {=usize} B; slots: stack {=u16}, locals {=u16}, inputs {=u16}, outputs {=u16}, state {=u16}",
                    what,
                    package.name,
                    package.version,
                    bytes.len(),
                    us,
                    engine.program.code.len(),
                    engine.program.consts.len(),
                    engine.program.max_stack,
                    engine.program.locals,
                    engine.program.input_count,
                    engine.program.output_count,
                    engine.layout.state_leaves
                ),
                None => defmt::info!(
                    "bundle {=str}: {=str} v{=u16}, {=usize} B, no engine, loaded in {=u64} us",
                    what,
                    package.name,
                    package.version,
                    bytes.len(),
                    us
                ),
            }
            Some(package)
        }
        Err(e) => {
            defmt::error!("bundle {=str}: not valid: {:?}", what, defmt::Debug2Format(&e));
            None
        }
    }
}

#[embassy_executor::task]
async fn httpd_task(stack: Stack<'static>, runtime: &'static Host) -> ! {
    let buffers = TcpBuffers::<MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE, SOCKET_BUFFER_SIZE>::new();
    let tcp = Tcp::new(stack, &buffers);
    serve_http(&tcp, HTTP_PORT, runtime).await
}

#[embassy_executor::task]
async fn ble_task(controller: BleController, runtime: &'static Host, address: [u8; 6]) -> ! {
    let diagnostics = Diagnostics {
        heap_free: || {
            let s = esp_alloc::HEAP.stats();
            s.size - s.current_usage
        },
        log_memory: memory::log,
    };
    serve_ble(controller, runtime, address, &FlashBonds, diagnostics).await
}

/// Feature `dev-pairing` (development only, off by default): the BLE
/// pairing window opens once at boot, exactly as "Pair watch" on the
/// Kindle opens it, so a browser can bond (and reach the encrypted
/// characteristics, BLE.md "Bundles") on a box with no Kindle.
#[cfg(feature = "dev-pairing")]
mod dev_pairing {
    use common::{ble_settings::BleCommand, config::BLE_PAIRING_WINDOW_SECS, host::Host};
    use embassy_time::{Duration, Timer};

    #[embassy_executor::task]
    pub async fn task(runtime: &'static Host) {
        // the BLE transport takes commands once it has published its status
        while runtime.ble_command(BleCommand::Pair).is_err() {
            Timer::after(Duration::from_millis(100)).await;
        }
        defmt::info!("ble: dev-pairing: window open for {=u64} s", BLE_PAIRING_WINDOW_SECS);
    }
}

/// BLE bonds in the settings partition (storage.rs).
struct FlashBonds;

impl BondStore for FlashBonds {
    async fn load(&self) -> [Option<BondRecord>; MAX_BLE_BONDS] {
        storage::bonds().await
    }

    async fn save(&self, slot: usize, bond: &BondRecord) -> bool {
        storage::store_bond(slot, bond).await
    }

    async fn clear(&self) -> bool {
        storage::clear_bonds().await
    }

    async fn load_gatt(&self) -> ([Option<GattClientRecord>; MAX_BLE_BONDS], Option<[u8; 16]>) {
        storage::gatt().await
    }

    async fn save_gatt(&self, slot: usize, record: &GattClientRecord) -> bool {
        storage::store_gatt(slot, record).await
    }

    async fn save_gatt_hash(&self, hash: &[u8; 16]) -> bool {
        storage::store_gatt_hash(hash).await
    }
}

/// This unit's BLE static random address, from the base MAC address burnt
/// into the chip's eFuses at the factory (unique per chip), so no two boxes
/// advertise the same address.
fn ble_address() -> [u8; 6] {
    let mac = esp_hal::efuse::base_mac_address();
    let mut bytes = [0u8; 6];
    bytes.copy_from_slice(mac.as_bytes());
    defmt::info!("efuse: base MAC {=[u8]:02x}", bytes);
    static_random_address(bytes)
}

#[embassy_executor::task]
async fn gps_task(rx: UartRx<'static, Async>, runtime: &'static Host) -> ! {
    read_gps(UartReader(rx), runtime).await
}

#[embassy_executor::task]
async fn timer_task(runtime: &'static Host) -> ! {
    runtime.run_timer().await
}

struct UartReader(UartRx<'static, Async>);

impl AsyncReader for UartReader {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        // returns as soon as some bytes have arrived
        self.0.read_async(buf).await.map_err(|e| {
            defmt::warn!("gps: UART read failed: {:?}", defmt::Debug2Format(&e));
        })
    }
}
