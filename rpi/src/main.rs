//! This example uses the RP Pico W board Wifi chip (cyw43).
//! Creates an Access point Wifi network and creates a TCP endpoint on port 1234.

#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![allow(incomplete_features)]

// #[cfg(test)]
// mod tests;

use cyw43_pio::PioSpi;
use embassy_executor::Spawner;
use embassy_net::{Config, Stack, StackResources, udp};
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::UART1;
use embassy_rp::peripherals::USB;
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::uart::{
    Async, Config as UartConfig, InterruptHandler as UartInterruptHandler, UartRx,
};
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};

use heapless::Vec;
use static_cell::StaticCell;

use core::net::{Ipv4Addr, SocketAddrV4};
use core::net::{SocketAddr, IpAddr};

use edge_net::dhcp::io::{self, DEFAULT_CLIENT_PORT, DEFAULT_SERVER_PORT};
use edge_net::dhcp::server::{Server, ServerOptions};

use edge_net::embassy::{Udp, UdpBuffers};
use edge_net::dhcp::client::Client;
use edge_net::dhcp::io::client::Lease;
use edge_net::nal::{MacAddr, RawBind};
use edge_net::raw::io::RawSocket2Udp;

use edge_net::nal::UdpBind;

use lib_extreme_nostd::{
    gps_task_impl, httpd_task_impl, sleeper_task_impl, AsyncReader, RingBuffer, MAX_SOCKETS,
};

use panic_probe as _;

use engine_race::RaceHttpd;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    UART1_IRQ => UartInterruptHandler<UART1>;
});

#[embassy_executor::task]
async fn wifi_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<cyw43::NetDriver<'static>>) -> ! {
    stack.run().await
}

#[embassy_executor::task]
async fn logger_task(driver: Driver<'static, USB>) {
    embassy_usb_logger::run!(1024, log::LevelFilter::Info, driver);
}

struct UartReader(UartRx<'static, UART1, Async>);
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
    httpd_mutex: &'static embassy_sync::mutex::Mutex<
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        RaceHttpd,
    >,
    rx: UartRx<'static, UART1, Async>,
) {
    let mut ring_buffer = RingBuffer::<UartReader, 32>::new(UartReader(rx));
    gps_task_impl(httpd_mutex, &mut ring_buffer).await;
}

#[embassy_executor::task(pool_size = MAX_SOCKETS)]
pub async fn httpd_task(
    httpd_mutex: &'static embassy_sync::mutex::Mutex<
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        RaceHttpd,
    >,
    stack: &'static embassy_net::Stack<cyw43::NetDriver<'static>>,
) -> ! {
    httpd_task_impl(httpd_mutex, stack).await
}

#[embassy_executor::task]
pub async fn sleeper_task(
    httpd_mutex: &'static embassy_sync::mutex::Mutex<
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        RaceHttpd,
    >,
) {
    sleeper_task_impl(httpd_mutex).await;
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    type RaceHttpdMutex = Mutex<CriticalSectionRawMutex, RaceHttpd>;

    static HTTPD: StaticCell<RaceHttpdMutex> = StaticCell::new();
    let httpd = HTTPD.init(RaceHttpdMutex::new(RaceHttpd::default()));

    let p = embassy_rp::init(Default::default());

    let driver = Driver::new(p.USB, Irqs);
    let result = spawner.spawn(logger_task(driver));
    if result.is_err() {
        log::warn!("failed to spawn logger task");
    }

    let mut config = UartConfig::default();
    config.baudrate = 9600;
    let uart_rx = UartRx::new(p.UART1, p.PIN_9, Irqs, p.DMA_CH1, config);

    let result = spawner.spawn(gps_task(httpd, uart_rx));
    if result.is_err() {
        log::warn!("failed to spawn gps task");
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
        p.DMA_CH0,
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

    static RESOURCES: StaticCell<StackResources<{ MAX_SOCKETS + 1 }>> = StaticCell::new();
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

    spawner.spawn(dhcp_server_task(stack, ip)).unwrap();

    let result = spawner.spawn(sleeper_task(httpd));
    if result.is_err() {
        log::warn!("failed to spawn sleeper task");
    }

    for _ in 0..MAX_SOCKETS {
        let result = spawner.spawn(httpd_task(httpd, stack));
        if result.is_err() {
            log::warn!("failed to spawn httpd task");
            break;
        }
    }
    loop {
        Timer::after(Duration::from_secs(10)).await;
        log::info!(".");
    }
}


#[embassy_executor::task]
async fn dhcp_server_task(stack: &'static Stack<cyw43::NetDriver<'static>>, ip: Ipv4Addr) -> ! {
    let buffers = UdpBuffers::<1, 1500, 1500, 2>::new();
    let udp = Udp::new(&stack, &buffers);

    let mut tx_buf = [0u8; 1500];
    let mut rx_buf = [0u8; 1500];

    let mut socket = udp
        .bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), DEFAULT_SERVER_PORT))
        .await
        .unwrap();


    // Will give IP addresses in the range x.x.x.50 - x.x.x.200, subnet 255.255.255.0
    let mut server = edge_net::dhcp::server::Server::<64>::new(ip);
    let mut gw_buf = [Ipv4Addr::UNSPECIFIED];
    let server_options = ServerOptions::new(ip, Some(&mut gw_buf));

    let mut buf = [0u8; 1500];

    loop {
        edge_net::dhcp::io::server::run(
            &mut server,
            &server_options,
            &mut socket,
            &mut buf,
        )
        .await;
    }
}