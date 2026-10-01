//! This example uses the RP Pico W board Wifi chip (cyw43).
//! Creates an Access point Wifi network and creates a TCP endpoint on port 1234.

#![no_std]
#![no_main]

// Standard library imports
use core::net::{IpAddr, Ipv4Addr, SocketAddr};

// Embassy framework imports
use embassy_executor::Spawner;
use embassy_net::{Config, Stack, StackResources};
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
use edge_http::io::server::Server;
use edge_nal::TcpBind;
use edge_nal_embassy::{Tcp, TcpBuffers};

// Other external crates
use cyw43::aligned_bytes;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use panic_probe as _;
use static_cell::StaticCell;

// Local modules
mod network_tasks;
mod nmea_parser;

use crate::{
    network_tasks::{dhcp_server_task, net_task, wifi_task},
    nmea_parser::{AsyncReader, RingBuffer, next_update},
};
use common::http::{HttpHandler, MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE};

use extreme_traits::{MAX_MESSAGE_SIZE, define_engines};

define_engines! {
    EngineType {
        Race(extreme_race::Race),
        TuneSpeed(extreme_tune::TuneSpeed<32>),
    }
}

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

    // Use a link-local address for communication without DHCP server
    let config = Config::ipv4_static(embassy_net::StaticConfigV4 {
        address: embassy_net::Ipv4Cidr::new(Ipv4Addr::new(169, 254, 1, 1), 16),
        dns_servers: [Ipv4Addr::new(169, 254, 1, 100)].into_iter().collect(),
        gateway: Some(Ipv4Addr::new(169, 254, 1, 100)),
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

    control.start_ap_wpa2("nacra17", "password", 1).await;

    let ip = Ipv4Addr::new(169, 254, 1, 1);

    match dhcp_server_task(stack, ip) {
        Ok(token) => spawner.spawn(token),
        Err(_) => log::warn!("failed to spawn dhcp server task"),
    }

    static HTTPD_HANDLER: StaticCell<HttpHandler<EngineType>> = StaticCell::new();
    let httpd_handler = HTTPD_HANDLER.init(HttpHandler::new(EngineType::default()));

    match httpd_task(stack, httpd_handler) {
        Ok(token) => spawner.spawn(token),
        Err(_) => log::warn!("failed to spawn httpd task"),
    }

    match sleeper_task(httpd_handler) {
        Ok(token) => spawner.spawn(token),
        Err(_) => log::warn!("failed to spawn sleeper task"),
    }

    let mut config = UartConfig::default();
    config.baudrate = 9600;
    let uart = Uart::new(
        p.UART1, p.PIN_8, p.PIN_9, Irqs, p.DMA_CH2, p.DMA_CH1, config,
    );
    let (mut uart_tx, uart_rx) = uart.split();

    // Configure GPS
    // Only generate GPRMC message twice per second
    send_pmtk_command(&mut uart_tx, "PMTK314,0,1,0,0,0,0,0,0").await;
    // Enable SBAS
    send_pmtk_command(&mut uart_tx, "PMTK313,1").await;
    // SBAS integrity mode
    send_pmtk_command(&mut uart_tx, "PMTK319,1").await;

    // Set new baud rate
    // send_pmtk_command(&mut uart_tx, "PMTK251,115200").await;
    // Need to wait a moment for the change to take effect
    // Timer::after(Duration::from_millis(100)).await;
    // config.baudrate = 115200;
    // uart.set_config(config);

    match gps_task(uart_rx, httpd_handler) {
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
pub async fn sleeper_task(handler: &'static HttpHandler<EngineType>) {
    handler.run_sleeper().await
}

struct UartReader(UartRx<'static, UartAsync>);
impl AsyncReader for UartReader {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        // fills the whole buffer
        match self.0.read(buf).await {
            Ok(()) => Ok(buf.len()),
            Err(_) => Err(()),
        }
    }
}

#[embassy_executor::task]
pub async fn gps_task(rx: UartRx<'static, UartAsync>, handler: &'static HttpHandler<EngineType>) {
    let mut ring_buffer = RingBuffer::<UartReader, 32>::new(UartReader(rx));
    loop {
        let (time, location, speed) = next_update(&mut ring_buffer).await;
        handler.location_event(time, location, speed).await;
    }
}

#[embassy_executor::task]
pub async fn httpd_task(stack: Stack<'static>, handler: &'static HttpHandler<EngineType>) -> ! {
    let buffers = TcpBuffers::<MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE, SOCKET_BUFFER_SIZE>::new();
    let tcp = Tcp::new(stack, &buffers);

    loop {
        let acceptor = match tcp
            .bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 80))
            .await
        {
            Ok(socket) => socket,
            Err(e) => {
                log::error!("Failed to bind httpd socket: {:?}", e);
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
        };

        let mut server: Server<MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE, 64> = Server::new();
        match server.run(None, acceptor, handler).await {
            Ok(_) => (),
            Err(e) => {
                log::error!("HTTPd server error: {:?}", e);
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
        }
    }
}

async fn send_pmtk_command(tx: &mut UartTx<'static, UartAsync>, command: &str) {
    // Calculate checksum
    let checksum = command.bytes().fold(0u8, |acc, b| acc ^ b);

    // We'll use a static buffer since we're in no_std
    let mut buffer: [u8; 64] = [0; 64];
    let mut pos = 0;

    // Build command manually
    buffer[pos] = b'$';
    pos += 1;
    for &byte in command.as_bytes() {
        buffer[pos] = byte;
        pos += 1;
    }
    buffer[pos] = b'*';
    pos += 1;

    // Convert checksum to hex (manual implementation)
    let hex_chars = [
        b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'A', b'B', b'C', b'D', b'E',
        b'F',
    ];
    buffer[pos] = hex_chars[(checksum >> 4) as usize];
    pos += 1;
    buffer[pos] = hex_chars[(checksum & 0xF) as usize];
    pos += 1;

    // Add CR+LF
    buffer[pos] = b'\r';
    pos += 1;
    buffer[pos] = b'\n';
    pos += 1;

    // Send command
    if let Err(e) = tx.write(&buffer[..pos]).await {
        log::error!("Failed to send GPS command: {:?}", e);
    }
}
