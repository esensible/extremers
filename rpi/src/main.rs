//! This example uses the RP Pico W board Wifi chip (cyw43).
//! Creates an Access point Wifi network and creates a TCP endpoint on port 1234.

#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![allow(incomplete_features)]

mod consts;
use consts::*;
mod nmea_parser;
mod task_gps;
mod task_httpd;
mod task_sleeper;

// mod flash;
// use flash::PicoFlash;
// use littlefs2::fs::Filesystem;

use cyw43_pio::PioSpi;
use embassy_executor::Spawner;
use embassy_net::{Config, Stack, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::UART1;
use embassy_rp::peripherals::USB;
use embassy_rp::peripherals::{DMA_CH0, PIN_23, PIN_25, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::uart::{
    Async, Config as UartConfig, InterruptHandler as UartInterruptHandler, UartRx,
};
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};
use static_cell::make_static;

use {defmt_rtt as _, panic_probe as _};

use engine_race::RaceHttpd;

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

#[embassy_executor::task]
async fn logger_task(driver: Driver<'static, USB>) {
    embassy_usb_logger::run!(1024, log::LevelFilter::Info, driver);
}

struct UartReader(UartRx<'static, UART1, Async>);
impl nmea_parser::AsyncReader for UartReader {
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
        embassy_sync::blocking_mutex::raw::ThreadModeRawMutex,
        RaceHttpd,
    >,
    rx: UartRx<'static, UART1, Async>,
) {
    let mut ring_buffer = nmea_parser::RingBuffer::<UartReader, 32>::new(UartReader(rx));
    task_gps::gps_task_impl(httpd_mutex, &mut ring_buffer).await;
}

#[embassy_executor::task(pool_size = MAX_SOCKETS)]
pub async fn httpd_task(
    httpd_mutex: &'static embassy_sync::mutex::Mutex<
        embassy_sync::blocking_mutex::raw::ThreadModeRawMutex,
        RaceHttpd,
    >,
    stack: &'static embassy_net::Stack<cyw43::NetDriver<'static>>,
) -> ! {
    task_httpd::httpd_task_impl(httpd_mutex, stack).await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let httpd = make_static!(Mutex::<ThreadModeRawMutex, _>::new(RaceHttpd::default()));

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

    let state = make_static!(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    let result = spawner.spawn(wifi_task(runner));
    if result.is_err() {
        log::warn!("failed to spawn wifi task");
    }

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::Performance)
        .await;

    // let config = Config::dhcpv4(Default::default());
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

    let result = spawner.spawn(net_task(stack));
    if result.is_err() {
        log::warn!("failed to spawn net task");
    }

    control.start_ap_wpa2("nacra17", "password", 1).await;

    let result = spawner.spawn(task_sleeper::sleeper_task(httpd));
    if result.is_err() {
        log::warn!("failed to spawn sleeper task");
    }

    // int err = lfs_mount(&lfs, &cfg);

    // // reformat if we can't mount the filesystem
    // // this should only happen on the first boot
    // if (err) {
    //     lfs_format(&lfs, &cfg);
    //     lfs_mount(&lfs, &cfg);
    // }

    // // read current count
    // uint32_t boot_count = 0;
    // lfs_file_open(&lfs, &file, "boot_count", LFS_O_RDWR | LFS_O_CREAT);
    // lfs_file_read(&lfs, &file, &boot_count, sizeof(boot_count));

    //     let mut storage = PicoFlash {};
    //     let mut alloc_fs = Filesystem::allocate();

    //     let result = Filesystem::mount(&mut alloc_fs, &mut storage);
    //     let fs = match result {
    //         Ok(fs) => fs,
    //         Err(_) => {
    //             log::warn!("failed to mount filesystem");
    //             let result = Filesystem::format(&mut storage);
    //             if result.is_err() {
    //                 log::warn!("failed to format filesystem");
    //             }
    //             let result = Filesystem::mount(&mut alloc_fs, &mut storage);
    //             if result.is_err() {
    //                 log::warn!("failed to mount filesystem");
    //             }
    //             result.unwrap()
    //         }
    //     };

    //     let _result = fs.create_file_and_then(b"/tmp/test_open.txt\0".try_into().unwrap(), |file| {

    //         // can write to files
    //         assert!(file.write(&[0u8, 1, 2]).unwrap() == 3);
    //         file.sync()?;
    //         // surprise surprise, inline files!
    // //        assert_eq!(fs.available_blocks()?, 512 - 2 - 2);
    //         // no longer exists!
    //         // file.close()?;
    //         Ok(())
    //     });

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
