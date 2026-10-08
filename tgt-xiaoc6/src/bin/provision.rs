//! provision -- erase the `settings` partition and write the WiFi credentials.
//! Also blanks `otadata`, so the bootloader boots `ota_0` (storage.rs,
//! `blank_otadata`).
//!
//! The app (`xiaoc6`) only reads settings. Erasing the partition and storing
//! the access point's SSID and password happen here, in a binary flashed on
//! purpose and replaced by the app afterwards. The credentials come from the
//! build environment, so they are never in the source:
//!
//!     EXTREMERS_SSID=nacra EXTREMERS_PSK='a passphrase' \
//!         cargo run --release --bin provision   # erase + write + read back
//!     cargo run --release                       # the app, which now uses them
//!
//! Give the variables to the same `cargo run` that flashes it: cargo
//! rebuilds when they change, so a bare `cargo run --bin provision` after a
//! build with them would rebuild without them and flash an erase-only
//! provision.
//!
//! Set both variables or neither; a length out of range (SSID 1..=32 bytes,
//! passphrase 8..=63) fails the build. With neither set it only erases the
//! partition, and the app falls back to the compiled-in defaults
//! (common/src/config.rs).

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

const SSID: Option<&str> = option_env!("EXTREMERS_SSID");
const PSK: Option<&str> = option_env!("EXTREMERS_PSK");

// Checked at build time: a bad value never reaches the board.
const _: () = match (SSID, PSK) {
    (Some(s), Some(p)) => assert!(
        !s.is_empty() && s.len() <= 32 && p.len() >= 8 && p.len() <= 63,
        "EXTREMERS_SSID must be 1..=32 bytes and EXTREMERS_PSK 8..=63 bytes"
    ),
    (None, None) => {}
    _ => panic!("set both EXTREMERS_SSID and EXTREMERS_PSK, or neither"),
};

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_defmt!();
    info!("=== provision: erase settings and otadata, write WiFi credentials ===");

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    if !storage::init(peripherals.FLASH).await {
        warn!("provision: no settings partition; nothing done");
        park().await
    }
    if !storage::format().await {
        warn!("provision: erase failed; nothing written");
        park().await
    }
    info!("provision: settings partition erased");

    // Blank otadata so the bootloader's choice is ota_0 by rule, not by
    // leftover bytes (storage::blank_otadata). Only when running from
    // ota_0: anywhere else, blanking it would switch the next boot.
    match storage::track_slot() {
        Some(t) if t.booted_subtype == 0x10 => {
            if storage::blank_otadata().await {
                info!("provision: otadata blanked (bootloader boots ota_0)");
            } else {
                warn!("provision: blanking otadata failed");
            }
        }
        _ => warn!("provision: not running from ota_0; otadata left as it is"),
    }

    match (SSID, PSK) {
        (Some(ssid), Some(psk)) => {
            // In range: checked at build time above.
            let creds = storage::WifiCreds::new(ssid.as_bytes(), psk.as_bytes()).unwrap();
            if storage::set_wifi_creds(&creds).await {
                info!("provision: stored ssid {=str}, password {=usize} bytes", ssid, psk.len());
            } else {
                warn!("provision: storing the credentials failed");
            }
        }
        _ => info!("provision: EXTREMERS_SSID/EXTREMERS_PSK not set at build time; nothing stored"),
    }

    // Read back through the path the app uses.
    match storage::wifi_creds().await {
        Some(c) if Some(c.ssid()) == SSID.map(str::as_bytes) && Some(c.psk()) == PSK.map(str::as_bytes) => {
            info!("provision: read back ssid {=[u8]:a}, password matches", c.ssid())
        }
        Some(c) => warn!("provision: read back ssid {=[u8]:a}, which is NOT what was written", c.ssid()),
        None if SSID.is_some() => warn!("provision: read back nothing, but credentials were written"),
        None => info!("provision: read back nothing stored (the app will use its defaults)"),
    }
    info!("provision: done -- flash the app now");
    park().await
}

async fn park() -> ! {
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
