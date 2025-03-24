#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![allow(incomplete_features)]

use core::{net::{Ipv4Addr, SocketAddr, IpAddr}, str::FromStr};
use heapless::Vec;
use static_cell::StaticCell;

use embassy_executor::Spawner;
use embassy_net::{Ipv4Cidr, Stack, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};

use esp_alloc as _;
use esp_backtrace as _;
use esp_println::{println, logger::init_logger};
use esp_hal::{
    clock::CpuClock,
    gpio::{
        Level, Output, OutputConfig,
    },
    uart::{Config as UartConfig, RxConfig, Uart, UartRx, UartTx},
    Async,    
    rng::Rng,
    timer::timg::TimerGroup,
};
use esp_wifi::{
    init,
    config::PowerSaveMode,
};
use edge_net::{
    embassy::{Tcp, TcpBuffers},
    http::io::server::Server,
    nal::TcpBind,
};

// Local modules
mod http;
mod network_tasks;
mod nmea_parser;

use crate::{
    http::HttpHandler,
    network_tasks::{dhcp_task, net_task, wifi_task},
    nmea_parser::{next_update, AsyncReader, RingBuffer},
};

// Constants
const MAX_WEB_SOCKETS: usize = 4;
const MAX_MESSAGE_SIZE: usize = 512;
const SOCKET_BUFFER_SIZE: usize = MAX_MESSAGE_SIZE * 4;

// UBX protocol constants
// const UBX_SYNC1: u8 = 0xB5;
// const UBX_SYNC2: u8 = 0x62;
// const UBX_CLASS_CFG: u8 = 0x06;
// const UBX_CFG_RXM: u8 = 0x11;
// const UBX_CFG_RXM_POWER_MODE: u8 = 0x02; // standby


use extreme_traits::define_engines;

// type EngineType = extreme_race::Race;
define_engines! {
    EngineType {
        Race(extreme_race::Race),
        TuneSpeed(extreme_tune::TuneSpeed<32>),
    }
}


#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Initialize system timer
    use esp_hal::timer::systimer::SystemTimer;
    let systimer = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(systimer.alarm0);

    // esp_alloc::heap_allocator!(76 * 1024);

    init_logger(log::LevelFilter::Info);


    // initialize wifi controller
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let mut rng = Rng::new(peripherals.RNG);

    static INIT: StaticCell<esp_wifi::EspWifiController<'static>> = StaticCell::new();
    let init = INIT.init(init(timg0.timer0, rng.clone(), peripherals.RADIO_CLK).unwrap());

    let (mut controller, interfaces) =
        esp_wifi::wifi::new(&*init, peripherals.WIFI).unwrap();
    let device = interfaces.ap;

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

    static RESOURCES: StaticCell<StackResources<{ MAX_WEB_SOCKETS + 5 }>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        device,
        config,
        RESOURCES.init(StackResources::new()),
        seed,
    );

    // Init network stack
    static STACK: StaticCell<Stack<'_>> = StaticCell::new();
    let stack = STACK.init(stack);

    controller.set_power_saving(PowerSaveMode::None).unwrap();
    spawner.spawn(wifi_task(controller)).ok();
    spawner.spawn(net_task(runner)).ok();
    spawner.spawn(dhcp_task(*stack, "192.168.1.100")).ok();

    // initialize httpd handler and associated tasks
    static HTTPD_HANDLER: StaticCell<HttpHandler<EngineType>> = StaticCell::new();
    let httpd_handler = HTTPD_HANDLER.init(HttpHandler::new(EngineType::default()));

    spawner.spawn(httpd_task(stack, httpd_handler)).ok();

    let (tx_pin, rx_pin) = (peripherals.GPIO16, peripherals.GPIO17);
    let config = UartConfig::default()
        .with_baudrate(9600)
        .with_rx(RxConfig::default())
       ; // .with_fifo_full_threshold(READ_BUF_SIZE as u16));

    let uart0 = Uart::new(peripherals.UART0, config)
        .unwrap()
        .with_tx(tx_pin)
        .with_rx(rx_pin)
        .into_async();
    let (uart_rx, mut _uart_tx) = uart0.split();
    spawner.spawn(gps_task(uart_rx, httpd_handler)).ok(); 

    spawner.spawn(sleeper_task(httpd_handler)).ok();

    // idle loop, blink LED
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
        // println!("Time: {:?}, Location: {:?}, Speed: {:?}", time, location, speed);
        handler.location_event(time, location, speed).await;
    }
}

// async fn send_ubx_command(tx: &mut UartTx<'static, Async>, class: u8, id: u8, payload: &[u8]) {
//     let mut buffer: [u8; 64] = [0; 64];
//     let mut pos = 0;

//     // Header
//     buffer[pos] = UBX_SYNC1; pos += 1;
//     buffer[pos] = UBX_SYNC2; pos += 1;
//     buffer[pos] = class; pos += 1;
//     buffer[pos] = id; pos += 1;
    
//     // Length (little endian)
//     buffer[pos] = payload.len() as u8; pos += 1;
//     buffer[pos] = 0; pos += 1;

//     // Payload
//     for &byte in payload {
//         buffer[pos] = byte;
//         pos += 1;
//     }

//     // Calculate checksum
//     let (ck_a, ck_b) = calculate_ubx_checksum(&buffer[2..pos]);
//     buffer[pos] = ck_a; pos += 1;
//     buffer[pos] = ck_b; pos += 1;

//     if let Err(e) = tx.write_async(&buffer[..pos]).await {
//         log::error!("Failed to send UBX command: {:?}", e);
//     }
// }

// fn calculate_ubx_checksum(data: &[u8]) -> (u8, u8) {
//     let mut ck_a: u8 = 0;
//     let mut ck_b: u8 = 0;

//     for &byte in data {
//         ck_a = ck_a.wrapping_add(byte);
//         ck_b = ck_b.wrapping_add(ck_a);
//     }

//     (ck_a, ck_b)
// }