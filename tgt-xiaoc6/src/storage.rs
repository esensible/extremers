//! Persistent settings in the `settings` flash partition, and where bundles
//! and GPS tracks go (the inactive app slot, see [`DataSlot`]).
//!
//! Layers, bottom up:
//!   `esp-storage`        the C6's internal SPI flash as a `NorFlash` (4-byte
//!                        writes, 4 KiB erases, ROM routines run with the
//!                        cache off and interrupts masked)
//!   `PagedFlash`         splits writes at the flash's 256-byte program pages
//!   `BlockingAsync`      embassy-embedded-hal's adapter to the async traits
//!   `sequential-storage` the key/value map: append-only items with CRCs, the
//!                        newest copy of a key wins, a 4 KiB page is erased
//!                        only when the map runs out of room and migrates
//!   this file            one-byte keys and the byte encoding of each record
//!
//! `settings` is declared in `partitions.csv` and found by label in the
//! partition table at boot -- no offsets in code. Flashing the app does not
//! touch it (see partitions.csv for why).
//!
//! The app writes at boot only after a firmware update (`KEY_OTA`'s boot
//! count) and to repair a corrupt `otadata` entry. It writes a bond record
//! when a device pairs during the pairing window and removes them all on
//! "Forget all watches" (BLE.md "Security"). Erasing the partition (WiFi credentials
//! and bonds alike) and writing the WiFi credentials is the `provision`
//! binary's job (src/bin/provision.rs); `dump` (src/bin/dump.rs) prints what
//! is stored.
//!
//! Records:
//!   `KEY_WIFI`           [ssid_len][ssid..][psk_len][psk..]   (`WifiCreds`)
//!   `KEY_BOND_BASE + n`  bond slot n < MAX_BLE_BONDS, 46 bytes
//!                        (`common::ble_settings::BondRecord`)
//!   `KEY_GATT_HASH`      the GATT database hash last served, 16 bytes
//!   `KEY_GATT_BASE + n`  bond slot n's GATT client state, 5 bytes
//!                        (`common::ble_settings::GattClientRecord`)
//!   `KEY_BOOT`           the package to boot with, u32 LE (a bundle's id;
//!                        0 = Choose), written by `Host::activate`; absent or
//!                        not installed: `Registry::boot_package`'s rule
//!   `KEY_OTA`            a firmware update on trial (`OtaRecord`): written
//!                        just before the switch to it, its boot count
//!                        raised at each boot of it, removed once it is
//!                        confirmed or rolled back (main.rs `ota_boot`)
//!
//! The `otadata` partition (which app slot the bootloader boots) has its own
//! flash handle ([`init_ota`], [`with_ota`]).

use core::ops::Range;
use core::sync::atomic::{AtomicBool, Ordering};

use common::{
    ble_settings::{BondRecord, GattClientRecord},
    config::{
        DATA_BUNDLES_LEN, DATA_BUNDLES_OFFSET, DATA_SLOT_USED, DATA_TRACKS_LEN, DATA_TRACKS_OFFSET,
        MAX_BLE_BONDS,
    },
    host::Identity,
};
use extreme_protocol::firmware;
use defmt::{info, warn};
use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embedded_storage_async::nor_flash::NorFlash as _;
use esp_bootloader_esp_idf::{
    ota::Ota,
    partitions::{self, AppPartitionSubType},
};
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;
use sequential_storage::cache::{Cache, Uncached};
use sequential_storage::map::{MapConfig, MapStorage};

pub type Key = [u8; 1];
pub const KEY_WIFI: Key = [0x10];
/// Bond slots: `KEY_BOND_BASE + slot`, slot < `MAX_BLE_BONDS`.
pub const KEY_BOND_BASE: u8 = 0x20;
/// The GATT database hash the bonded clients were last served.
pub const KEY_GATT_HASH: Key = [0x30];
/// Each bond slot's GATT client state: `KEY_GATT_BASE + slot`.
pub const KEY_GATT_BASE: u8 = 0x40;
/// The package to boot with (u32 LE; 0 = Choose).
pub const KEY_BOOT: Key = [0x50];
/// A firmware update on trial (`OtaRecord`).
pub const KEY_OTA: Key = [0x60];

/// sequential-storage's scratch buffer. It must hold the largest item of
/// ANY key plus its key, word-rounded: a page migration triggered by storing
/// one key copies every live item through it. Generous for today's largest
/// record (WiFi, ~100 bytes; a bond is 46), so a new key does not have to
/// resize it.
pub const SCRATCH_LEN: usize = 512;

/// Label of the settings partition in `partitions.csv`.
const PARTITION_LABEL: &str = "settings";
/// esp-storage's sector size: the map's page size and the erase unit.
const SECTOR: u32 = 4096;
/// The SPI flash's program page: one ROM page-program operation.
const FLASH_PAGE: u32 = 256;

/// esp-storage's `FlashStorage` with every write split at flash-page
/// boundaries.
///
/// esp-storage runs each ROM flash call inside a critical section that masks
/// every interrupt (the cache is off meanwhile, so no handler in flash may
/// run), and hands a write of up to 4 KiB to one ROM call. One page per call
/// bounds each masked window to a single page program, so the radio and the
/// GPS UART are serviced between pages. On hilux/wireless-can (same chip,
/// same crates) this took CAN frame loss during ~1.6 KB writes from 1-3
/// frames per write to none. The bytes written are the same; only the calls
/// differ. Erases cannot be split below a sector.
pub struct PagedFlash(FlashStorage<'static>);

impl PagedFlash {
    pub fn new(flash: FlashStorage<'static>) -> Self {
        PagedFlash(flash)
    }
}

impl embedded_storage::nor_flash::ErrorType for PagedFlash {
    type Error = <FlashStorage<'static> as embedded_storage::nor_flash::ErrorType>::Error;
}

impl embedded_storage::nor_flash::ReadNorFlash for PagedFlash {
    const READ_SIZE: usize =
        <FlashStorage<'static> as embedded_storage::nor_flash::ReadNorFlash>::READ_SIZE;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        embedded_storage::nor_flash::ReadNorFlash::read(&mut self.0, offset, bytes)
    }

    fn capacity(&self) -> usize {
        embedded_storage::nor_flash::ReadNorFlash::capacity(&self.0)
    }
}

impl embedded_storage::nor_flash::NorFlash for PagedFlash {
    const WRITE_SIZE: usize =
        <FlashStorage<'static> as embedded_storage::nor_flash::NorFlash>::WRITE_SIZE;
    const ERASE_SIZE: usize =
        <FlashStorage<'static> as embedded_storage::nor_flash::NorFlash>::ERASE_SIZE;

    fn write(&mut self, mut offset: u32, mut bytes: &[u8]) -> Result<(), Self::Error> {
        // Page boundaries are multiples of WRITE_SIZE (4), so every chunk
        // keeps the alignment the caller's write had.
        while !bytes.is_empty() {
            let n = ((FLASH_PAGE - offset % FLASH_PAGE) as usize).min(bytes.len());
            embedded_storage::nor_flash::NorFlash::write(&mut self.0, offset, &bytes[..n])?;
            offset += n as u32;
            bytes = &bytes[n..];
        }
        Ok(())
    }

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        embedded_storage::nor_flash::NorFlash::erase(&mut self.0, from, to)
    }
}

// Splitting a write changes nothing about what may be written over what.
impl embedded_storage::nor_flash::MultiwriteNorFlash for PagedFlash {}

type Flash = BlockingAsync<PagedFlash>;
type Map = MapStorage<Key, Flash, Cache<Uncached, Uncached, Uncached, Key>>;

struct Store {
    map: Map,
    /// sequential-storage's scratch (`SCRATCH_LEN`), in .bss via a
    /// `StaticCell` rather than inline in `STORE`.
    buf: &'static mut [u8; SCRATCH_LEN],
    /// The partition, kept so `format` can rebuild the map after erasing it.
    range: Range<u32>,
    /// The `otadata` partition, for `blank_otadata`.
    otadata: Option<Range<u32>>,
}

static STORE: Mutex<CriticalSectionRawMutex, Option<Store>> = Mutex::new(None);

/// WiFi access point credentials: an SSID of 1..=32 bytes and a WPA2
/// passphrase of 8..=63 bytes. Nothing else can be constructed or decoded.
#[derive(Clone, Copy)]
pub struct WifiCreds {
    ssid: [u8; 32],
    ssid_len: u8,
    psk: [u8; 63],
    psk_len: u8,
}

/// Longest encoded `WifiCreds`: two length bytes, a 32-byte SSID and a
/// 63-byte passphrase.
const WIFI_RECORD_MAX: usize = 2 + 32 + 63;

impl WifiCreds {
    pub const SSID_LEN: core::ops::RangeInclusive<usize> = 1..=32;
    /// WPA2's passphrase length (64 bytes would be a raw hex key instead).
    pub const PSK_LEN: core::ops::RangeInclusive<usize> = 8..=63;

    /// `None` unless both lengths are in range.
    pub fn new(ssid: &[u8], psk: &[u8]) -> Option<Self> {
        if !Self::SSID_LEN.contains(&ssid.len()) || !Self::PSK_LEN.contains(&psk.len()) {
            return None;
        }
        let mut c = Self {
            ssid: [0; 32],
            ssid_len: ssid.len() as u8,
            psk: [0; 63],
            psk_len: psk.len() as u8,
        };
        c.ssid[..ssid.len()].copy_from_slice(ssid);
        c.psk[..psk.len()].copy_from_slice(psk);
        Some(c)
    }

    pub fn ssid(&self) -> &[u8] {
        &self.ssid[..self.ssid_len as usize]
    }

    pub fn psk(&self) -> &[u8] {
        &self.psk[..self.psk_len as usize]
    }

    fn encode<'a>(&self, out: &'a mut [u8; WIFI_RECORD_MAX]) -> &'a [u8] {
        let s = self.ssid_len as usize;
        let p = self.psk_len as usize;
        out[0] = self.ssid_len;
        out[1..1 + s].copy_from_slice(self.ssid());
        out[1 + s] = self.psk_len;
        out[2 + s..2 + s + p].copy_from_slice(self.psk());
        &out[..2 + s + p]
    }

    fn decode(b: &[u8]) -> Option<Self> {
        let s = *b.first()? as usize;
        let ssid = b.get(1..1 + s)?;
        let p = *b.get(1 + s)? as usize;
        let psk = b.get(2 + s..2 + s + p)?;
        if b.len() != 2 + s + p {
            return None;
        }
        Self::new(ssid, psk)
    }
}

/// Locates the `settings` partition and opens the map. Reads only. Returns
/// `false` (and logs why) if there is no such partition, in which case every
/// accessor below reports "nothing stored" and every write fails.
pub async fn init(flash: FLASH<'static>) -> bool {
    let mut flash = FlashStorage::new(flash);
    info!(
        "storage: flash capacity {=usize:#x} (settings handle)",
        embedded_storage::nor_flash::ReadNorFlash::capacity(&flash)
    );

    let mut table_buf = [0u8; partitions::PARTITION_TABLE_MAX_LEN];
    let (range, otadata) = {
        let table = match partitions::read_partition_table(&mut flash, &mut table_buf) {
            Ok(t) => t,
            Err(e) => {
                warn!("storage: cannot read partition table: {:?}", defmt::Debug2Format(&e));
                return false;
            }
        };
        let mut found = None;
        let mut otadata = None;
        for p in table.iter() {
            // Raw type/subtype on purpose: `partition_type()` panics on a
            // type esp-bootloader-esp-idf has no name for.
            info!(
                "storage: partition {=str} type {=u8:#04x}/{=u8:#04x} at {=u32:#08x} len {=u32:#08x}",
                p.label_as_str(),
                p.raw_type(),
                p.raw_subtype(),
                p.offset(),
                p.len()
            );
            if p.label_as_str() == PARTITION_LABEL {
                found = Some(p.offset()..p.offset() + p.len());
            }
            if p.raw_type() == 1 && p.raw_subtype() == OTADATA_SUBTYPE {
                otadata = Some(p.offset()..p.offset() + p.len());
                let _ = OTADATA.init(p);
            }
        }
        locate_data_slot(&table, &mut flash);
        match found {
            Some(r) => (r, otadata),
            None => {
                warn!(
                    "storage: no '{=str}' partition -- flash with --idf-partition-table partitions.csv",
                    PARTITION_LABEL
                );
                return false;
            }
        }
    };

    let map = MapStorage::new(
        BlockingAsync::new(PagedFlash(flash)),
        MapConfig::new(range.clone()),
        Cache::new_uncached(),
    );
    static SCRATCH: static_cell::StaticCell<[u8; SCRATCH_LEN]> = static_cell::StaticCell::new();
    let buf = SCRATCH.init([0; SCRATCH_LEN]);
    *STORE.lock().await = Some(Store { map, buf, range, otadata });
    true
}

/// Where bundles and GPS tracks are kept: the app slot the running firmware
/// was NOT booted from.
///
/// The flash has two app slots, `ota_0` and `ota_1` (partitions.csv), and
/// no separate data partitions (the firmware is too big for a 1 MiB slot).
/// One slot holds the running firmware; the other holds, from its start
/// (common/src/config.rs):
///
///   +0x00000  bundles, 512 KiB (`DATA_BUNDLES_*`)
///   +0x80000  tracks, 640 KiB (`DATA_TRACKS_*`; `common::track::FlashTrack`)
///   +0x120000 spare, to the slot's end
///
/// Both are read in place through the flash MMU (src/xip.rs). A firmware
/// update over BLE (`common::firmware`, BLE.md "Firmware update") writes
/// the new image over the slot and so wipes both, by choice: bundles are
/// installed again (the studio saves and restores them) and tracks start
/// afresh, and afterwards the data lives in the other slot, which holds the
/// image the update came from. That one stays bootable while the new
/// firmware is on trial (nothing is written to the slot until it is
/// confirmed: main.rs `ota_boot`), and is the rollback; once data is
/// written to it, it is not any more.
#[derive(Clone, Copy)]
pub struct DataSlot {
    /// The running slot's subtype: 0x10 + n for `ota_n`.
    pub booted_subtype: u8,
    /// The inactive slot's subtype.
    pub subtype: u8,
    /// Flash offset of the inactive slot.
    pub offset: u32,
    /// Its size in bytes.
    pub len: u32,
}

impl DataSlot {
    /// The bundles region, as flash offsets.
    pub fn bundles(&self) -> Range<u32> {
        let start = self.offset + DATA_BUNDLES_OFFSET;
        start..start + DATA_BUNDLES_LEN
    }

    /// The tracks region, as flash offsets.
    pub fn tracks(&self) -> Range<u32> {
        let start = self.offset + DATA_TRACKS_OFFSET;
        start..start + DATA_TRACKS_LEN
    }
}

static DATA_SLOT: embassy_sync::once_lock::OnceLock<DataSlot> =
    embassy_sync::once_lock::OnceLock::new();

/// The inactive app slot, as found by [`init`]; `None` if it could not be
/// determined or is too small for the regions (logged at boot).
pub fn data_slot() -> Option<DataSlot> {
    DATA_SLOT.try_get().copied()
}

/// App partition subtypes `ota_0`..`ota_15` are 0x10..=0x1F; 0x00 is
/// factory, 0x20 test.
const OTA_SUBTYPES: core::ops::RangeInclusive<u8> = 0x10..=0x1F;
/// Data partition subtype of `otadata`.
const OTADATA_SUBTYPE: u8 = 0x00;

/// Finds the running slot and the other OTA app slot, logs both, and keeps
/// the inactive one for [`data_slot`]. Reads only.
///
/// The running slot comes from `booted_partition()`, which reads the MMU
/// entry that maps the start of the app's flash address space: that is
/// the slot actually executing, whatever `otadata` says. `otadata`'s
/// selection is logged next to it as a cross-check (all-0xFF `otadata`
/// selects nothing, and the bootloader then boots `ota_0`).
fn locate_data_slot(table: &partitions::PartitionTable<'_>, flash: &mut FlashStorage<'static>) {
    let booted = match table.booted_partition() {
        Ok(Some(p)) => p,
        Ok(None) => {
            warn!("storage: running app slot not found in the partition table");
            return;
        }
        Err(e) => {
            warn!("storage: cannot determine the running app slot: {:?}", defmt::Debug2Format(&e));
            return;
        }
    };
    // Raw type/subtype: see `init`.
    let is_ota_app = |p: &partitions::PartitionEntry| p.raw_type() == 0 && OTA_SUBTYPES.contains(&p.raw_subtype());
    if !is_ota_app(&booted) {
        warn!(
            "storage: running from {=str} (subtype {=u8:#04x}), not an OTA slot; no data slot",
            booted.label_as_str(),
            booted.raw_subtype()
        );
        return;
    }
    identify(&booted, flash);

    let mut ota_slots = 0usize;
    let mut otadata = None;
    let mut inactive = None;
    for p in table.iter() {
        if is_ota_app(&p) {
            ota_slots += 1;
            if p.offset() != booted.offset() {
                inactive = Some(p);
            }
        } else if p.raw_type() == 1 && p.raw_subtype() == OTADATA_SUBTYPE {
            otadata = Some(p);
        }
    }

    match otadata {
        Some(p) => log_otadata(p, ota_slots, flash),
        None => warn!("storage: no otadata partition"),
    }

    let Some(slot) = inactive.filter(|_| ota_slots == 2) else {
        warn!("storage: {=usize} OTA app slots, expected 2; no data slot", ota_slots);
        return;
    };
    info!(
        "storage: running app slot {=str}; inactive slot {=str} at {=u32:#08x}, {=u32} bytes: the data slot",
        booted.label_as_str(),
        slot.label_as_str(),
        slot.offset(),
        slot.len()
    );
    // whole MMU pages, room for both regions
    if slot.offset() % 0x1_0000 != 0 || slot.len() < DATA_SLOT_USED {
        warn!(
            "storage: data slot not 64 KiB aligned or under {=u32} bytes; no data slot",
            DATA_SLOT_USED
        );
        return;
    }
    let _ = DATA_SLOT.init(DataSlot {
        booted_subtype: booted.raw_subtype(),
        subtype: slot.raw_subtype(),
        offset: slot.offset(),
        len: slot.len(),
    });
}

static RUNNING: embassy_sync::once_lock::OnceLock<Identity> = embassy_sync::once_lock::OnceLock::new();

/// The running firmware, as [`init`] found it (`firmware: running ...` at
/// boot), with the data slot's length as the largest update; `None` if its
/// image did not check out (logged).
pub fn running_identity() -> Option<Identity> {
    let running = RUNNING.try_get()?;
    Some(Identity { max_len: data_slot().map_or(0, |s| s.len), ..*running })
}

/// Walks the image in the running slot `booted` (the app image format,
/// `extreme_protocol::firmware`) and keeps its appended digest, its
/// identity, for [`running_identity`]. Reads only. The digest is not
/// checked against the image: the bootloader did that before booting it.
fn identify(booted: &partitions::PartitionEntry, flash: &mut FlashStorage<'static>) {
    let started = esp_hal::time::Instant::now();
    let (offset, len) = (booted.offset(), booted.len());
    let read = |at: u32, buf: &mut [u8]| embedded_storage::nor_flash::ReadNorFlash::read(flash, offset + at, buf);
    let image = match firmware::walk(len, read) {
        Ok(image) => image,
        Err((reason, detail)) => {
            warn!(
                "firmware: the image in {=str} does not check out (reason {=u8}, detail {=u32:#x}); no identity",
                booted.label_as_str(),
                reason,
                detail
            );
            return;
        }
    };
    let mut sha256 = [0u8; 32];
    if let Err(e) = embedded_storage::nor_flash::ReadNorFlash::read(flash, offset + image.digest_at, &mut sha256) {
        warn!("firmware: cannot read the digest: {:?}; no identity", defmt::Debug2Format(&e));
        return;
    }
    let slot = booted.raw_subtype() - OTA_SUBTYPES.start();
    let mut hex = [0u8; 64];
    for (i, b) in sha256.iter().enumerate() {
        hex[2 * i] = b"0123456789abcdef"[(b >> 4) as usize];
        hex[2 * i + 1] = b"0123456789abcdef"[(b & 0xF) as usize];
    }
    info!(
        "firmware: running ota_{=u8}, {=u32} B, sha256 {=str}, version {=str} (walked in {=u64} us)",
        slot,
        image.len(),
        core::str::from_utf8(&hex).unwrap_or("?"),
        crate::ESP_APP_DESC.version(),
        started.elapsed().as_micros()
    );
    let _ = RUNNING.init(Identity { sha256, slot, len: image.len(), max_len: 0 });
}

static OTADATA_SELECTS: embassy_sync::once_lock::OnceLock<u8> = embassy_sync::once_lock::OnceLock::new();

/// The app slot `otadata` selected at boot (n of `ota_n`; blank selects
/// `ota_0`); `None` if it could not be read.
pub fn otadata_selects() -> Option<u8> {
    OTADATA_SELECTS.try_get().copied()
}

/// Logs which slot `otadata` selects, with each sector's `ota_seq` and
/// state. Reads only.
///
/// Each of the two 4 KiB sectors starts with a 32-byte select entry; the
/// higher valid `ota_seq` wins (seq n selects slot (n - 1) mod 2). Both
/// all-0xFF selects nothing: the bootloader then boots `ota_0`, and records
/// that by writing `ota_seq` 1 into the first sector ("Set actual ota_seq=1
/// in otadata[0]"; seen on this board after `provision` blanked otadata).
/// An entry that is neither all-0xFF nor CRC-valid makes esp-bootloader-
/// esp-idf reject the whole of otadata, so the raw entries are dumped.
fn log_otadata(p: partitions::PartitionEntry, ota_slots: usize, flash: &mut FlashStorage<'static>) {
    let mut entries = [[0u8; 32]; 2];
    for (sector, entry) in entries.iter_mut().enumerate() {
        let at = p.offset() + sector as u32 * SECTOR;
        if let Err(e) = embedded_storage::nor_flash::ReadNorFlash::read(flash, at, entry) {
            warn!("storage: cannot read otadata at {=u32:#08x}: {:?}", at, defmt::Debug2Format(&e));
            return;
        }
    }
    let seq = |e: &[u8; 32]| u32::from_le_bytes([e[0], e[1], e[2], e[3]]);
    let state = |e: &[u8; 32]| u32::from_le_bytes([e[24], e[25], e[26], e[27]]);
    let selected = esp_bootloader_esp_idf::ota::Ota::new(p.as_flash_region(flash), ota_slots)
        .and_then(|mut ota| ota.current_app_partition());
    match selected {
        // `current_app_partition` reports blank otadata as Factory.
        Ok(partitions::AppPartitionSubType::Factory) => {
            let _ = OTADATA_SELECTS.init(0);
            info!("storage: otadata blank, selects no slot (the bootloader boots ota_0)")
        }
        Ok(s) => {
            let _ = OTADATA_SELECTS.init(s as u8 - OTA_SUBTYPES.start());
            info!(
                "storage: otadata selects {:?} (ota_seq {=u32:#x} / {=u32:#x}, state {=u32:#x} / {=u32:#x})",
                defmt::Debug2Format(&s),
                seq(&entries[0]),
                seq(&entries[1]),
                state(&entries[0]),
                state(&entries[1])
            )
        }
        Err(e) => {
            warn!(
                "storage: cannot read otadata: {:?} (the `provision` binary blanks it)",
                defmt::Debug2Format(&e)
            );
            for (sector, entry) in entries.iter().enumerate() {
                warn!("storage: otadata sector {=usize} entry: {=[u8]:02x}", sector, &entry[..]);
            }
        }
    }
}

/// Erases the whole settings partition, one 4 KiB sector at a time. Only the
/// `provision` binary calls this.
///
/// Sector by sector on purpose: an erase of the whole range is turned by
/// esp-storage into one ROM 64 KiB block erase, which faulted inside ROM on
/// one of two XIAO C6 boards in hilux/wireless-can. The map itself only
/// ever erases sectors, so this keeps provisioning on the same path.
pub async fn format() -> bool {
    erase_sectors(|s| Some(s.range.clone())).await
}

/// Erases `otadata` to all 0xFF, so that the bootloader boots `ota_0` (the
/// slot probe-rs flashes). Only the `provision` binary calls this.
///
/// Needed on a board flashed before this layout: its old app image started
/// at 0x10000, which is now `otadata`'s second sector, so that sector began
/// with an image header (0xE9 ...) rather than a select entry. The
/// bootloader rejects such an entry and falls back to `ota_0` anyway, but
/// the selection should not rest on leftover bytes failing a CRC.
pub async fn blank_otadata() -> bool {
    erase_sectors(|s| s.otadata.clone()).await
}

/// Erases the sectors of the range `which` picks, one 4 KiB sector at a
/// time (see `format`), with the map taken apart meanwhile.
async fn erase_sectors(which: impl FnOnce(&Store) -> Option<Range<u32>>) -> bool {
    let mut guard = STORE.lock().await;
    let Some(store) = guard.take() else {
        return false;
    };
    let target = which(&store);
    let Store { map, buf, range, otadata } = store;
    let (mut flash, cache) = map.destroy();
    let mut ok = target.is_some();
    if let Some(target) = target {
        let mut a = target.start;
        while a < target.end {
            if let Err(e) = flash.erase(a, a + SECTOR).await {
                warn!("storage: erase sector at {=u32:#x} failed: {:?}", a, defmt::Debug2Format(&e));
                ok = false;
                break;
            }
            a += SECTOR;
        }
    }
    *guard = Some(Store {
        map: MapStorage::new(flash, MapConfig::new(range.clone()), cache),
        buf,
        range,
        otadata,
    });
    ok
}

/// Runs `f` on the stored value for `key` (`None` if absent, unreadable or
/// no partition) while the map is locked, so the value is used in place
/// rather than copied out. `f` must not touch storage.
pub async fn with_value<R>(key: Key, f: impl FnOnce(Option<&[u8]>) -> R) -> R {
    let mut guard = STORE.lock().await;
    let Some(store) = guard.as_mut() else {
        return f(None);
    };
    match store.map.fetch_item::<&[u8]>(&mut store.buf[..], &key).await {
        Ok(v) => f(v),
        Err(e) => {
            warn!("storage: fetch {=u8:#04x} failed: {:?}", key[0], defmt::Debug2Format(&e));
            f(None)
        }
    }
}

/// Length of the stored value for `key`, for the `dump` binary.
pub async fn value_len(key: Key) -> Option<usize> {
    with_value(key, |v| v.map(|v| v.len())).await
}

async fn store(key: Key, value: &[u8]) -> bool {
    let mut guard = STORE.lock().await;
    let Some(store) = guard.as_mut() else {
        warn!("storage: not initialised; {=u8:#04x} not stored", key[0]);
        return false;
    };
    match store.map.store_item(&mut store.buf[..], &key, &value).await {
        Ok(()) => true,
        Err(e) => {
            warn!("storage: store {=u8:#04x} failed: {:?}", key[0], defmt::Debug2Format(&e));
            false
        }
    }
}

/// The stored WiFi credentials; `None` if none are stored or the record does
/// not decode to valid ones (logged).
pub async fn wifi_creds() -> Option<WifiCreds> {
    with_value(KEY_WIFI, |v| {
        let v = v?;
        let c = WifiCreds::decode(v);
        if c.is_none() {
            warn!("storage: stored wifi record ({=usize} bytes) is not valid credentials", v.len());
        }
        c
    })
    .await
}

pub async fn set_wifi_creds(creds: &WifiCreds) -> bool {
    let mut raw = [0u8; WIFI_RECORD_MAX];
    store(KEY_WIFI, creds.encode(&mut raw)).await
}

fn bond_key(slot: usize) -> Key {
    [KEY_BOND_BASE + slot as u8]
}

/// The stored bonds by slot. A slot whose record does not decode is logged
/// and read as empty (and is overwritten by the next pairing that needs it).
pub async fn bonds() -> [Option<BondRecord>; MAX_BLE_BONDS] {
    let mut out = [None; MAX_BLE_BONDS];
    for (slot, entry) in out.iter_mut().enumerate() {
        *entry = with_value(bond_key(slot), |v| {
            let v = v?;
            let r = BondRecord::decode(v);
            if r.is_none() {
                warn!("storage: bond slot {=usize}: {=usize}-byte record does not decode", slot, v.len());
            }
            r
        })
        .await;
    }
    out
}

pub async fn store_bond(slot: usize, bond: &BondRecord) -> bool {
    if slot >= MAX_BLE_BONDS {
        return false;
    }
    store(bond_key(slot), &bond.encode()).await
}

/// Removes every bond slot and its GATT client record. `true` if all
/// removals succeeded (removing an absent key succeeds).
pub async fn clear_bonds() -> bool {
    let mut guard = STORE.lock().await;
    let Some(store) = guard.as_mut() else {
        return false;
    };
    let mut ok = true;
    for slot in 0..MAX_BLE_BONDS {
        for key in [bond_key(slot), gatt_key(slot)] {
            if let Err(e) = store.map.remove_item(&mut store.buf[..], &key).await {
                warn!("storage: remove {=u8:#04x} failed: {:?}", key[0], defmt::Debug2Format(&e));
                ok = false;
            }
        }
    }
    ok
}

fn gatt_key(slot: usize) -> Key {
    [KEY_GATT_BASE + slot as u8]
}

/// Every bond slot's GATT client record (a record that does not decode is
/// logged and read as absent), and the database hash last served.
pub async fn gatt() -> ([Option<GattClientRecord>; MAX_BLE_BONDS], Option<[u8; 16]>) {
    let mut records = [None; MAX_BLE_BONDS];
    for (slot, entry) in records.iter_mut().enumerate() {
        *entry = with_value(gatt_key(slot), |v| {
            let v = v?;
            let r = GattClientRecord::decode(v);
            if r.is_none() {
                warn!("storage: GATT slot {=usize}: {=usize}-byte record does not decode", slot, v.len());
            }
            r
        })
        .await;
    }
    let hash = with_value(KEY_GATT_HASH, |v| v.and_then(|v| <[u8; 16]>::try_from(v).ok())).await;
    (records, hash)
}

pub async fn store_gatt(slot: usize, record: &GattClientRecord) -> bool {
    if slot >= MAX_BLE_BONDS {
        return false;
    }
    store(gatt_key(slot), &record.encode()).await
}

pub async fn store_gatt_hash(hash: &[u8; 16]) -> bool {
    store(KEY_GATT_HASH, hash).await
}

/// The package to boot with (`KEY_BOOT`); `None` if none is stored or the
/// record is not 4 bytes (logged).
pub async fn boot_id() -> Option<u32> {
    with_value(KEY_BOOT, |v| {
        let v = v?;
        let id = <[u8; 4]>::try_from(v).ok().map(u32::from_le_bytes);
        if id.is_none() {
            warn!("storage: boot record is {=usize} bytes, not 4; ignored", v.len());
        }
        id
    })
    .await
}

pub async fn store_boot_id(id: u32) -> bool {
    store(KEY_BOOT, &id.to_le_bytes()).await
}

// ---- firmware updates ----

/// A firmware update on trial (`KEY_OTA`): the slot it came from and the
/// one it went to (n of `ota_n`), the image's appended digest, and how often
/// it has booted without confirming itself.
#[derive(Clone, Copy, Debug)]
pub struct OtaRecord {
    pub tries: u8,
    pub from: u8,
    pub to: u8,
    pub digest: [u8; 32],
}

impl OtaRecord {
    const LEN: usize = 3 + 32;

    fn encode(&self) -> [u8; Self::LEN] {
        let mut b = [0u8; Self::LEN];
        b[..3].copy_from_slice(&[self.tries, self.from, self.to]);
        b[3..].copy_from_slice(&self.digest);
        b
    }

    fn decode(b: &[u8]) -> Option<Self> {
        let b: &[u8; Self::LEN] = b.try_into().ok()?;
        Some(OtaRecord { tries: b[0], from: b[1], to: b[2], digest: b[3..].try_into().ok()? })
    }
}

/// The update on trial (`KEY_OTA`); `None` if there is none, or the record
/// does not decode (logged).
pub async fn ota_record() -> Option<OtaRecord> {
    with_value(KEY_OTA, |v| {
        let v = v?;
        let r = OtaRecord::decode(v);
        if r.is_none() {
            warn!("storage: update record is {=usize} bytes, not {=usize}; ignored", v.len(), OtaRecord::LEN);
        }
        r
    })
    .await
}

pub async fn store_ota_record(record: &OtaRecord) -> bool {
    store(KEY_OTA, &record.encode()).await
}

/// Removes the update record (removing an absent one succeeds).
pub async fn clear_ota_record() -> bool {
    let mut guard = STORE.lock().await;
    let Some(store) = guard.as_mut() else {
        return false;
    };
    match store.map.remove_item(&mut store.buf[..], &KEY_OTA).await {
        Ok(()) => true,
        Err(e) => {
            warn!("storage: remove {=u8:#04x} failed: {:?}", KEY_OTA[0], defmt::Debug2Format(&e));
            false
        }
    }
}

/// The `otadata` partition's entry, as [`init`] found it.
static OTADATA: embassy_sync::once_lock::OnceLock<partitions::PartitionEntry> = embassy_sync::once_lock::OnceLock::new();
/// The flash handle `otadata` is written through ([`init_ota`]).
static OTA_FLASH: Mutex<CriticalSectionRawMutex, Option<FlashStorage<'static>>> = Mutex::new(None);

/// Set while the firmware runs on trial after an update (main.rs
/// `ota_boot`): a panic then resets the chip, so that the boot count sees
/// it, rather than halting for a debugger as it otherwise does.
pub static RESET_ON_PANIC: AtomicBool = AtomicBool::new(false);

/// esp-backtrace's halt (feature `custom-halt`), after the panic is logged:
/// on trial ([`RESET_ON_PANIC`]) a reset, a second after the panic so the
/// log gets out over RTT; else the halt esp-backtrace has without it.
#[unsafe(no_mangle)]
fn custom_halt() -> ! {
    if RESET_ON_PANIC.load(Ordering::Relaxed) {
        let start = esp_hal::time::Instant::now();
        while start.elapsed() < esp_hal::time::Duration::from_millis(1000) {}
        esp_hal::system::software_reset();
    }
    #[allow(clippy::empty_loop)]
    loop {}
}

/// App slot `n` (`ota_n`) as the OTA API names it.
pub fn app_slot(n: u8) -> Option<AppPartitionSubType> {
    AppPartitionSubType::try_from(OTA_SUBTYPES.start() + n).ok()
}

/// Whether a 32-byte `otadata` select entry is one the bootloader (and
/// esp-bootloader-esp-idf) takes: blank, or a known state with the CRC of
/// its `ota_seq` (`crc32_le(u32::MAX, seq)`, as both compute it).
fn entry_valid(e: &[u8; 32]) -> bool {
    let word = |i: usize| u32::from_le_bytes([e[i], e[i + 1], e[i + 2], e[i + 3]]);
    let (seq, state, crc) = (word(0), word(24), word(28));
    if seq == u32::MAX && state == u32::MAX {
        return true;
    }
    (state <= 4 || state == u32::MAX) && crc == esp_hal::rom::crc::crc32_le(u32::MAX, &e[..4])
}

/// Erases each `otadata` sector whose entry is corrupt (neither blank nor
/// CRC-valid; the bootloader ignores such an entry, but esp-bootloader-
/// esp-idf refuses the whole of `otadata` for it), logging it: a write cut
/// off by a reset leaves that. The other entry then selects, as it already
/// did for the bootloader. `false` if a read or an erase failed.
fn repair_otadata(entry: &partitions::PartitionEntry, flash: &mut FlashStorage<'static>) -> bool {
    use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
    for sector in 0..2u32 {
        let at = entry.offset() + sector * SECTOR;
        let mut e = [0u8; 32];
        if let Err(err) = ReadNorFlash::read(flash, at, &mut e) {
            warn!("otadata: cannot read sector {=u32}: {:?}", sector, defmt::Debug2Format(&err));
            return false;
        }
        if entry_valid(&e) {
            continue;
        }
        warn!("otadata: sector {=u32} holds a corrupt entry {=[u8]:02x}: erasing it", sector, &e[..]);
        if let Err(err) = NorFlash::erase(flash, at, at + SECTOR) {
            warn!("otadata: erasing sector {=u32} failed: {:?}", sector, defmt::Debug2Format(&err));
            return false;
        }
    }
    true
}

/// Takes `flash` as the handle `otadata` is written through ([`with_ota`]),
/// after [`init`] found the partition, and repairs a corrupt entry
/// (`repair_otadata`). `false` (logged) without an `otadata` partition.
pub async fn init_ota(mut flash: FlashStorage<'static>) -> bool {
    let Some(entry) = OTADATA.try_get() else {
        warn!("otadata: no partition: no firmware updates");
        return false;
    };
    let ok = repair_otadata(entry, &mut flash);
    *OTA_FLASH.lock().await = Some(flash);
    ok
}

/// Runs `f` on `otadata` (esp-bootloader-esp-idf's [`Ota`], two app
/// slots), its entries checked first and again after (`repair_otadata`), so
/// it is never left with one corrupt. `None` (logged as `what` failing) if
/// there is no `otadata` or `f` failed.
pub async fn with_ota<R>(what: &str, f: impl FnOnce(&mut Ota<'_, 'static>) -> Result<R, partitions::Error>) -> Option<R> {
    let mut guard = OTA_FLASH.lock().await;
    let (Some(flash), Some(entry)) = (guard.as_mut(), OTADATA.try_get()) else {
        warn!("otadata: {=str}: no otadata", what);
        return None;
    };
    if !repair_otadata(entry, flash) {
        return None;
    }
    let done = Ota::new(entry.as_flash_region(flash), 2).and_then(|mut ota| f(&mut ota));
    repair_otadata(entry, flash);
    match done {
        Ok(r) => Some(r),
        Err(e) => {
            warn!("otadata: {=str} failed: {:?}", what, defmt::Debug2Format(&e));
            None
        }
    }
}
