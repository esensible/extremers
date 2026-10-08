#![no_std]
#![no_main]

use static_cell::StaticCell;

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
use esp_radio::wifi::{
    ControllerConfig, CountryInfo, Interface, OperatingClass, PowerSaveMode, WifiController,
};

esp_bootloader_esp_idf::esp_app_desc!();

mod network_tasks;

use crate::network_tasks::{access_point_config, dhcp_task, net_task, wifi_task};
use common::{
    config::{AP_IP, AP_PREFIX_LEN, GPS_BAUD, HTTP_PORT, MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE},
    nmea::AsyncReader,
    runtime::EngineRuntime,
    tasks::{read_gps, serve_http},
};

use extreme_traits::define_engines;

define_engines! {
    EngineType {
        Race(extreme_race::Race),
        TuneSpeed(extreme_tune::TuneSpeed<32>),
    }
}

type Runtime = EngineRuntime<EngineType>;

// Every log line carries the uptime (probe-rs shows it as seconds with ms).
// esp-hal's clock, not embassy-time's, so it works before esp_rtos::start.
defmt::timestamp!("{=u64:ms}", esp_hal::time::Instant::now().duration_since_epoch().as_millis());

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // Logs go over RTT as defmt, read with probe-rs (`probe-rs run` or
    // `probe-rs attach`). 4 KiB covers boot when attaching late.
    rtt_target::rtt_init_defmt!(rtt_target::ChannelMode::NoBlockSkip, 4096);

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // esp-radio needs a heap
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    // start the scheduler (also provides the embassy time driver)
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // initialize wifi controller as an access point
    let controller_config = ControllerConfig::default()
        .with_country_info(
            CountryInfo::from(*b"AU").with_operating_class(OperatingClass::Repr(0x21)),
        )
        .with_initial_config(access_point_config());
    let mut controller = WifiController::new(peripherals.WIFI, controller_config).unwrap();
    controller.set_power_saving(PowerSaveMode::None).unwrap();
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

    static RUNTIME: StaticCell<Runtime> = StaticCell::new();
    let runtime: &'static Runtime = RUNTIME.init(EngineRuntime::new(EngineType::default()));

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

    // idle loop, blink LED
    let mut led = Output::new(peripherals.GPIO15, Level::Low, OutputConfig::default());
    led.set_high();
    loop {
        Timer::after(Duration::from_millis(100)).await;
        led.toggle();
    }
}

#[embassy_executor::task]
async fn httpd_task(stack: Stack<'static>, runtime: &'static Runtime) -> ! {
    let buffers = TcpBuffers::<MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE, SOCKET_BUFFER_SIZE>::new();
    let tcp = Tcp::new(stack, &buffers);
    serve_http(&tcp, HTTP_PORT, runtime).await
}

#[embassy_executor::task]
async fn gps_task(rx: UartRx<'static, Async>, runtime: &'static Runtime) -> ! {
    read_gps(UartReader(rx), runtime).await
}

#[embassy_executor::task]
async fn timer_task(runtime: &'static Runtime) -> ! {
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
