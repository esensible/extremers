//! dump -- print what the `settings` partition holds (the boot package
//! among it), where the data slot's regions are, the GPS track's ring, and
//! the bundles region's extents. Read-only.
//!
//!     cargo run --release --bin dump
//!
//! Flashing it replaces the app (flash the app again afterwards); the
//! settings partition and the data slot are untouched by either flash. The
//! password itself is never printed, only its length; of BLE bonds only the
//! count and the devices' identity addresses, never their keys.

#![no_std]
#![no_main]

use common::{ble_settings::AddrText, store, track};
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
#[allow(dead_code)] // shared with the app
#[path = "../xip.rs"]
mod xip;

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
        // Addresses only: the keys (LTK, IRK) are never printed.
        let bonds = storage::bonds().await;
        info!(
            "dump: ble bonds: {=usize} of {=usize}",
            bonds.iter().flatten().count(),
            bonds.len()
        );
        for (slot, bond) in bonds.iter().enumerate() {
            if let Some(b) = bond {
                info!(
                    "dump: ble bond slot {=usize}: {} ({=str} identity, {=str}, seq {=u32})",
                    slot,
                    defmt::Display2Format(&AddrText(b.addr)),
                    if b.addr_kind == 0 { "public" } else { "random" },
                    if b.irk.is_some() { "IRK" } else { "no IRK" },
                    b.seq
                );
            }
        }
        match storage::boot_id().await {
            Some(0) => info!("dump: boot package: Choose"),
            Some(id) => info!("dump: boot package: {=u32:#010x}", id),
            None => match storage::value_len(storage::KEY_BOOT).await {
                Some(n) => warn!("dump: boot package: {=usize}-byte record, not 4", n),
                None => info!("dump: boot package: none kept (the only app, else Choose)"),
            },
        }
    }
    dump_track();
    dump_bundles();
    info!("dump: done");
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

/// The data slot's regions, and the track ring as the app's boot scan
/// finds it (`common::track::scan`: reads only; the app would also make the
/// head's sector free, `keep`) with its newest records.
fn dump_track() {
    let Some(slot) = storage::data_slot() else {
        warn!("dump: no data slot (see the storage lines above)");
        return;
    };
    let (b, t) = (slot.bundles(), slot.tracks());
    info!(
        "dump: data slot {=u32:#08x}+{=u32:#x}: bundles {=u32:#08x}..{=u32:#08x}, tracks {=u32:#08x}..{=u32:#08x}, spare from {=u32:#08x}",
        slot.offset,
        slot.len,
        b.start,
        b.end,
        t.start,
        t.end,
        t.end
    );
    if !xip::map(&slot) {
        return;
    }
    let Some(bytes) = xip::tracks_bytes(&slot) else {
        warn!("dump: tracks region not mapped");
        return;
    };
    let scan = track::scan(bytes);
    info!(
        "dump: track: {:?}, head {=u32} (the next boot's run starts here), oldest {=u32}, {=u32} of {=usize} records held, keep {:?}",
        defmt::Debug2Format(&scan.kind),
        scan.head,
        scan.oldest,
        scan.held,
        bytes.len() / track::RECORD_LEN,
        scan.keep
    );
    for back in 0..3 {
        if let Some(f) = scan.newest(bytes, back) {
            info!(
                "dump: track: newest-{=u32}: t {=i64}, lat {=f64}, lon {=f64}, alt {:?}",
                back,
                f.t,
                f.lat,
                f.lon,
                f.alt
            );
        }
    }
}

/// The bundles region's extents as the app's boot scan walks them
/// (`common::store::walk`: reads only), each with its manifest's name and
/// version where the archive has a readable one (not validated), and the
/// free sectors. The region's flash offset is where
/// `extreme-pack --store-image`'s image goes (README.md).
fn dump_bundles() {
    let Some(slot) = storage::data_slot() else {
        return;
    };
    let Some(bytes) = xip::bundles_bytes(&slot) else {
        warn!("dump: bundles region not mapped");
        return;
    };
    info!(
        "dump: bundles region at flash {=u32:#08x} (probe-rs download --binary-format bin --base-address {=u32:#x} bundles.img)",
        slot.bundles().start,
        slot.bundles().start
    );
    let (mut free, mut extents) = (0u32, 0u32);
    for item in store::walk(bytes) {
        match item {
            store::Item::Free { .. } => free += 1,
            store::Item::Extent { first, record: r } => {
                extents += 1;
                let at = store::archive_offset(first);
                let manifest = bytes
                    .get(at..at + r.len as usize)
                    .and_then(|a| bundle_manifest(a));
                match manifest {
                    Some((name, version)) => info!(
                        "dump: bundles: sectors {=u16}+{=u16}: {=str}, seq {=u32}, id {=u32:#010x}, {=u32} B: {=str} v{=u16}",
                        first,
                        r.sectors,
                        store::state_name(r.state),
                        r.seq,
                        r.id,
                        r.len,
                        name,
                        version
                    ),
                    None => info!(
                        "dump: bundles: sectors {=u16}+{=u16}: {=str}, seq {=u32}, id {=u32:#010x}, {=u32} B: no readable manifest",
                        first,
                        r.sectors,
                        store::state_name(r.state),
                        r.seq,
                        r.id,
                        r.len
                    ),
                }
            }
        }
    }
    info!("dump: bundles: {=u32} extents, {=u32} sectors without one", extents, free);
}

/// The name and version in an archive's `manifest.bin`, read without
/// validating anything else.
fn bundle_manifest(archive: &'static [u8]) -> Option<(&'static str, u16)> {
    let manifest = common::package::Package::manifest_of(archive)?;
    Some((manifest.name, manifest.version))
}
