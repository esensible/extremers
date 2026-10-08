//! dump -- print what the `settings` partition holds. Read-only.
//!
//!     cargo run --release --bin dump
//!
//! Flashing it replaces the app (flash the app again afterwards); the
//! settings partition is untouched by either flash. The password itself is
//! never printed, only its length.

#![no_std]
#![no_main]

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
// esp-rtos needs a global allocator to link. No heap is given to it, as in
// hilux/wireless-can's provision/dump, which run without one.
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;

#[allow(dead_code)] // shared with the app; each binary uses a subset
#[path = "../storage.rs"]
mod storage;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_defmt!();
    info!("=== dump: settings partition contents ===");

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    if !storage::init(peripherals.FLASH).await {
        warn!("dump: no settings partition on this chip");
    } else {
        match storage::wifi_creds().await {
            Some(c) => info!(
                "dump: wifi ssid {=[u8]:a}, password {=usize} bytes",
                c.ssid(),
                c.psk().len()
            ),
            None => match storage::value_len(storage::KEY_WIFI).await {
                Some(n) => warn!("dump: wifi: {=usize}-byte record that is not valid credentials", n),
                None => info!("dump: wifi: nothing stored (the app uses its compiled-in defaults)"),
            },
        }
    }
    info!("dump: done");
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
