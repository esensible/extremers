//! This example uses the RP Pico W board Wifi chip (cyw43).
//! Creates an Access point Wifi network and creates a TCP endpoint on port 1234.

#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![feature(async_fn_in_trait)]
#![allow(incomplete_features)]

use core::str::from_utf8;

// use {defmt_rtt as _, panic_probe as _};
use ::core::panic::PanicInfo;
use cyw43_pio::PioSpi;
// use defmt::*;
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Config, Stack, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::UART1;
use embassy_rp::peripherals::{DMA_CH0, PIN_23, PIN_25, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::uart::{
    Async, Config as UartConfig, InterruptHandler as UartInterruptHandler, UartRx, UartTx,
};
use embassy_time::{with_timeout, Duration, Timer};
use lib_httpd::{EngineHttpdTrait, RaceHttpd, Response};
// use embedded_io::Read;
use defmt::*;
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::{DynSubscriber, PubSubChannel, Subscriber};
use embedded_io_async::Read;
use embedded_io_async::Write;
use static_cell::make_static;
use {defmt_rtt as _, panic_probe as _};

mod nmea_parser;
use nmea_parser::{NMEAMessage, NMEAParser};

// #[panic_handler]
// fn panic(_info: &PanicInfo) -> ! {
//     loop {}
// }

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    UART1_IRQ => UartInterruptHandler<UART1>;

});

#[embassy_executor::task]
async fn wifi_task(
    runner: cyw43::Runner<
        'static,
        Output<'static, PIN_23>,
        PioSpi<'static, PIN_25, PIO0, 0, DMA_CH0>,
    >,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<cyw43::NetDriver<'static>>) -> ! {
    stack.run().await
}

const PORT: u16 = 80;
const TIMEOUT: Duration = Duration::from_secs(10);
const RX_BUF_SIZE: usize = 2048;
const TX_BUF_SIZE: usize = 2048;
const READ_BUF_SIZE: usize = 4096;
const RESPONSE_BUF_SIZE: usize = 4096;
const UPDATE_BUF_SIZE: usize = 4096;

const MAX_SOCKETS: usize = 4;

#[derive(Clone)]
struct UpdateMessage([u8; UPDATE_BUF_SIZE], usize);

impl Default for UpdateMessage {
    fn default() -> Self {
        Self([0; UPDATE_BUF_SIZE], 0)
    }
}

static UPDATES_BUS: PubSubChannel<ThreadModeRawMutex, UpdateMessage, 1, 2, 1> =
    PubSubChannel::new();

#[embassy_executor::task(pool_size = MAX_SOCKETS)]
async fn httpd_task(
    httpd_mutex: &'static Mutex<ThreadModeRawMutex, RaceHttpd>,
    stack: &'static Stack<cyw43::NetDriver<'static>>,
) -> ! {
    let mut rx_buffer = [0; RX_BUF_SIZE];
    let mut tx_buffer = [0; TX_BUF_SIZE];
    let mut read_buffer = [0; READ_BUF_SIZE];
    let mut response_buffer = [0; RESPONSE_BUF_SIZE];
    let mut update = UpdateMessage::default();

    loop {
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        // socket.set_timeout(Some(Duration::from_secs(10)));

        log::info!("Listening...");
        if let Err(e) = socket.accept(PORT).await {
            warn!("accept error: {:?}", e);
            continue;
        }

        log::info!("Connect");

        let mut partial_offs = 0;
        loop {
            match socket.read(&mut read_buffer[partial_offs..]).await {
                Ok(0) => {
                    log::warn!("read EOF");
                    break;
                }
                Ok(n) => {
                    unsafe {
                        log::info!("Received");
                    }
                    let sleep_closure: &dyn Fn(usize, usize) = &|time, pos| {};

                    let response = {
                        let mut engine = httpd_mutex.lock().await;

                        (*engine).handle_request(
                            &read_buffer[..partial_offs + n],
                            &mut response_buffer,
                            &mut update.0,
                            &sleep_closure,
                        )
                    };

                    if let Err(len) = response {
                        log::warn!("handle_request error: {:?}", len);
                        socket.write_all(&response_buffer[..len]).await;
                        partial_offs = 0;
                        continue;
                    }

                    let response = response.unwrap();

                    match response {
                        Response::Partial(to_go) => {
                            partial_offs = n;
                            continue;
                        }

                        Response::Complete(r_len, up_len, ex) => {
                            log::info!("handle_request -> {:?}, {:?}", r_len, up_len);

                            if let Some(r_len) = r_len {
                                socket.write_all(&response_buffer[..r_len]).await;
                            }
                            if let Some(ex) = ex {
                                socket.write_all(ex).await;
                            }
                            if let Some(up_len) = up_len {
                                let publisher = UPDATES_BUS.publisher();
                                match publisher {
                                    Ok(publisher) => {
                                        update.1 = up_len;
                                        publisher.publish_immediate(update.clone());
                                    }
                                    Err(_) => {
                                        log::warn!("Error obtaining publisher");
                                    }
                                }
                            }
                        }
                        Response::None => {
                            log::info!("handle_request -> None");
                            let mut message_subscriber = UPDATES_BUS.dyn_subscriber().unwrap();
                            match with_timeout(
                                Duration::from_secs(5),
                                message_subscriber.next_message_pure(),
                            )
                            .await
                            {
                                Ok(message) => {
                                    log::info!("update: {:?}", message.1);
                                    socket.write_all(&message.0[..message.1]).await;
                                }
                                Err(_) => {
                                    socket.write_all(b"HTTP/1.1 204 Timeout\r\n\r\n").await;
                                }
                            }
                        }
                        _ => {
                            log::warn!("Invalid response type");
                            break;
                        }
                    }
                }
                Err(e) => {
                    log::warn!("read error: {:?}", e);
                    break;
                }
            };
            partial_offs = 0;
        }
    }
}

#[embassy_executor::task]
async fn logger_task(driver: Driver<'static, USB>) {
    embassy_usb_logger::run!(1024, log::LevelFilter::Info, driver);
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // info!("Hello World!");

    let httpd = make_static!(Mutex::<ThreadModeRawMutex, _>::new(RaceHttpd::default()));

    let p = embassy_rp::init(Default::default());

    let driver = Driver::new(p.USB, Irqs);
    spawner.spawn(logger_task(driver)).unwrap();

    let mut config = UartConfig::default();
    config.baudrate = 9600;
    let uart_rx = UartRx::new(p.UART1, p.PIN_9, Irqs, p.DMA_CH1, config);

    spawner.spawn(reader(uart_rx)).unwrap();

    //
    // BEGIN WIFI SETUP
    //
    let fw = include_bytes!("/Users/esensible/src/extremers/rpi/cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("/Users/esensible/src/extremers/rpi/cyw43-firmware/43439A0_clm.bin");

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

    let state = make_static!(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    spawner.spawn(wifi_task(runner));

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let config = Config::dhcpv4(Default::default());
    // Use a link-local address for communication without DHCP server
    let config = Config::ipv4_static(embassy_net::StaticConfigV4 {
        address: embassy_net::Ipv4Cidr::new(embassy_net::Ipv4Address::new(169, 254, 1, 1), 16),
        dns_servers: heapless::Vec::from_slice(&[
            embassy_net::Ipv4Address::new(169, 254, 1, 100).into()
        ])
        .unwrap(),
        gateway: Some(embassy_net::Ipv4Address::new(169, 254, 1, 100)),
        // gateway: None,
    });

    // Generate random seed
    let seed = 0x0123_a5a7_83a4_fdef; // chosen by fair dice roll. guarenteed to be random.

    // Init network stack
    let stack = &*make_static!(Stack::new(
        net_device,
        config,
        make_static!(StackResources::<{ MAX_SOCKETS + 1 }>::new()),
        seed
    ));

    spawner.spawn(net_task(stack));

    //control.start_ap_open("cyw43", 5).await;
    control.start_ap_wpa2("nacra17", "password", 1).await;
    // control.start_ap_open("cyw43", 1).await;

    for _ in 0..MAX_SOCKETS {
        spawner.spawn(httpd_task(httpd, stack));
    }
    loop {
        Timer::after(Duration::from_secs(3)).await;
        log::info!("done");
    }
}

#[embassy_executor::task]
async fn reader(rx: UartRx<'static, UART1, Async>) {
    log::info!("Reading...");
    let mut parser = NMEAParser::<32>::new(rx);
    loop {
        let token = parser.next_token().await;
        match token {
            Some(NMEAMessage::GNRMC(gnrmc)) => {
                // log::info!("{:?}", gnrmc);
            }
            Some(NMEAMessage::Unknown) => {
                log::info!("Unknown");
            }
            None => {
                Timer::after(Duration::from_secs(1)).await;
                // log::info!("None");
            }
        }
    }
}
