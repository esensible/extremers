//! The data slot (storage.rs, `DataSlot`) read in place: execute-in-place
//! (XIP) through the flash MMU and the cache, like the firmware's own
//! constants, so bundles and tracks are used where they lie rather than
//! copied into RAM.
//!
//! At boot [`map`] maps the whole slot into free flash-MMU entries. On the
//! C6 the flash MMU has 256 entries of one 64 KiB page each; entry `e`
//! shows at `0x4200_0000 + e * 0x1_0000` (esp-metadata: `mmu.entry_num`,
//! `mmu.page_size`, the `DROM` range; esp-storage's `entry_id_to_vaddr`).
//! The bootloader maps the running image into the low entries; esp-storage's
//! `map_flash_page` takes the highest free one (the last entry, 255, is
//! left for the bootloader), so mapping the slot's pages last first gives
//! them ascending entries and one contiguous range. That is checked page by
//! page, not assumed.
//!
//! The cache doesn't see writes and erases (the ROM routines bypass it),
//! so [`XipFlash`], the one way the data slot is written, invalidates
//! exactly the range of every write and erase (ROM `Cache_Invalidate_Addr`)
//! before returning. esp-storage does that only for encrypted writes.
//!
//! Three [`XipFlash`] regions share the slot: tracks, bundles, and the
//! whole slot, which a firmware update's image is written over
//! (`common::firmware::FirmwareSink`). That one overlaps the other two, and
//! is safe all the same: nothing writes it until the host has dropped the
//! store and moved the track to RAM (`Host::firmware_begin`), and nothing
//! writes those again before the reboot that ends every update.
//!
//! esp-storage keeps its MMU API to itself; the repo's copy
//! (vendor/esp-storage, the root Cargo.toml's `[patch.crates-io]`) makes it
//! public.

use core::ops::Range;
use core::ptr;

use common::track::{SECTOR_LEN, TrackFlash};
use defmt::{info, warn};
use embassy_sync::once_lock::OnceLock;
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_storage::{FlashStorage, FlashStorageError, mmu};

use crate::storage::{DataSlot, PagedFlash};

/// The flash MMU's page.
const PAGE: u32 = 0x1_0000;
/// Where flash MMU entry 0 shows (the C6's `DROM` start).
const VADDR_BASE: usize = 0x4200_0000;
/// The most pages [`map`] maps: a 2 MiB slot.
const MAX_PAGES: usize = 32;

/// The data slot as mapped.
struct Mapped {
    /// Its flash offset.
    offset: u32,
    bytes: &'static [u8],
}

static MAPPED: OnceLock<Mapped> = OnceLock::new();

/// Maps `slot` for reads in place; logs where. `false` (logged) if it is
/// already mapped elsewhere, no run of free MMU entries holds it, or it is
/// larger than [`MAX_PAGES`]: then nothing is left mapped.
pub fn map(slot: &DataSlot) -> bool {
    let pages = (slot.len / PAGE) as usize;
    if slot.offset % PAGE != 0 || slot.len % PAGE != 0 || pages == 0 || pages > MAX_PAGES {
        warn!(
            "xip: data slot {=u32:#x}+{=u32:#x} is not 1..={=usize} whole pages; not mapped",
            slot.offset,
            slot.len,
            MAX_PAGES
        );
        return false;
    }
    let mut guards: [Option<mmu::FlashMmapGuard>; MAX_PAGES] = [const { None }; MAX_PAGES];
    let mut ok = true;
    // where the page after this one is mapped
    let mut next: Option<usize> = None;
    for p in (0..pages).rev() {
        let at = slot.offset + p as u32 * PAGE;
        match mmu::map_flash_page(at) {
            Ok(g) => {
                let vaddr = g.vaddr() as usize;
                guards[p] = Some(g);
                if next.is_some_and(|n| n != vaddr + PAGE as usize) {
                    warn!("xip: flash {=u32:#x} mapped at {=usize:#x}, not just below the next page", at, vaddr);
                    ok = false;
                    break;
                }
                next = Some(vaddr);
            }
            Err(e) => {
                warn!("xip: cannot map flash {=u32:#x}: {:?}", at, defmt::Debug2Format(&e));
                ok = false;
                break;
            }
        }
    }
    if !ok {
        for g in guards.into_iter().flatten() {
            mmu::unmap_flash_page(g);
        }
        return false;
    }
    let start = guards[0].as_ref().map_or(0, |g| g.vaddr() as usize);
    // the mapping stays for good: a guard only unmaps when handed back
    core::mem::forget(guards);
    // SAFETY: `pages` pages from `start` now map the slot, read-only, for
    // the rest of the program; the bytes change only by flash writes and
    // erases, which `XipFlash` follows with a cache invalidation, and are
    // read with `read_volatile` (common::track, the store).
    let bytes = unsafe { core::slice::from_raw_parts(start as *const u8, slot.len as usize) };
    let entry = (start - VADDR_BASE) / PAGE as usize;
    info!(
        "xip: data slot flash {=u32:#08x}..{=u32:#08x} mapped at {=usize:#010x}..{=usize:#010x}, MMU entries {=usize}..={=usize}",
        slot.offset,
        slot.offset + slot.len,
        start,
        start + slot.len as usize,
        entry,
        entry + pages - 1
    );
    MAPPED.init(Mapped { offset: slot.offset, bytes }).is_ok()
}

/// The whole data slot, read in place; `None` unless [`map`] succeeded.
pub fn slot_bytes() -> Option<&'static [u8]> {
    MAPPED.try_get().map(|m| m.bytes)
}

/// The bytes of `range` (flash offsets) within the mapped slot.
pub fn region_bytes(range: Range<u32>) -> Option<&'static [u8]> {
    let m = MAPPED.try_get()?;
    let start = range.start.checked_sub(m.offset)? as usize;
    m.bytes.get(start..start + range.len())
}

/// The bundles region, read in place.
pub fn bundles_bytes(slot: &DataSlot) -> Option<&'static [u8]> {
    region_bytes(slot.bundles())
}

/// The tracks region, read in place.
pub fn tracks_bytes(slot: &DataSlot) -> Option<&'static [u8]> {
    region_bytes(slot.tracks())
}

/// Drops the cache's lines for `len` bytes from `vaddr`, so the next read
/// there fetches from flash. In RAM, with interrupts masked: the ROM routine
/// may stop the cache meanwhile, and no code in flash may run then
/// (esp-storage does the same around its encrypted writes).
#[esp_hal::ram]
fn invalidate(vaddr: usize, len: usize) {
    unsafe extern "C" {
        // C6 ROM (esp-rom-sys ld/esp32c6/rom/esp32c6.rom.ld)
        fn Cache_Invalidate_Addr(addr: u32, size: u32);
    }
    // SAFETY: a ROM routine on a range that is mapped.
    critical_section::with(|_| unsafe { Cache_Invalidate_Addr(vaddr as u32, len as u32) });
}

/// A region of the data slot (bundles or tracks): written and erased
/// through esp-storage (page by page, [`PagedFlash`]), read in place. Every
/// region has its own `FlashStorage`: each ROM flash operation is its own
/// critical section, and the regions are disjoint, so they need no lock
/// between them.
pub struct XipFlash {
    flash: PagedFlash,
    /// The region's flash offset.
    offset: u32,
    /// The region, read in place.
    bytes: &'static [u8],
    /// Whether a write was read back through the cache yet (logged once).
    write_checked: bool,
}

impl XipFlash {
    /// `range` (flash offsets, whole sectors) of the mapped slot, written
    /// through `flash`; `None` if it is not within it.
    pub fn new(flash: FlashStorage<'static>, range: Range<u32>) -> Option<XipFlash> {
        if range.start % SECTOR_LEN as u32 != 0 || range.len() % SECTOR_LEN != 0 {
            return None;
        }
        // esp-storage bounds every operation by the size it detected for
        // this handle (absolute flash offsets, as `range` is)
        let capacity = ReadNorFlash::capacity(&flash);
        if (capacity as u64) < range.end as u64 {
            warn!(
                "xip: flash handle for {=u32:#x}..{=u32:#x} has a capacity of {=usize:#x}: it would refuse the region",
                range.start,
                range.end,
                capacity
            );
            return None;
        }
        Some(XipFlash {
            offset: range.start,
            bytes: region_bytes(range)?,
            flash: PagedFlash::new(flash),
            write_checked: false,
        })
    }

    /// Reads the first 256 bytes of the tracks and the bundles regions both
    /// through the cache and through esp-storage (the ROM read, around the
    /// cache), and logs whether they agree: `xip: self-check ok` or each
    /// mismatch. `false` on a mismatch or a failed read.
    pub fn self_check(&mut self, slot: &DataSlot) -> bool {
        #[repr(align(4))]
        struct Buf([u8; 256]);
        let mut ok = true;
        let mut not_blank = [0usize; 2];
        for (i, (name, range)) in [("tracks", slot.tracks()), ("bundles", slot.bundles())].into_iter().enumerate() {
            let mut read = Buf([0; 256]);
            if let Err(e) = ReadNorFlash::read(&mut self.flash, range.start, &mut read.0) {
                warn!(
                    "xip: self-check: {=str}: flash read at {=u32:#x} failed: {:?}",
                    name,
                    range.start,
                    defmt::Debug2Format(&e)
                );
                ok = false;
                continue;
            }
            let Some(xip) = region_bytes(range.start..range.start + 256) else {
                warn!("xip: self-check: {=str} not mapped", name);
                ok = false;
                continue;
            };
            // SAFETY: mapped (above); volatile: see `map`
            let xip: [u8; 256] = unsafe { ptr::read_volatile(xip.as_ptr().cast()) };
            not_blank[i] = read.0.iter().filter(|&&b| b != 0xFF).count();
            if let Some(at) = (0..256).find(|&j| xip[j] != read.0[j]) {
                warn!(
                    "xip: self-check MISMATCH: {=str} byte {=usize} (flash {=u32:#x}): XIP {=u8:#04x}, flash read {=u8:#04x}",
                    name,
                    at,
                    range.start + at as u32,
                    xip[at],
                    read.0[at]
                );
                warn!("xip: {=str} XIP   {=[u8]:02x}", name, &xip[..32]);
                warn!("xip: {=str} flash {=[u8]:02x}", name, &read.0[..32]);
                ok = false;
            }
        }
        if ok {
            info!(
                "xip: self-check ok (first 256 B of tracks and bundles agree; {=usize} and {=usize} bytes not 0xFF)",
                not_blank[0],
                not_blank[1]
            );
        }
        ok
    }

    /// The region, as flash offsets.
    pub fn range(&self) -> Range<u32> {
        self.offset..self.offset + self.bytes.len() as u32
    }

    fn vaddr(&self, offset: u32) -> usize {
        self.bytes.as_ptr() as usize + offset as usize
    }
}

impl TrackFlash for XipFlash {
    type Error = FlashStorageError;

    fn erase_sector(&mut self, index: u32) -> Result<(), FlashStorageError> {
        let offset = index * SECTOR_LEN as u32;
        assert!((offset as usize) < self.bytes.len());
        let at = self.offset + offset;
        let erased = NorFlash::erase(&mut self.flash, at, at + SECTOR_LEN as u32);
        invalidate(self.vaddr(offset), SECTOR_LEN);
        erased
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashStorageError> {
        assert!(offset as usize + bytes.len() <= self.bytes.len());
        let written = NorFlash::write(&mut self.flash, self.offset + offset, bytes);
        invalidate(self.vaddr(offset), bytes.len());
        if written.is_ok() && !self.write_checked {
            self.write_checked = true;
            let seen = &self.bytes[offset as usize..][..bytes.len()];
            // SAFETY: in the region (asserted); volatile: see `map`
            let same = (0..bytes.len()).all(|i| unsafe { ptr::read_volatile(seen.as_ptr().add(i)) } == bytes[i]);
            if same {
                info!(
                    "xip: first write since boot (flash {=u32:#x}, {=usize} B) reads back through XIP",
                    self.offset + offset,
                    bytes.len()
                );
            } else {
                warn!(
                    "xip: first write since boot (flash {=u32:#x}, {=usize} B) does NOT read back through XIP",
                    self.offset + offset,
                    bytes.len()
                );
            }
        }
        written
    }

    fn bytes(&self) -> &'static [u8] {
        self.bytes
    }
}
