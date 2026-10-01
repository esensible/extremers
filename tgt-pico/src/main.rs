//! Raspberry Pi Pico W target: runs the race computer behind a WiFi access
//! point (cyw43), with a MediaTek GPS module on UART1.

#![no_std]
#![no_main]

// Embassy framework imports
use embassy_executor::Spawner;
use embassy_net::{Config, Ipv4Cidr, Stack, StackResources, StaticConfigV4};
use embassy_rp::{
    bind_interrupts,
    clocks::RoscRng,
    dma,
    gpio::{Level, Output},
    peripherals::{DMA_CH1, DMA_CH2, DMA_CH3, PIO0, UART1, USB},
    pio::{InterruptHandler, Pio},
    uart::{
        Async as UartAsync, Config as UartConfig, InterruptHandler as UartInterruptHandler, Uart,
        UartRx, UartTx,
    },
    usb::{Driver, InterruptHandler as UsbInterruptHandler},
};
use embassy_time::{Duration, Timer};

// Networking imports
use edge_nal_embassy::{Tcp, TcpBuffers};

// Other external crates
use cyw43::aligned_bytes;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use panic_probe as _;
use static_cell::StaticCell;

// Local modules
mod network_tasks;

use crate::network_tasks::{dhcp_server_task, net_task, wifi_task};
use common::{
    config::{
        AP_IP, AP_PREFIX_LEN, GPS_BAUD, HTTP_PORT, MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE,
        WIFI_CHANNEL, WIFI_PASSWORD, WIFI_SSID,
    },
    nmea::{AsyncReader, pmtk_sentence},
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

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    UART1_IRQ => UartInterruptHandler<UART1>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH1>, dma::InterruptHandler<DMA_CH2>, dma::InterruptHandler<DMA_CH3>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let driver = Driver::new(p.USB, Irqs);
    match logger_task(driver) {
        Ok(token) => spawner.spawn(token),
        Err(_) => log::warn!("failed to spawn logger task"),
    }

    //
    // BEGIN WIFI SETUP
    //
    let fw = aligned_bytes!("../cyw43-firmware/43439A0.bin");
    let clm = aligned_bytes!("../cyw43-firmware/43439A0_clm.bin");
    let nvram = aligned_bytes!("../cyw43-firmware/nvram_rp2040.bin");

    // To make flashing faster for development, you may want to flash the firmwares independently
    // at hardcoded addresses, instead of baking them into the program with `include_bytes!`:
    //     probe-rs download 43439A0.bin --binary-format bin --chip RP2040 --base-address 0x10100000
    //     probe-rs download 43439A0_clm.bin --binary-format bin --chip RP2040 --base-address 0x10140000
    //let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    //let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };

    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        dma::Channel::new(p.DMA_CH3, Irqs),
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw, nvram).await;

    match wifi_task(runner) {
        Ok(token) => spawner.spawn(token),
        Err(_) => log::warn!("failed to spawn wifi task"),
    }

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::Performance)
        .await;

    let config = Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(AP_IP, AP_PREFIX_LEN),
        dns_servers: [AP_IP].into_iter().collect(),
        gateway: Some(AP_IP),
    });

    // Generate random seed
    let seed = RoscRng.next_u64();

    // Init network stack
    static RESOURCES: StaticCell<StackResources<{ MAX_WEB_SOCKETS + 2 }>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        net_device,
        config,
        RESOURCES.init(StackResources::new()),
        seed,
    );

    match net_task(runner) {
        Ok(token) => spawner.spawn(token),
        Err(_) => log::warn!("failed to spawn net task"),
    }

    control
        .start_ap_wpa2(WIFI_SSID, WIFI_PASSWORD, WIFI_CHANNEL)
        .await;

    match dhcp_server_task(stack, AP_IP) {
        Ok(token) => spawner.spawn(token),
        Err(_) => log::warn!("failed to spawn dhcp server task"),
    }

    static RUNTIME: StaticCell<Runtime> = StaticCell::new();
    let runtime: &'static Runtime = RUNTIME.init(EngineRuntime::new(EngineType::default()));

    match httpd_task(stack, runtime) {
        Ok(token) => spawner.spawn(token),
        Err(_) => log::warn!("failed to spawn httpd task"),
    }

    match timer_task(runtime) {
        Ok(token) => spawner.spawn(token),
        Err(_) => log::warn!("failed to spawn timer task"),
    }

    let mut config = UartConfig::default();
    config.baudrate = GPS_BAUD;
    let uart = Uart::new(
        p.UART1, p.PIN_8, p.PIN_9, Irqs, p.DMA_CH2, p.DMA_CH1, config,
    );
    let (mut uart_tx, uart_rx) = uart.split();

    // Configure the (MediaTek) GPS module
    // Output only RMC sentences, one per fix
    send_pmtk_command(&mut uart_tx, "PMTK314,0,1,0,0,0,0,0,0").await;
    // Enable SBAS
    send_pmtk_command(&mut uart_tx, "PMTK313,1").await;
    // SBAS integrity mode
    send_pmtk_command(&mut uart_tx, "PMTK319,1").await;

    match gps_task(uart_rx, runtime) {
        Ok(token) => spawner.spawn(token),
        Err(_) => log::warn!("failed to spawn gps task"),
    }

    loop {
        Timer::after(Duration::from_secs(2)).await;
        log::info!(".");
    }
}

#[embassy_executor::task]
async fn logger_task(driver: Driver<'static, USB>) {
    embassy_usb_logger::run!(1024, log::LevelFilter::Debug, driver);
}

#[embassy_executor::task]
async fn httpd_task(stack: Stack<'static>, runtime: &'static Runtime) -> ! {
    let buffers = TcpBuffers::<MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE, SOCKET_BUFFER_SIZE>::new();
    let tcp = Tcp::new(stack, &buffers);
    serve_http(&tcp, HTTP_PORT, runtime).await
}

#[embassy_executor::task]
async fn gps_task(rx: UartRx<'static, UartAsync>, runtime: &'static Runtime) -> ! {
    read_gps(UartReader(rx), runtime).await
}

#[embassy_executor::task]
async fn timer_task(runtime: &'static Runtime) -> ! {
    runtime.run_timer().await
}

struct UartReader(UartRx<'static, UartAsync>);

impl AsyncReader for UartReader {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        // fills the whole buffer
        match self.0.read(buf).await {
            Ok(()) => Ok(buf.len()),
            Err(e) => {
                log::warn!("gps: UART read failed: {:?}", e);
                Err(())
            }
        }
    }
}

async fn send_pmtk_command(tx: &mut UartTx<'static, UartAsync>, command: &str) {
    let mut buf = [0; 64];
    if let Err(e) = tx.write(pmtk_sentence(command, &mut buf)).await {
        log::error!("gps: failed to send {}: {:?}", command, e);
    }
}
