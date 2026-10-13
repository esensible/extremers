//! The bundle store (the plan's A5): the installed bundles, each in an
//! extent of whole sectors of a flash region read in place (on the C6 the
//! data slot's bundles region, `config::DATA_BUNDLES_LEN`), so a
//! [`Package`] is used where it lies.
//!
//! An extent is a 64-byte record (`bundle::store::Record`: state, seq, id,
//! length, sectors, the archive's SHA-256, a CRC) and the archive at +64.
//! A record's state only ever clears bits: *writing* (an upload in
//! progress, or one cut off), *committed* (installed), *deleted*. The
//! region is a sequence of items: an extent where a sector starts with a
//! valid record ([`walk`]), else a single free sector.
//!
//! - **Boot** ([`BundleStore::open`]): the committed extents are the
//!   installed bundles; of two with one id the higher `seq` wins and the
//!   other is marked deleted. Each is validated (`Package::load`); one that
//!   doesn't validate is logged and skipped but left as it is, so a newer
//!   firmware with a bug destroys nothing. A *writing* extent is free (an
//!   upload cut off by a reboot).
//! - **Upload** ([`Bundles::begin`], [`write`](Bundles::write),
//!   [`commit`](Bundles::commit)): the first run of sectors that are not
//!   live (committed, or the upload in progress) and hold it; a sector is
//!   erased just before it is first written, if it isn't blank. The record
//!   goes first (writing, `seq` one past the highest), the archive in
//!   order, hashed as it goes. A commit checks the hash, validates the
//!   bundle in place, lets the registry refuse it, then marks it committed
//!   and the older bundle with its id deleted: a replacement is written
//!   beside the bundle it replaces and swapped at the commit.
//! - **Deleting** marks the record. Its sectors are erased only when a
//!   later upload needs them: deleted extents are never erased on their
//!   own.
//!
//! A bundle deleted (or replaced) since boot may still be read: an HTTP
//! response or a BLE transfer of one of its files took it from the
//! registry before it went. Its sectors are *stale*; an upload that needs
//! one waits for no reader to be left ([`Host::lease`]), and is told
//! [`Error::Leased`] until then.
//!
//! [`Host::lease`]: crate::host::Host::lease

use core::ptr;

use bundle::{
    Invalid, Limits,
    store::{RECORD_BYTES, SECTOR, STATE_OFFSET},
};
/// The record (extreme-bundle's) and its states.
pub use bundle::store::{COMMITTED, DELETED, Record, WRITING};
use extreme_protocol::{PackageId, REGISTRY_MAX};
use sha2::{Digest, Sha256};

use crate::{
    fmt::{Dbg, Disp, Hex},
    host::InstallError,
    package::Package,
    track::{SECTOR_LEN, TrackFlash},
};

/// The most sectors a store's region has: the C6's bundles region.
pub const MAX_SECTORS: usize = 128;
/// The most bundles handed to the registry at boot (Choose is the other).
pub const MAX_APPS: usize = REGISTRY_MAX - 1;
/// The bundles [`BundleStore::open`] hands the registry.
pub type Apps = heapless::Vec<Package, MAX_APPS>;

/// A record state's name, for logs.
pub fn state_name(state: u32) -> &'static str {
    match state {
        COMMITTED => "committed",
        DELETED => "deleted",
        WRITING => "writing",
        _ => "?",
    }
}

/// Where the archive of the extent at sector `first` starts, from the
/// region's start.
pub fn archive_offset(first: u16) -> usize {
    first as usize * SECTOR + RECORD_BYTES
}

const _: () = assert!(SECTOR == SECTOR_LEN);
const _: () = assert!(crate::config::DATA_BUNDLES_LEN as usize == MAX_SECTORS * SECTOR);

/// Why a store operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// An upload is in progress.
    Busy,
    /// No run of free sectors holds it.
    NoSpace,
    /// The sectors it needs held a bundle deleted since boot, and something
    /// may still be reading it (a [`Host::lease`](crate::host::Host::lease)
    /// is held); asked again once none is, it goes ahead.
    Leased,
    /// No upload is in progress.
    NoUpload,
    /// Not the next bytes of the upload: `expected` is the offset written
    /// up to.
    Sequence { expected: u32 },
    /// A length that doesn't fit: zero, past the declared length, a chunk
    /// other than the last that isn't a multiple of 4 bytes, or a commit
    /// before every byte is written.
    Length,
    /// The bytes' SHA-256 isn't the one declared.
    Hash,
    /// The bundle doesn't validate.
    Invalid(Invalid),
    /// Its manifest's id isn't the one declared.
    Id,
    /// The registry doesn't take it.
    Install(InstallError),
    /// No bundle with that id.
    NotFound,
    /// A flash write or erase failed (logged).
    Flash,
}

/// One item of the region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// An extent: its first sector, and its record.
    Extent { first: u16, record: Record },
    /// A sector with no record at its start: erased, or left over from an
    /// extent whose record is gone.
    Free { sector: u16 },
}

impl Item {
    pub fn first(&self) -> u16 {
        match *self {
            Item::Extent { first, .. } => first,
            Item::Free { sector } => sector,
        }
    }

    pub fn sectors(&self) -> u16 {
        match self {
            Item::Extent { record, .. } => record.sectors,
            Item::Free { .. } => 1,
        }
    }

    /// The sector after it.
    pub fn end(&self) -> u16 {
        self.first() + self.sectors()
    }

    /// A committed extent's record.
    pub fn committed(&self) -> Option<(u16, Record)> {
        match *self {
            Item::Extent { first, record } if record.state == COMMITTED => Some((first, record)),
            _ => None,
        }
    }
}

/// The region read in place.
#[derive(Clone, Copy)]
struct Region {
    bytes: &'static [u8],
    sectors: u16,
}

impl Region {
    fn new(bytes: &'static [u8]) -> Region {
        let sectors = bytes.len() / SECTOR;
        assert!(
            bytes.len() % SECTOR == 0 && (1..=MAX_SECTORS).contains(&sectors),
            "a bundles region is 1 to {MAX_SECTORS} whole sectors"
        );
        Region { bytes, sectors: sectors as u16 }
    }

    /// The record at the start of `sector`, if there is one.
    fn record(&self, sector: u16) -> Option<Record> {
        let at = sector as usize * SECTOR;
        assert!(at < self.bytes.len());
        // SAFETY: in bounds (above); the region is flash read in place (or
        // the tests' model of it), changed by writes behind the reference,
        // hence volatile. `[u8; 64]` has alignment 1.
        let b = unsafe { ptr::read_volatile(self.bytes.as_ptr().add(at).cast::<[u8; RECORD_BYTES]>()) };
        Record::parse(&b).ok()
    }

    /// Whether every byte of `sector` is 0xFF.
    fn blank(&self, sector: u16) -> bool {
        let from = sector as usize * SECTOR;
        // SAFETY: as in `record`; word reads, the region being sector-aligned.
        (from..from + SECTOR).step_by(4).all(|o| unsafe { ptr::read_volatile(self.bytes.as_ptr().add(o).cast::<[u8; 4]>()) } == [0xFF; 4])
    }

    /// The archive of the extent at `first`, `len` bytes.
    fn archive(&self, first: u16, len: u32) -> &'static [u8] {
        let at = archive_offset(first);
        &self.bytes[at..at + len as usize]
    }
}

/// The region's items, in order ([`walk`]).
pub struct Walk {
    region: Region,
    at: u16,
}

impl Iterator for Walk {
    type Item = Item;

    fn next(&mut self) -> Option<Item> {
        let at = self.at;
        if at >= self.region.sectors {
            return None;
        }
        let item = match self.region.record(at) {
            Some(record) if at as u32 + record.sectors as u32 <= self.region.sectors as u32 => Item::Extent { first: at, record },
            _ => Item::Free { sector: at },
        };
        self.at = item.end();
        Some(item)
    }
}

/// The items of a bundles region (`bytes`, read in place), as the store
/// finds them; reads only.
pub fn walk(bytes: &'static [u8]) -> Walk {
    Walk { region: Region::new(bytes), at: 0 }
}

/// Free space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Space {
    /// Every sector not live, in bytes.
    pub free_bytes: u32,
    /// The largest archive a new upload has room for (the longest run of
    /// sectors not live, less its record); 0 if none.
    pub largest_free: u32,
}

/// What [`Bundles::uploading`] tells of the upload in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Uploading {
    pub id: PackageId,
    pub len: u32,
    pub sha256: [u8; 32],
    /// Bytes written so far.
    pub written: u32,
}

/// The store's operations, as the host uses them (object safe: the host
/// holds a `&'static mut dyn Bundles`, whatever flash is under it).
pub trait Bundles {
    /// Starts an upload of a bundle `id` of `len` bytes whose SHA-256 is
    /// `sha256`: allocates its extent and writes its record. `readers_idle`:
    /// no [`Host::lease`](crate::host::Host::lease) is held, so sectors of
    /// bundles deleted since boot may be erased ([`Error::Leased`] if they
    /// are needed and it is `false`).
    fn begin(&mut self, id: PackageId, len: u32, sha256: &[u8; 32], readers_idle: bool) -> Result<(), Error>;
    /// Writes the upload's next bytes, from `offset` in the archive (which
    /// must be where it is up to); every chunk but the last a multiple of 4
    /// bytes. The bytes written so far.
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<u32, Error>;
    /// Ends the upload, once every byte is written: checks the hash,
    /// validates the bundle in place (`limits`, `scratch` as for
    /// [`Package::load`]), asks `accept` (the registry), and only then
    /// marks it committed and an older bundle with its id deleted. Every
    /// outcome but [`Error::NoUpload`] and [`Error::Length`] ends the
    /// upload; a refused one is marked deleted.
    fn commit(
        &mut self,
        limits: &Limits,
        scratch: &mut [u16],
        accept: &mut dyn FnMut(&Package) -> Result<(), InstallError>,
    ) -> Result<Package, Error>;
    /// Drops the upload in progress (marked deleted); whether there was one.
    fn abort(&mut self) -> bool;
    /// Marks the bundle `id` deleted.
    fn delete(&mut self, id: PackageId) -> Result<(), Error>;
    fn space(&self) -> Space;
    fn uploading(&self) -> Option<Uploading>;
    /// The region's items, as [`walk`].
    fn entries(&self) -> Walk;
}

/// The upload in progress.
struct Upload {
    first: u16,
    record: Record,
    written: u32,
    /// Sectors of the extent ready to be written (erased or blank), from
    /// its first.
    ready: u16,
    hasher: Sha256,
}

/// The bundles in a flash region ([`TrackFlash`]: NOR, read in place).
pub struct BundleStore<F: TrackFlash> {
    flash: F,
    region: Region,
    /// The `seq` the next upload gets.
    next_seq: u32,
    upload: Option<Upload>,
    /// Sectors of bundles deleted since boot, one bit each (see the module
    /// documentation).
    stale: u128,
}

/// The bits of sectors `first..first + sectors`.
fn mask(first: u16, sectors: u16) -> u128 {
    let bits = if sectors as u32 >= 128 { !0 } else { (1u128 << sectors) - 1 };
    bits << first
}

impl<F: TrackFlash> BundleStore<F> {
    /// Opens the store in `flash` at boot: resolves duplicate ids (the
    /// lower `seq` marked deleted) and validates every committed bundle,
    /// oldest first, with `limits` and `scratch` (as [`Package::load`]).
    /// The bundles that validate, at most [`MAX_APPS`], go to the
    /// registry; the rest are logged and left in flash as they are.
    pub fn open(flash: F, limits: &Limits, scratch: &mut [u16]) -> (Self, Apps) {
        let region = Region::new(flash.bytes());
        let mut store = BundleStore { flash, region, next_seq: 1, upload: None, stale: 0 };
        let walk = || Walk { region, at: 0 };

        let max_seq = walk()
            .filter_map(|i| match i {
                Item::Extent { record, .. } => Some(record.seq),
                Item::Free { .. } => None,
            })
            .max();
        store.next_seq = max_seq.map_or(1, |s| s.saturating_add(1));

        // of two with one id, the higher seq (then the later sector) wins
        for (first, r) in walk().filter_map(|i| i.committed()) {
            let newer = walk().filter_map(|i| i.committed()).find(|&(f, o)| o.id == r.id && (o.seq, f) > (r.seq, first));
            if let Some((f, o)) = newer {
                warn!(
                    "store: bundle {} in sectors {}+{} (seq {}) is replaced by seq {} in sector {}: marked deleted",
                    Disp(&Hex(&r.id.to_be_bytes())),
                    first,
                    r.sectors,
                    r.seq,
                    o.seq,
                    f
                );
                // a failure is logged; the next boot resolves it again
                let _ = store.set_state(first, DELETED);
            }
        }

        let mut apps = heapless::Vec::new();
        let mut after: Option<u32> = None;
        while let Some((first, r)) = walk()
            .filter_map(|i| i.committed())
            .filter(|(_, r)| after.is_none_or(|a| r.seq > a))
            .min_by_key(|(_, r)| r.seq)
        {
            after = Some(r.seq);
            let id = Disp(&Hex(&r.id.to_be_bytes()));
            match Package::load(region.archive(first, r.len), limits, scratch) {
                Ok(p) if p.id != r.id => error!(
                    "store: bundle {} in sector {}: its manifest's id is {}; skipped",
                    id,
                    first,
                    Disp(&Hex(&p.id.to_be_bytes()))
                ),
                Ok(p) => {
                    if apps.push(p).is_err() {
                        error!("store: bundle {} ({}) in sector {}: already {} bundles; skipped", id, p.name, first, MAX_APPS);
                    }
                }
                Err(e) => error!("store: bundle {} in sector {} does not validate: {:?}; skipped (left in flash)", id, first, Dbg(&e)),
            }
        }
        (store, apps)
    }

    /// Writes `state` into the record at `first`.
    fn set_state(&mut self, first: u16, state: u32) -> Result<(), Error> {
        let at = first as u32 * SECTOR as u32 + STATE_OFFSET as u32;
        self.flash.write(at, &state.to_le_bytes()).map_err(|e| {
            error!("store: writing the state of sector {} failed: {:?}", first, Dbg(&e));
            Error::Flash
        })
    }

    fn erase(&mut self, sector: u16) -> Result<(), Error> {
        self.flash.erase_sector(sector as u32).map_err(|e| {
            error!("store: erasing sector {} failed: {:?}", sector, Dbg(&e));
            Error::Flash
        })
    }

    fn walk(&self) -> Walk {
        Walk { region: self.region, at: 0 }
    }

    /// Whether `item` is live: committed, or the upload in progress.
    fn live(&self, item: &Item) -> bool {
        match item {
            Item::Extent { first, record } => record.state == COMMITTED || self.upload.as_ref().is_some_and(|u| u.first == *first),
            Item::Free { .. } => false,
        }
    }

    /// The first run of `need` sectors not live: it starts at an item's
    /// first sector, so the walk never skips into it from an extent before
    /// it.
    fn first_fit(&self, need: u16) -> Option<u16> {
        let mut start = None;
        for item in self.walk() {
            if self.live(&item) {
                start = None;
                continue;
            }
            let s = *start.get_or_insert(item.first());
            if item.end() - s >= need {
                return Some(s);
            }
        }
        None
    }

    /// Marks every committed extent of `id` but the one at `keep` deleted
    /// (stale: read, maybe, until the readers are gone); how many.
    fn delete_except(&mut self, id: PackageId, keep: Option<u16>) -> Result<usize, Error> {
        let mut n = 0;
        let mut failed = false;
        while let Some((first, r)) = self.walk().filter_map(|i| i.committed()).find(|&(f, r)| r.id == id && Some(f) != keep) {
            if self.set_state(first, DELETED).is_err() {
                failed = true;
                break;
            }
            self.stale |= mask(first, r.sectors);
            n += 1;
        }
        if failed { Err(Error::Flash) } else { Ok(n) }
    }
}

impl<F: TrackFlash> Bundles for BundleStore<F> {
    fn begin(&mut self, id: PackageId, len: u32, sha256: &[u8; 32], readers_idle: bool) -> Result<(), Error> {
        if self.upload.is_some() {
            return Err(Error::Busy);
        }
        if len == 0 {
            return Err(Error::Length);
        }
        if len as usize > self.region.bytes.len() {
            return Err(Error::NoSpace);
        }
        let sectors = Record::sectors_for(len);
        let first = self.first_fit(sectors).ok_or(Error::NoSpace)?;
        if self.stale & mask(first, sectors) != 0 {
            if !readers_idle {
                return Err(Error::Leased);
            }
            // nothing reads a deleted bundle now, and nothing can start to
            // (it is out of the registry): no sector is stale any more
            self.stale = 0;
        }
        if !self.region.blank(first) {
            self.erase(first)?;
        }
        let record = Record { state: WRITING, seq: self.next_seq, id, len, sectors, sha256: *sha256 };
        self.next_seq = self.next_seq.saturating_add(1);
        let at = first as u32 * SECTOR as u32;
        if let Err(e) = self.flash.write(at, &record.to_bytes()) {
            error!("store: writing the record at sector {} failed: {:?}", first, Dbg(&e));
            return Err(Error::Flash);
        }
        info!(
            "store: upload of {} begun: {} B in sectors {}+{}, seq {}",
            Disp(&Hex(&id.to_be_bytes())),
            len,
            first,
            sectors,
            record.seq
        );
        self.upload = Some(Upload { first, record, written: 0, ready: 1, hasher: Sha256::new() });
        Ok(())
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<u32, Error> {
        let region = self.region;
        let u = self.upload.as_mut().ok_or(Error::NoUpload)?;
        if offset != u.written {
            return Err(Error::Sequence { expected: u.written });
        }
        let end = offset.checked_add(bytes.len() as u32).ok_or(Error::Length)?;
        if end > u.record.len || (bytes.len() % 4 != 0 && end != u.record.len) {
            return Err(Error::Length);
        }
        if bytes.is_empty() {
            return Ok(u.written);
        }
        let at = u.first as u32 * SECTOR as u32 + RECORD_BYTES as u32 + offset;
        // the last chunk is padded to a word with 0xFF, which programs
        // nothing; the extent's last sector has room (sectors are words)
        let padded_end = at + (bytes.len() as u32).next_multiple_of(4);
        let last = ((padded_end - 1) / SECTOR as u32) as u16;
        let first = u.first;
        while first + u.ready <= last {
            let s = first + u.ready;
            if !region.blank(s) {
                self.flash.erase_sector(s as u32).map_err(|e| {
                    error!("store: erasing sector {} failed: {:?}", s, Dbg(&e));
                    Error::Flash
                })?;
            }
            u.ready += 1;
        }
        let whole = bytes.len() / 4 * 4;
        let written = self.flash.write(at, &bytes[..whole]).and_then(|()| {
            let tail = &bytes[whole..];
            if tail.is_empty() {
                return Ok(());
            }
            let mut word = [0xFF; 4];
            word[..tail.len()].copy_from_slice(tail);
            self.flash.write(at + whole as u32, &word)
        });
        if let Err(e) = written {
            error!("store: writing {} B at {} failed: {:?}", bytes.len(), at, Dbg(&e));
            return Err(Error::Flash);
        }
        u.hasher.update(bytes);
        u.written = end;
        Ok(end)
    }

    fn commit(
        &mut self,
        limits: &Limits,
        scratch: &mut [u16],
        accept: &mut dyn FnMut(&Package) -> Result<(), InstallError>,
    ) -> Result<Package, Error> {
        let u = self.upload.as_ref().ok_or(Error::NoUpload)?;
        if u.written != u.record.len {
            return Err(Error::Length);
        }
        let Some(u) = self.upload.take() else {
            return Err(Error::NoUpload);
        };
        let refused = |store: &mut Self, e: Error| {
            warn!("store: upload of {} refused: {:?}; marked deleted", Disp(&Hex(&u.record.id.to_be_bytes())), Dbg(&e));
            let _ = store.set_state(u.first, DELETED);
            Err(e)
        };
        let sha256: [u8; 32] = u.hasher.finalize().into();
        if sha256 != u.record.sha256 {
            return refused(self, Error::Hash);
        }
        let package = match Package::load(self.region.archive(u.first, u.record.len), limits, scratch) {
            Ok(p) if p.id != u.record.id => return refused(self, Error::Id),
            Ok(p) => p,
            Err(e) => return refused(self, Error::Invalid(e)),
        };
        if let Err(e) = accept(&package) {
            return refused(self, Error::Install(e));
        }
        self.set_state(u.first, COMMITTED)?;
        // a failure is logged; the new one wins at the next boot regardless
        // (the higher seq)
        let replaced = self.delete_except(u.record.id, Some(u.first)).unwrap_or(0);
        info!(
            "store: {} v{} committed in sectors {}+{}, seq {}{}",
            package.name,
            package.version,
            u.first,
            u.record.sectors,
            u.record.seq,
            if replaced > 0 { ", replacing the one before" } else { "" }
        );
        Ok(package)
    }

    fn abort(&mut self) -> bool {
        let Some(u) = self.upload.take() else {
            return false;
        };
        info!("store: upload of {} aborted at {} of {} B", Disp(&Hex(&u.record.id.to_be_bytes())), u.written, u.record.len);
        let _ = self.set_state(u.first, DELETED);
        true
    }

    fn delete(&mut self, id: PackageId) -> Result<(), Error> {
        match self.delete_except(id, None)? {
            0 => Err(Error::NotFound),
            _ => Ok(()),
        }
    }

    fn space(&self) -> Space {
        let (mut free, mut run, mut longest) = (0u32, 0u32, 0u32);
        for item in self.walk() {
            if self.live(&item) {
                run = 0;
            } else {
                free += item.sectors() as u32;
                run += item.sectors() as u32;
                longest = longest.max(run);
            }
        }
        Space {
            free_bytes: free * SECTOR as u32,
            largest_free: (longest * SECTOR as u32).saturating_sub(RECORD_BYTES as u32),
        }
    }

    fn uploading(&self) -> Option<Uploading> {
        self.upload.as_ref().map(|u| Uploading { id: u.record.id, len: u.record.len, sha256: u.record.sha256, written: u.written })
    }

    fn entries(&self) -> Walk {
        self.walk()
    }
}

#[cfg(test)]
mod tests;
