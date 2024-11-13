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
    gpio::{Level, Output},
    peripherals::{PIO0, UART1, USB},
    pio::{InterruptHandler, Pio},
    uart::{
        Async as UartAsync, Config as UartConfig, InterruptHandler as UartInterruptHandler, UartRx,
    },
    usb::{Driver, InterruptHandler as UsbInterruptHandler},
};
use embassy_time::{Duration, Timer};

// Networking imports
use edge_net::{
    embassy::{Tcp, TcpBuffers},
    http::io::server::Server,
    nal::TcpBind,
};

// Other external crates
use cyw43_pio::PioSpi;
use heapless::Vec;
use panic_probe as _;
use static_cell::StaticCell;

// Local modules
mod http;
mod network_tasks;
mod nmea_parser;

use crate::{
    http::HttpHandler,
    network_tasks::{dhcp_server_task, net_task, wifi_task},
    nmea_parser::{next_update, AsyncReader, RingBuffer},
};

// Constants
const MAX_WEB_SOCKETS: usize = 4;
const MAX_MESSAGE_SIZE: usize = 512;
const SOCKET_BUFFER_SIZE: usize = MAX_MESSAGE_SIZE * 4;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    UART1_IRQ => UartInterruptHandler<UART1>;

});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let driver = Driver::new(p.USB, Irqs);
    let result = spawner.spawn(logger_task(driver));
    if result.is_err() {
        log::warn!("failed to spawn logger task");
    }

    //
    // BEGIN WIFI SETUP
    //
    let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");

    // To make flashing faster for development, you may want to flash the firmwares independently
    // at hardcoded addresses, instead of baking them into the program with `include_bytes!`:
    //     probe-rs download 43439A0.bin --format bin --chip RP2040 --base-address 0x10100000
    //     probe-rs download 43439A0_clm.bin --format bin --chip RP2040 --base-address 0x10140000
    //let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    //let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };

    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        p.DMA_CH2,
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;

    let result = spawner.spawn(wifi_task(runner));
    if result.is_err() {
        log::warn!("failed to spawn wifi task");
    }

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::Performance)
        .await;

    let mut dns_servers: Vec<_, 3> = Vec::new();
    dns_servers
        .push(embassy_net::Ipv4Address::new(169, 254, 1, 100))
        .unwrap();

    // let config = Config::default();
    // Use a link-local address for communication without DHCP server
    let config = Config::ipv4_static(embassy_net::StaticConfigV4 {
        address: embassy_net::Ipv4Cidr::new(embassy_net::Ipv4Address::new(169, 254, 1, 1), 16),
        dns_servers: dns_servers,
        gateway: Some(embassy_net::Ipv4Address::new(169, 254, 1, 100)),
        // gateway: None,
    });

    // Generate random seed
    let seed = 0x0123_a5a7_83a4_fdef; // chosen by fair dice roll. guarenteed to be random.

    // Init network stack
    static RESOURCES: StaticCell<StackResources<6>> = StaticCell::new();
    static STACK: StaticCell<Stack<cyw43::NetDriver<'static>>> = StaticCell::new();
    let stack = Stack::new(
        net_device,
        config,
        RESOURCES.init(StackResources::new()),
        seed,
    );
    let stack = STACK.init(stack);

    let result = spawner.spawn(net_task(stack));
    if result.is_err() {
        log::warn!("failed to spawn net task");
    }

    control.start_ap_wpa2("nacra17", "password", 1).await;

    let ip = Ipv4Addr::new(169, 254, 1, 1);

    let result = spawner.spawn(dhcp_server_task(stack, ip));
    if result.is_err() {
        log::warn!("failed to spawn dhcp server task");
    }

    static HTTPD_HANDLER: StaticCell<HttpHandler<extreme_race::Race>> = StaticCell::new();
    let httpd_handler = HTTPD_HANDLER.init(HttpHandler::new(extreme_race::Race::default()));

    let result = spawner.spawn(httpd_task(stack, httpd_handler));
    if result.is_err() {
        log::warn!("failed to spawn httpd task");
    }

    let result = spawner.spawn(sleeper_task(httpd_handler));
    if result.is_err() {
        log::warn!("failed to spawn sleeper task");
    }

    let mut config = UartConfig::default();
    config.baudrate = 9600;
    let uart_rx = UartRx::new(p.UART1, p.PIN_9, Irqs, p.DMA_CH1, config);
    let result = spawner.spawn(gps_task(uart_rx, httpd_handler));
    if result.is_err() {
        log::warn!("failed to spawn gps task");
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
pub async fn sleeper_task(handler: &'static HttpHandler<extreme_race::Race>) {
    handler.run_sleeper().await
}

struct UartReader(UartRx<'static, UART1, UartAsync>);
impl AsyncReader for UartReader {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        match self.0.read(buf).await {
            Ok(_) => Ok(buf.len()),
            Err(_) => Err(()),
        }
    }
}

#[embassy_executor::task]
pub async fn gps_task(
    rx: UartRx<'static, UART1, UartAsync>,
    handler: &'static HttpHandler<extreme_race::Race>,
) {
    let mut ring_buffer = RingBuffer::<UartReader, 32>::new(UartReader(rx));
    loop {
        let (time, location, speed) = next_update(&mut ring_buffer).await;
        handler.location_event(time, location, speed).await;
    }
}

#[embassy_executor::task]
pub async fn httpd_task(
    stack: &'static Stack<cyw43::NetDriver<'static>>,
    handler: &'static HttpHandler<extreme_race::Race>,
) -> ! {
    let buffers = TcpBuffers::<MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE, SOCKET_BUFFER_SIZE>::new();
    let tcp = Tcp::new(&stack, &buffers);

    loop {
        let acceptor = match tcp
            .bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 80))
            .await
        {
            Ok(socket) => socket,
            Err(e) => {
                log::error!("Failed to bind httpd socket: {:?}", e);
                Timer::after(Duration::from_secs(10)).await;
                continue;
            }
        };

        let mut server: Server<MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE, 64> = Server::new();
        match server.run(None, acceptor, handler).await {
            Ok(_) => (),
            Err(e) => {
                log::error!("HTTPd server error: {:?}", e);
                Timer::after(Duration::from_secs(10)).await;
                continue;
            }
        }
    }
}
