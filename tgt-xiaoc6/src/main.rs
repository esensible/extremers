//! This example uses the RP Pico W board Wifi chip (cyw43).
//! Creates an Access point Wifi network and creates a TCP endpoint on port 1234.

#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![allow(incomplete_features)]

// #[cfg(test)]
// mod tests;

use embassy_executor::Spawner;
use embassy_net::driver::Driver;
use embassy_net::{Config, Ipv4Address, Ipv4Cidr, Stack, Runner, StackResources, StaticConfigV4};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};
use core::{net::{Ipv4Addr, SocketAddr, IpAddr}, str::FromStr};
use esp_println::{print, println};

// use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    gpio::Io,
    gpio::{
        etm::{Channels, OutputConfig as EtmOutputConfig},
        Level, Output, OutputConfig, Pull,
    },
    uart::{AtCmdConfig, Config as UartConfig, RxConfig, Uart, UartRx, UartTx},
    Async,    
    peripherals::UART0,
    rng::Rng,
    timer::timg::TimerGroup,
};

use esp_wifi::{
    init,

    wifi::{
        AccessPointConfiguration, AuthMethod, Configuration, WifiApDevice, WifiController,
        WifiDevice, WifiEvent, WifiState,
    },
    // EspWifiInitFor,
};

use esp_alloc as _;
use heapless::Vec;
use static_cell::StaticCell;


// Local modules
mod http;
mod network_tasks;
mod nmea_parser;

use crate::{
    http::HttpHandler,
    network_tasks::{dhcp_task, net_task, wifi_task},
    nmea_parser::{next_update, AsyncReader, RingBuffer},
};

// Networking imports
use edge_net::{
    embassy::{Tcp, TcpBuffers},
    http::io::server::Server,
    nal::TcpBind,
};

// Constants
const MAX_WEB_SOCKETS: usize = 4;
const MAX_MESSAGE_SIZE: usize = 512;
const SOCKET_BUFFER_SIZE: usize = MAX_MESSAGE_SIZE * 4;


use extreme_traits::define_engines;

define_engines! {
    EngineType {
        Race(extreme_race::Race),
        TuneSpeed(extreme_tune::TuneSpeed<32>),
    }
}


#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {

    static HTTPD_HANDLER: StaticCell<HttpHandler<EngineType>> = StaticCell::new();
    let httpd_handler = HTTPD_HANDLER.init(HttpHandler::new(EngineType::default()));

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let mut rng = Rng::new(peripherals.RNG);

    static INIT: StaticCell<esp_wifi::EspWifiController<'static>> = StaticCell::new();
    let init = INIT.init(init(timg0.timer0, rng.clone(), peripherals.RADIO_CLK).unwrap());

    let wifi = peripherals.WIFI;
    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&*init, wifi, WifiApDevice).unwrap();

    use esp_hal::timer::systimer::SystemTimer;
    let systimer = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(systimer.alarm0);


    let gw_ip_addr = Ipv4Addr::from_str("192.168.1.100").expect("failed to parse gateway ip");
    let mut dns_servers: Vec<_, 3> = Vec::new();
    dns_servers
        .push(embassy_net::Ipv4Address::new(169, 254, 1, 100))
        .unwrap();

    let config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(gw_ip_addr, 24),
        gateway: Some(gw_ip_addr),
        dns_servers: dns_servers,
    });

    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    static RESOURCES: StaticCell<StackResources<{ MAX_WEB_SOCKETS + 1 }>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        config,
        RESOURCES.init(StackResources::new()),
        seed,
    );

    // Init network stack
    static STACK: StaticCell<Stack<'_>> = StaticCell::new();
    let stack = STACK.init(stack);

    spawner.spawn(wifi_task(controller)).ok();
    spawner.spawn(net_task(runner)).ok();
    spawner.spawn(dhcp_task(*stack, "192.168.1.100")).ok();
    spawner.spawn(httpd_task(stack, httpd_handler)).ok();



    let (tx_pin, rx_pin) = (peripherals.GPIO16, peripherals.GPIO17);
    let config = UartConfig::default()
        .with_baudrate(9600)
        .with_rx(RxConfig::default())
       ; // .with_fifo_full_threshold(READ_BUF_SIZE as u16));

    let mut uart0 = Uart::new(peripherals.UART0, config)
        .unwrap()
        .with_tx(tx_pin)
        .with_rx(rx_pin)
        .into_async();
    let (uart_rx, mut uart_tx) = uart0.split();

    Timer::after(Duration::from_secs(10)).await;
    // Configure GPS
    // Only generate GPRMC message twice per second
    send_pmtk_command(&mut uart_tx, "PMTK314,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0").await;
    // send_pmtk_command(&mut uart_tx, "PMTK314,0,1,0,0,0,0,0,0").await;
    send_pmtk_command(&mut uart_tx, "PMTK220,500").await;
    Timer::after(Duration::from_secs(1)).await;

    // Enable SBAS
    send_pmtk_command(&mut uart_tx, "PMTK313,1").await;
    Timer::after(Duration::from_secs(1)).await;

    // SBAS integrity mode
    send_pmtk_command(&mut uart_tx, "PMTK319,1").await;
    Timer::after(Duration::from_secs(1)).await;

    spawner.spawn(gps_task(uart_rx, httpd_handler)).ok(); 
    // spawner.spawn(sleeper_task(httpd_handler)).ok();

    let mut led = Output::new(peripherals.GPIO15, Level::Low, OutputConfig::default());
    led.set_high();

    loop {
        Timer::after(Duration::from_millis(100)).await;
        led.toggle();
    }
}

#[embassy_executor::task]
pub async fn sleeper_task(handler: &'static HttpHandler<EngineType>) {
    handler.run_sleeper().await
}


#[embassy_executor::task]
pub async fn httpd_task(
    stack: &'static Stack<'static>,
    handler: &'static HttpHandler<EngineType>,
) -> ! {
    let buffers = TcpBuffers::<MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE, SOCKET_BUFFER_SIZE>::new();
    let tcp = Tcp::new(*stack, &buffers);

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


struct UartReader(UartRx<'static, Async>);
impl AsyncReader for UartReader {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        match self.0.read_async(buf).await {
            Ok(_) => {
                
                Ok(buf.len())
            },
            Err(_) => {
                println!("Failed to read from UART");
                Err(())
            },
        }
    }
}

#[embassy_executor::task]
pub async fn gps_task(
    rx: UartRx<'static, Async>,
    handler: &'static HttpHandler<EngineType>,
) {
    let mut ring_buffer = RingBuffer::<UartReader, 32>::new(UartReader(rx));
    loop {
        let (time, location, speed) = next_update(&mut ring_buffer).await;
        println!("Time: {:?}, Location: {:?}, Speed: {:?}", time, location, speed);
        handler.location_event(time, location, speed).await;
    }
}


async fn send_pmtk_command(tx: &mut UartTx<'static, Async>, command: &str) {
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
    if let Err(e) = tx.write_async(&buffer[..pos]).await {
        log::error!("Failed to send GPS command: {:?}", e);
    }
}
