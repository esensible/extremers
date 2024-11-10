//! This example uses the RP Pico W board Wifi chip (cyw43).
//! Creates an Access point Wifi network and creates a TCP endpoint on port 1234.

#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![allow(incomplete_features)]

// #[cfg(test)]
// mod tests;

use edge_net::embassy::TcpBuffers;
use embassy_executor::Spawner;
use embassy_net::{Config, Stack, StackResources};
use embassy_time::{Duration, Timer};


use cyw43_pio::PioSpi;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{USB, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};

use heapless::Vec;
use static_cell::StaticCell;
use core::net::Ipv4Addr;

mod network_tasks;
use crate::network_tasks::{dhcp_server_task, wifi_task, net_task};


use panic_probe as _;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
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
    static RESOURCES: StaticCell<StackResources<6 >> = StaticCell::new();
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

    let result = spawner.spawn(httpd_task(stack));
    if result.is_err() {
        log::warn!("failed to spawn httpd task");
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


use core::fmt::Debug;
use core::fmt::Display;
use edge_net::http::io::server::{Connection, DefaultServer, Server, Handler};
use edge_net::http::ws::MAX_BASE64_KEY_RESPONSE_LEN;
use edge_net::http::Method;
use edge_net::nal::TcpBind;
use edge_net::ws::{FrameHeader, FrameType};
use core::net::IpAddr;
use embedded_io_async::{Read, Write};
use edge_net::embassy::Tcp;
use core::net::SocketAddr;
use log::info;
use edge_net::http::io::Error;
use edge_net::nal::TcpAccept;

#[embassy_executor::task]
pub async fn httpd_task(stack: &'static Stack<cyw43::NetDriver<'static>>) -> !{
    let buffers = TcpBuffers::<1, 2048, 2048>::new();
    let tcp = Tcp::new(&stack, &buffers);

    let acceptor = match tcp
        .bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 80))
        .await {
            Ok(socket) => socket,
            Err(e) => {
                log::error!("Failed to bind httpd socket: {:?}", e);
                loop {
                    Timer::after(Duration::from_secs(1)).await;
                    log::error!("HTTPd server loop");
                }
            }
        };

    log::error!("HTTP server stall");
    Timer::after(Duration::from_secs(10)).await;
    log::error!("HTTP server go time");

    let mut server: Server<1, 2048, 64>= Server::new();
    let _ = server.run(None, acceptor, HttpHandler).await;

    loop {
        Timer::after(Duration::from_secs(1)).await;
        log::error!("HTTP server loop");
    }
}


struct HttpHandler;

impl Handler for HttpHandler {
    type Error<E>
        = Error<E>
    where
        E: Debug;

    async fn handle<T, const N: usize>(
        &self,
        _task_id: impl Display + Clone,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write,
    {
        let headers = conn.headers()?;

        if headers.method != Method::Get {
            conn.initiate_response(405, Some("Method Not Allowed"), &[])
                .await?;
        } else if headers.path != "/" {
            conn.initiate_response(404, Some("Not Found"), &[]).await?;
        } else if !conn.is_ws_upgrade_request()? {
            conn.initiate_response(200, Some("OK"), &[("Content-Type", "text/plain")])
                .await?;

            conn.write_all(b"Initiate WS Upgrade request to switch this connection to WS")
                .await?;
        } else {
            let mut buf = [0_u8; MAX_BASE64_KEY_RESPONSE_LEN];
            conn.initiate_ws_upgrade_response(&mut buf).await?;

            conn.complete().await?;

            info!("Connection upgraded to WS, starting a simple WS echo server now");

            // Now we have the TCP socket in a state where it can be operated as a WS connection
            // Run a simple WS echo server here

            let mut socket = conn.unbind()?;

            let mut buf = [0_u8; 8192];

            loop {
                let mut header = FrameHeader::recv(&mut socket)
                    .await;
                if header.is_err() {
                    log::error!("Failed to recv header");
                    break;
                }
                let mut header = header.unwrap();
                    
                let payload = header
                    .recv_payload(&mut socket, &mut buf)
                    .await;
                if payload.is_err() {
                    log::error!("Failed to recv payload");
                    break;
                }
                let payload = payload.unwrap();

                match header.frame_type {
                    FrameType::Text(_) => {
                        info!(
                            "Got {header}, with payload \"{}\"",
                            core::str::from_utf8(payload).unwrap()
                        );
                    }
                    FrameType::Binary(_) => {
                        info!("Got {header}, with payload {payload:?}");
                    }
                    FrameType::Close => {
                        info!("Got {header}, client closed the connection cleanly");
                        break;
                    }
                    _ => {
                        info!("Got {header}");
                    }
                }

                // Echo it back now

                header.mask_key = None; // Servers never mask the payload

                if matches!(header.frame_type, FrameType::Ping) {
                    header.frame_type = FrameType::Pong;
                }

                info!("Echoing back as {header}");

                header.send(&mut socket).await;
                header
                    .send_payload(&mut socket, payload)
                    .await;
                
            }
        }

        Ok(())
    }
}