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

use embassy_time::{Duration, Timer};
// use embedded_io::Read;
use defmt::*;
use embedded_io_async::Read;
use embedded_io_async::Write;
use static_cell::make_static;

use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use {defmt_rtt as _, panic_probe as _};

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

#[embassy_executor::task(pool_size = 2)]
async fn buffer_task(stack: &'static Stack<cyw43::NetDriver<'static>>) -> ! {
    let mut rx_buffer = [0; 2048];
    let mut tx_buffer = [0; 2048];
    let mut buf = [0; 4096];

    let port = 1234;

    loop {
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));

        log::info!("Listening on TCP:1234...");
        if let Err(e) = socket.accept(port).await {
            // warn!("accept error: {:?}", e);
            continue;
        }

        log::info!("Received connection from {:?}", socket.remote_endpoint());

        loop {
            let n = match socket.read(&mut buf).await {
                Ok(0) => {
                    log::warn!("read EOF");
                    break;
                }
                Ok(n) => n,
                Err(e) => {
                    log::warn!("read error: {:?}", e);
                    break;
                }
            };

            let delay: Duration = Duration::from_secs(5);
            Timer::after(delay).await;

            let request_str = from_utf8(&buf[..n]).unwrap();
            if request_str.ends_with("\r\n\r\n") {
                let response = b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\nHello, World!";
                match socket.write_all(response).await {
                    Ok(()) => {}
                    Err(e) => {
                        log::warn!("write error: {:?}", e);
                        break;
                    }
                };
            }
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

    // let httpd = RaceHttpd::default();

    let p = embassy_rp::init(Default::default());

    let driver = Driver::new(p.USB, Irqs);
    spawner.spawn(logger_task(driver)).unwrap();

    let mut config = UartConfig::default();
    config.baudrate = 9600;
    let uart_rx = UartRx::new(p.UART1, p.PIN_9, Irqs, p.DMA_CH1, config);

    spawner.spawn(reader(uart_rx)).unwrap();

    // let fw = include_bytes!("/Users/esensible/src/extremers/rpi/cyw43-firmware/43439A0.bin");
    // let clm = include_bytes!("/Users/esensible/src/extremers/rpi/cyw43-firmware/43439A0_clm.bin");

    // // To make flashing faster for development, you may want to flash the firmwares independently
    // // at hardcoded addresses, instead of baking them into the program with `include_bytes!`:
    // //     probe-rs download 43439A0.bin --format bin --chip RP2040 --base-address 0x10100000
    // //     probe-rs download 43439A0_clm.bin --format bin --chip RP2040 --base-address 0x10140000
    // //let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    // //let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };

    // let pwr = Output::new(p.PIN_23, Level::Low);
    // let cs = Output::new(p.PIN_25, Level::High);
    // let mut pio = Pio::new(p.PIO0, Irqs);
    // let spi = PioSpi::new(
    //     &mut pio.common,
    //     pio.sm0,
    //     pio.irq0,
    //     cs,
    //     p.PIN_24,
    //     p.PIN_29,
    //     p.DMA_CH0,
    // );

    // let state = make_static!(cyw43::State::new());
    // let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    // spawner.spawn(wifi_task(runner));

    // control.init(clm).await;
    // control
    //     .set_power_management(cyw43::PowerManagementMode::PowerSave)
    //     .await;

    // // Use a link-local address for communication without DHCP server
    // let config = Config::ipv4_static(embassy_net::StaticConfigV4 {
    //     address: embassy_net::Ipv4Cidr::new(embassy_net::Ipv4Address::new(169, 254, 1, 1), 16),
    //     dns_servers: heapless::Vec::new(),
    //     gateway: None,
    // });

    // // Generate random seed
    // let seed = 0x0123_4567_89ab_cdef; // chosen by fair dice roll. guarenteed to be random.

    // // Init network stack
    // let stack = &*make_static!(Stack::new(
    //     net_device,
    //     config,
    //     make_static!(StackResources::<3>::new()),
    //     seed
    // ));

    // spawner.spawn(net_task(stack));

    // //control.start_ap_open("cyw43", 5).await;
    // control.start_ap_wpa2("cyw43", "password", 5).await;

    // let task_count = 2;

    // // And now we can use it!
    // for _ in 0..task_count {
    //     spawner.spawn(buffer_task(stack));
    // }
    loop {
        Timer::after(Duration::from_secs(1)).await;
        log::info!("done");
    }
}

// #[embassy_executor::task]
// async fn reader(mut rx: UartRx<'static, UART1, Async>) {
//     log::info!("Reading...");
//     loop {
//         let mut buf = [0; 32];
//         let result = rx.read(&mut buf).await;
//         match result {
//             Ok(len) => {
//                 // let s = core::str::from_utf8(&buf).unwrap_or("<invalid utf-8>");
//                 log::info!("Read {:?} bytes", len);
//             }
//             Err(_) => {
//                 log::info!("RX error");
//             }
//         }
//     }
// }

#[derive(Debug)]
enum Status {
    Active,
    Void,
    Unknown,
}

#[derive(Debug)]
enum Mode {
    Autonomous,
    Differential,
    Estimated,
    NotValid,
    Unknown,
}

#[derive(Default, Debug)]
struct GNRMC {
    utc_time: Option<f32>,
    status: Option<Status>,
    latitude: Option<f32>,
    ns_indicator: Option<char>,
    longitude: Option<f32>,
    ew_indicator: Option<char>,
    speed_over_ground: Option<f32>,
    course_over_ground: Option<u16>,
    date: Option<u64>,
    magnetic_variation: Option<f32>,
    ew_indicator_mag: Option<char>,
    mode: Option<Mode>,
}

enum NMEAMessage {
    GNRMC(GNRMC),
    Unknown,
}

struct RingBuffer<const N: usize> {
    reader: UartRx<'static, UART1, Async>,
    buf: [u8; N],
    read_ptr: usize,
}

impl<const N: usize> RingBuffer<N> {
    pub fn new(reader: UartRx<'static, UART1, Async>) -> Self {
        Self {
            reader,
            buf: [0; N],
            read_ptr: N,
        }
    }

    pub async fn next_token(&mut self) -> Option<&str> {
        let mut cursor = self.read_ptr;
        let old_ptr = self.read_ptr;

        let next_comma = loop {
            if cursor == N {
                let partial_len = N - self.read_ptr;
                if partial_len > 0 {
                    self.buf.copy_within(self.read_ptr.., 0);
                }
                cursor = partial_len;
                let result = self.reader.read(&mut self.buf[cursor..]).await;
                match result {
                    Ok(_) => {
                        // log::info!("read: {:?}", core::str::from_utf8(&self.buf));
                    }
                    Err(_) => {
                        log::info!("RX error");
                        return None;
                    }
                }
                self.read_ptr = 0;
            }
            if cursor > 0
                && (self.buf[cursor] == b','
                    || self.buf[cursor] == b'\n'
                    || self.buf[cursor] == b'*')
            {
                self.read_ptr = cursor + 1;
                break cursor;
            }
            cursor += 1;
        };

        if next_comma >= old_ptr {
            Some(core::str::from_utf8(&self.buf[old_ptr..next_comma]).unwrap())
        } else {
            Some(core::str::from_utf8(&self.buf[..next_comma]).unwrap())
        }
    }
}

struct NMEAParser<const N: usize>(RingBuffer<N>);

impl<const N: usize> NMEAParser<N> {
    pub fn new(rx: UartRx<'static, UART1, Async>) -> Self {
        Self(RingBuffer::new(rx))
    }

    pub async fn next_token(&mut self) -> Option<NMEAMessage> {
        let mut message = NMEAMessage::Unknown;
        let mut field = -1;

        loop {
            let token = self.0.next_token().await;
            if token.is_none() {
                log::warn!("No token");
                continue;
            }
            let token = token.unwrap();
            match &mut message {
                NMEAMessage::Unknown => {
                    if token == "$GNRMC" {
                        message = NMEAMessage::GNRMC(GNRMC::default());
                        field = -1;
                    } else if token.starts_with("$") {
                        log::info!("{}", token);
                    }
                }

                NMEAMessage::GNRMC(gnrmc) => {
                    match field {
                        0 => gnrmc.utc_time = token.parse::<f32>().ok(),
                        1 => {
                            gnrmc.status = Some(match token {
                                "A" => Status::Active,
                                "V" => Status::Void,
                                _ => Status::Unknown,
                            })
                        }
                        2 => gnrmc.latitude = token.parse::<f32>().ok(),
                        3 => gnrmc.ns_indicator = token.chars().next(),
                        4 => gnrmc.longitude = token.parse::<f32>().ok(),
                        5 => gnrmc.ew_indicator = token.chars().next(),
                        6 => gnrmc.speed_over_ground = token.parse::<f32>().ok(),
                        7 => gnrmc.course_over_ground = token.parse::<u16>().ok(),
                        8 => gnrmc.date = token.parse::<u64>().ok(),
                        9 => gnrmc.magnetic_variation = token.parse::<f32>().ok(),
                        10 => gnrmc.ew_indicator_mag = token.chars().next(),
                        11 => {
                            gnrmc.mode = Some(match token {
                                "A" => Mode::Autonomous,
                                "D" => Mode::Differential,
                                "E" => Mode::Estimated,
                                "N" => Mode::NotValid,
                                _ => Mode::Unknown,
                            })
                        }
                        12 => {
                            // checksum
                        }
                        _ => {
                            return Some(message);
                        }
                    }
                }
            }
            field += 1;
        }
    }
}

#[embassy_executor::task]
async fn reader(mut rx: UartRx<'static, UART1, Async>) {
    log::info!("Reading...");
    let mut parser = NMEAParser::<32>::new(rx);
    loop {
        let token = parser.next_token().await;
        match token {
            Some(NMEAMessage::GNRMC(gnrmc)) => {
                log::info!("{:?}", gnrmc);
            }
            Some(NMEAMessage::Unknown) => {
                log::info!("Unknown");
            }
            None => {
                log::info!("None");
            }
        }
    }
}
