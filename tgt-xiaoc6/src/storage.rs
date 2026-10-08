//! Persistent settings in the `settings` flash partition, and where GPS
//! tracks will go (the inactive app slot, see [`TrackSlot`]).
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
//! The app only reads: it never writes or erases at boot. Erasing the
//! partition and writing the WiFi credentials is the `provision` binary's
//! job (src/bin/provision.rs); `dump` (src/bin/dump.rs) prints what is
//! stored.
//!
//! Records:
//!   `KEY_WIFI`  [ssid_len][ssid..][psk_len][psk..]   (`WifiCreds`)

use core::ops::Range;

use defmt::{info, warn};
use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embedded_storage_async::nor_flash::NorFlash as _;
use esp_bootloader_esp_idf::partitions;
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;
use sequential_storage::cache::{Cache, Uncached};
use sequential_storage::map::{MapConfig, MapStorage};

pub type Key = [u8; 1];
pub const KEY_WIFI: Key = [0x10];

/// sequential-storage's scratch buffer. It must hold the largest item of
/// ANY key plus its key, word-rounded: a page migration triggered by storing
/// one key copies every live item through it. Generous for today's single
/// ~100-byte record, so a new key does not have to resize it.
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
            }
        }
        locate_track_slot(&table, &mut flash);
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

/// Where GPS tracks are to be kept: the app slot the running firmware was
/// NOT booted from.
///
/// The flash has two app slots, `ota_0` and `ota_1` (partitions.csv), and
/// no separate tracks partition. One slot holds the running firmware; the
/// other is free space until the next update. The scheme (none of it is
/// implemented yet):
///
/// - while sailing, track points are appended to the inactive slot;
/// - they are downloaded over BLE, after which the slot may be erased;
/// - an OTA update erases the inactive slot and writes the new firmware
///   into it, then selects it in `otadata`.
///
/// So an update requires the tracks to have been downloaded first, and once
/// tracks have been written over the previous firmware image there is no
/// rollback to it: the inactive slot never holds a bootable fallback.
///
/// Nothing in this crate writes or erases the slot yet; this only reports
/// where it is.
#[derive(Clone, Copy)]
pub struct TrackSlot {
    /// The running slot's subtype: 0x10 + n for `ota_n`.
    pub booted_subtype: u8,
    /// The inactive slot's subtype.
    pub subtype: u8,
    /// Flash offset of the inactive slot.
    pub offset: u32,
    /// Its size in bytes.
    pub len: u32,
}

static TRACK_SLOT: embassy_sync::once_lock::OnceLock<TrackSlot> =
    embassy_sync::once_lock::OnceLock::new();

/// The inactive app slot, as found by [`init`]; `None` if it could not be
/// determined (logged at boot).
pub fn track_slot() -> Option<TrackSlot> {
    TRACK_SLOT.try_get().copied()
}

/// App partition subtypes `ota_0`..`ota_15` are 0x10..=0x1F; 0x00 is
/// factory, 0x20 test.
const OTA_SUBTYPES: core::ops::RangeInclusive<u8> = 0x10..=0x1F;
/// Data partition subtype of `otadata`.
const OTADATA_SUBTYPE: u8 = 0x00;

/// Finds the running slot and the other OTA app slot, logs both, and keeps
/// the inactive one for [`track_slot`]. Reads only.
///
/// The running slot comes from `booted_partition()`, which reads the MMU
/// entry that maps the start of the app's flash address space: that is
/// the slot actually executing, whatever `otadata` says. `otadata`'s
/// selection is logged next to it as a cross-check (all-0xFF `otadata`
/// selects nothing, and the bootloader then boots `ota_0`).
fn locate_track_slot(table: &partitions::PartitionTable<'_>, flash: &mut FlashStorage<'static>) {
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
            "storage: running from {=str} (subtype {=u8:#04x}), not an OTA slot; no track slot",
            booted.label_as_str(),
            booted.raw_subtype()
        );
        return;
    }

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
        warn!("storage: {=usize} OTA app slots, expected 2; no track slot", ota_slots);
        return;
    };
    info!(
        "storage: running app slot {=str}; inactive slot {=str} at {=u32:#08x}, {=u32} bytes (reserved for tracks, untouched)",
        booted.label_as_str(),
        slot.label_as_str(),
        slot.offset(),
        slot.len()
    );
    let _ = TRACK_SLOT.init(TrackSlot {
        booted_subtype: booted.raw_subtype(),
        subtype: slot.raw_subtype(),
        offset: slot.offset(),
        len: slot.len(),
    });
}

/// Logs which slot `otadata` selects, with each sector's `ota_seq`. Reads
/// only.
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
    let selected = esp_bootloader_esp_idf::ota::Ota::new(p.as_flash_region(flash), ota_slots)
        .and_then(|mut ota| ota.current_app_partition());
    match selected {
        // `current_app_partition` reports blank otadata as Factory.
        Ok(partitions::AppPartitionSubType::Factory) => {
            info!("storage: otadata blank, selects no slot (the bootloader boots ota_0)")
        }
        Ok(s) => info!(
            "storage: otadata selects {:?} (ota_seq {=u32:#x} / {=u32:#x})",
            defmt::Debug2Format(&s),
            seq(&entries[0]),
            seq(&entries[1])
        ),
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
