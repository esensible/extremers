//! This example uses the RP Pico W board Wifi chip (cyw43).
//! Creates an Access point Wifi network and creates a TCP endpoint on port 1234.

#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![allow(incomplete_features)]

// #[cfg(test)]
// mod tests;


use embassy_executor::Spawner;
use embassy_net::{Config, Stack, StackResources, StaticConfigV4, Ipv4Cidr, Ipv4Address};
use embassy_net::driver::Driver;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};

// use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{prelude::*, rng::Rng, timer::timg::TimerGroup, gpio::Io, uart::Uart, peripherals::UART0};
use esp_println::{print, println};
use esp_wifi::{
    init,
    
    wifi::{
        AccessPointConfiguration,
        Configuration,
        WifiApDevice,
        WifiController,
        WifiDevice,
        WifiEvent,
        WifiState,
        AuthMethod,
    },
    EspWifiInitFor,
};

use heapless::Vec;
use static_cell::StaticCell;

use engine_race::RaceHttpd;

// traits
use esp_hal::uart::UartRx;
use esp_hal::Async;

use lib_extreme_nostd::{
    gps_task_impl, httpd_task_impl, sleeper_task_impl, AsyncReader, RingBuffer, MAX_SOCKETS,
};


// #[embassy_executor::task]
// async fn logger_task(driver: Driver<'static, USB>) {
//     embassy_usb_logger::run!(1024, log::LevelFilter::Info, driver);
// }

struct UartReader(UartRx<'static, UART0, Async>);
impl AsyncReader for UartReader {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        match self.0.read_async(buf).await {
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
    rx: UartRx<'static, UART0, Async>,
) {
    let mut ring_buffer = RingBuffer::<UartReader, 32>::new(UartReader(rx));
    gps_task_impl(httpd_mutex, &mut ring_buffer).await;
}

#[embassy_executor::task(pool_size = MAX_SOCKETS)]
pub async fn httpd_task(
    httpd_mutex: &'static embassy_sync::mutex::Mutex<
        CriticalSectionRawMutex,
        RaceHttpd,
    >,
    stack: &'static embassy_net::Stack<WifiDevice<'static, WifiApDevice>>,
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

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {


    esp_println::logger::init_logger_from_env();
    let peripherals = esp_hal::init({
        let mut config = esp_hal::Config::default();
        config.cpu_clock = CpuClock::max();
        config
    });

    // esp_alloc::heap_allocator!(72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);

    let init = init(
        EspWifiInitFor::Wifi,
        timg0.timer0,
        Rng::new(peripherals.RNG),
        peripherals.RADIO_CLK,
    )
    .unwrap();

    let wifi = peripherals.WIFI;
    let (mut net_device, mut controller) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiApDevice).unwrap();

    use esp_hal::timer::systimer::{SystemTimer, Target};
    let systimer = SystemTimer::new(peripherals.SYSTIMER).split::<Target>();
    esp_hal_embassy::init(systimer.alarm0);

    let config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(embassy_net::Ipv4Address::new(169, 254, 1, 100), 24),
        gateway: Some(embassy_net::Ipv4Address::new(169, 254, 1, 100)),
        dns_servers: Default::default(),
    });

    let seed = 1234; // very random, very secure seed

    // Init network stack
    static RESOURCES: StaticCell<StackResources<{ MAX_SOCKETS + 1 }>> = StaticCell::new();
    static STACK: StaticCell<Stack<WifiDevice<'_, WifiApDevice>>> = StaticCell::new();
    let stack = Stack::new(net_device, config, RESOURCES.init(StackResources::new()), seed);
    let stack = STACK.init(stack);

    spawner.spawn(ap_task(controller)).ok();
    spawner.spawn(net_task(stack)).ok();



    type RaceHttpdMutex = Mutex::<CriticalSectionRawMutex, RaceHttpd>;

    static HTTPD: StaticCell<RaceHttpdMutex> = StaticCell::new();
    let httpd = HTTPD.init(RaceHttpdMutex::new(RaceHttpd::default()));

    let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);
    let (tx_pin, rx_pin) = (io.pins.gpio21, io.pins.gpio20);
    let config = esp_hal::uart::config::Config::default().baudrate(9600);
    let uart0 = UartRx::new_async_with_config(peripherals.UART0, config, rx_pin).unwrap();
    
    let result = spawner.spawn(gps_task(httpd, uart0));

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
async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiApDevice>>) -> ! {
    stack.run().await
}

#[embassy_executor::task]
async fn ap_task(mut controller: WifiController<'static>) {
    println!("start connection task");
    println!("Device capabilities: {:?}", controller.get_capabilities());
    loop {
        match esp_wifi::wifi::get_wifi_state() {
            WifiState::ApStarted => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::ApStop).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::AccessPoint(AccessPointConfiguration {
                ssid: "nacra17".try_into().unwrap(),
                password: "password".try_into().unwrap(),
                auth_method: AuthMethod::WPA2Personal,
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            println!("Starting wifi");
            controller.start().await.unwrap();
            println!("Wifi started!");
        }
    }
}