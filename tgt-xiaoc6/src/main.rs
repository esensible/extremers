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
    tasks::{read_gps, serve_http},
    track::FlashTrack,
};
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;

/// The engines' buffers (common::engine), static: not the heap, not the
/// main stack.
static ENGINE_MEMORY: ConstStaticCell<EngineMemory> = ConstStaticCell::new(EngineMemory::new());

/// Feature `dev-race`: the one app, a bundle embedded in flash (64-byte
/// aligned, as a bundle in the store will be), until bundles are installed
/// into their own partition.
#[cfg(feature = "dev-race")]
mod dev_bundle {
    #[repr(C, align(64))]
    struct Aligned<const N: usize>([u8; N]);

    const PATH: &str = env!(
        "EXTREMERS_DEV_BUNDLE",
        "feature dev-race: set EXTREMERS_DEV_BUNDLE to a bundle's path (cargo run -p extreme-pack -- --bundle writes target/packages/race.bundle)"
    );
    static BUNDLE: &Aligned<{ include_bytes!(env!("EXTREMERS_DEV_BUNDLE")).len() }> =
        &Aligned(*include_bytes!(env!("EXTREMERS_DEV_BUNDLE")));

    pub static BYTES: &[u8] = &BUNDLE.0;

    pub fn path() -> &'static str {
        PATH
    }
}

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

    // A second handle on the flash, for the data slot (xip.rs): its own
    // FlashStorage, over flash disjoint from the settings partition's.
    // SAFETY: esp-storage runs every operation in its own critical section,
    // so the two never interleave; neither assumes it is the only writer of
    // anything outside its own region.
    let data_flash = unsafe { peripherals.FLASH.clone_unchecked() };
    // Settings partition (read only here; see storage.rs), then the access
    // point's credentials from it, or the compiled-in defaults.
    storage::init(peripherals.FLASH).await;
    choose_access_point_credentials().await;
    let track = flash_track(data_flash);

    memory::log("before the host");
    let engine_memory = ENGINE_MEMORY.take();
    let choose = load("choose", extreme_packages::CHOOSE, engine_memory);
    #[cfg(feature = "dev-race")]
    let apps = load(dev_bundle::path(), dev_bundle::BYTES, engine_memory);
    #[cfg(not(feature = "dev-race"))]
    let apps: Option<Package> = None;
    let registry = Registry::new(choose, apps.as_slice());
    static HOST: StaticCell<Host> = StaticCell::new();
    let runtime: &'static Host = HOST.init(match track {
        Some(track) => Host::with_track(registry, engine_memory, track),
        None => Host::new(registry, engine_memory),
    });
    memory::log("host up");

    // BLE (BLE.md) shares the radio with WiFi (esp-radio `coex`); the
    // controller's connection limit matches the host's.
    let ble_config = esp_radio::ble::Config::default().with_max_connections(CONNECTIONS_MAX as u16);
    let connector = BleConnector::new(peripherals.BT, ble_config).unwrap();
    spawner.spawn(ble_task(ExternalController::new(connector), runtime, ble_address()).unwrap());

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

/// The engines' `gps` in the data slot's tracks region (common::track,
/// xip.rs), read in place: maps the slot, checks that reads in place see
/// what the flash holds, and opens the ring (a new run from its head).
/// `None` (logged) if any of that fails: the host then keeps the track in
/// RAM, gone at a reboot.
fn flash_track(flash: FLASH<'static>) -> Option<&'static mut Tracks> {
    let fallback = "track: in RAM instead (lost at a reboot)";
    let Some(slot) = storage::data_slot() else {
        defmt::warn!("track: no data slot; {=str}", fallback);
        return None;
    };
    if !xip::map(&slot) {
        defmt::warn!("{=str}", fallback);
        return None;
    }
    let Some(mut region) = xip::XipFlash::new(FlashStorage::new(flash), slot.tracks()) else {
        defmt::warn!("track: tracks region not mapped; {=str}", fallback);
        return None;
    };
    if !region.self_check(&slot) {
        defmt::warn!("{=str}", fallback);
        return None;
    }
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
        slot.tracks().start,
        slot.tracks().end,
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
