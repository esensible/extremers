//! The engine's `gps` list: the fixes since boot, oldest first
//! (engine/PROFILE.md §3).
//!
//! A [`Track`] holds the fixes; [`GpsList`] serves one to the VM as the
//! `gps` input (`vm::ListInput`), each fix's fields at the leaves the
//! engine's layout names. A fix is `{t, lat, lon, alt?}`; `t` is the device
//! clock, strictly increasing.
//!
//! Two tracks:
//!
//! - [`FlashTrack`], on the C6: a ring of plain 32-byte records in a flash
//!   region read in place (execute-in-place), kept across reboots. The
//!   engine sees the fixes since boot (a new *run* per boot); older ones
//!   stay in the ring for a later download.
//! - [`RamTrack`], for hosts without one (`tgt-std`, the tests): the newest
//!   `N` fixes, gone at a reboot.
//!
//! [`TrackLog`] is what the host appends through, whichever it has.

use core::ptr;

use bundle::Layout;
use vm::{ListInput, Value};

/// One fix in a track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackFix {
    /// Device clock, epoch ms; strictly increasing along the track.
    pub t: i64,
    pub lat: f64,
    pub lon: f64,
    /// Altitude, if the fix had one.
    pub alt: Option<f64>,
}

/// Fixes, oldest first.
pub trait Track {
    fn len(&self) -> u32;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Fix `i` (`None` past the end).
    fn get(&self, i: u32) -> Option<TrackFix>;
}

/// A track the host appends each fix to.
pub trait TrackLog: Track {
    /// Appends a fix taken at `now`; its `t`: `now`, or one more than the
    /// last fix's if the clock has not moved past it.
    fn append(&mut self, now: i64, lat: f64, lon: f64) -> i64;
}

/// A fix's leaves within an item of `gps`, from the engine's layout
/// (`fix_*`, item-relative).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixLeaves {
    pub t: u16,
    pub lat: u16,
    pub lon: u16,
    pub alt: u16,
}

impl FixLeaves {
    pub fn of(layout: &Layout) -> FixLeaves {
        FixLeaves {
            t: layout.fix_t,
            lat: layout.fix_lat,
            lon: layout.fix_lon,
            alt: layout.fix_alt,
        }
    }
}

/// `track` as an engine's `gps` input.
pub struct GpsList<'t> {
    pub track: &'t dyn Track,
    pub leaves: FixLeaves,
}

impl<'a> ListInput<'a> for GpsList<'_> {
    fn len(&self) -> u32 {
        self.track.len()
    }

    fn read(&self, item: u32, leaf: u16) -> Value<'a> {
        let Some(f) = self.track.get(item) else {
            return Value::Nil;
        };
        let l = self.leaves;
        if leaf == l.t {
            Value::Int(f.t)
        } else if leaf == l.lat {
            Value::Float(f.lat)
        } else if leaf == l.lon {
            Value::Float(f.lon)
        } else if leaf == l.alt {
            f.alt.map_or(Value::Nil, Value::Float)
        } else {
            Value::Nil
        }
    }
}

/// `t` given the last fix's: `now`, or one more than the last if the clock
/// has not moved past it.
fn next_t(last: Option<i64>, now: i64) -> i64 {
    match last {
        Some(last) => now.max(last.saturating_add(1)),
        None => now,
    }
}

// ---- in flash ----

/// A record: `t` i64, `lat` f64, `lon` f64, `alt` f64 (NaN: none), little
/// endian, nothing else (PROFILE §3).
pub const RECORD_LEN: usize = 32;
/// The flash's erase unit.
pub const SECTOR_LEN: usize = 4096;
/// Records in a sector.
pub const SECTOR_RECORDS: u32 = (SECTOR_LEN / RECORD_LEN) as u32;

/// The flash region a [`FlashTrack`] lives in: NOR flash (a write only
/// clears bits; an erase sets a whole sector to 0xFF), read in place.
/// Offsets are from the region's start.
pub trait TrackFlash {
    type Error: core::fmt::Debug;
    /// Erases sector `index` (bytes `index * SECTOR_LEN..`) to 0xFF.
    fn erase_sector(&mut self, index: u32) -> Result<(), Self::Error>;
    /// Writes `bytes` at `offset` (4-byte aligned, as are the lengths
    /// written here).
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error>;
    /// The region as read in place, whole sectors. Every write and erase
    /// that has returned is visible in it, so it is read with
    /// `ptr::read_volatile`: the bytes change under the reference.
    fn bytes(&self) -> &'static [u8];
}

/// What [`scan`] found in the region at boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanKind {
    /// Nothing written: the ring starts at record 0.
    Empty,
    /// A ring: written records, then the head (the first free record) and
    /// the free rest of its sector.
    Ring,
    /// A ring whose newest record is torn (the write was cut short: `t`
    /// written, `lat`/`lon` not, or `t` not after the record before). The
    /// torn record becomes the head; NOR flash can't take a record over
    /// it, so its sector is rewritten without it (the records before it
    /// kept, [`Scan::keep`]).
    Torn,
    /// A ring with no free record: cut off between filling a sector and
    /// erasing the next. The head is the `t` discontinuity (where `t`
    /// falls: the oldest record), and that sector is erased.
    Full,
    /// No ring: no free record and no single `t` discontinuity, or no
    /// consistent head (the slot held something else, such as an old
    /// firmware image). The ring starts at record 0, whose sector is
    /// erased; the other sectors are erased as the head reaches them.
    Garbage,
}

/// Where the ring is, as [`scan`] found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scan {
    pub kind: ScanKind,
    /// The record the next fix goes to.
    pub head: u32,
    /// The ring's oldest record still held.
    pub oldest: u32,
    /// Records held, `oldest` up to `head`.
    pub held: u32,
    /// `Some(n)`: the head's sector is not free from the head on, and is to
    /// be erased and its first `n` records written back (`n` = the head's
    /// place in the sector).
    pub keep: Option<u32>,
}

/// A record's fields; `t` all-0xFF bytes is a free record.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Record {
    t_free: bool,
    t: i64,
    lat: f64,
    lon: f64,
    alt: f64,
}

impl Record {
    fn decode(b: &[u8; RECORD_LEN]) -> Record {
        let f = |i: usize| f64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        Record {
            t_free: b[..8] == [0xFF; 8],
            t: i64::from_le_bytes(b[..8].try_into().unwrap()),
            lat: f(8),
            lon: f(16),
            alt: f(24),
        }
    }

    fn encode(t: i64, lat: f64, lon: f64, alt: f64) -> [u8; RECORD_LEN] {
        let mut b = [0u8; RECORD_LEN];
        b[..8].copy_from_slice(&t.to_le_bytes());
        b[8..16].copy_from_slice(&lat.to_le_bytes());
        b[16..24].copy_from_slice(&lon.to_le_bytes());
        b[24..].copy_from_slice(&alt.to_le_bytes());
        b
    }

    /// Whether it can be a fix this firmware wrote: a `t` (never negative:
    /// the device clock starts at 0), a latitude and a longitude in range
    /// (NaN is not), an altitude or NaN. An all-0xFF `lat` is NaN, so a
    /// record whose write stopped after `t` is not.
    fn plausible(&self) -> bool {
        !self.t_free
            && self.t >= 0
            && self.lat.abs() <= 90.0
            && self.lon.abs() <= 180.0
            && !self.alt.is_infinite()
    }

    fn fix(&self) -> TrackFix {
        TrackFix {
            t: self.t,
            lat: self.lat,
            lon: self.lon,
            alt: (!self.alt.is_nan()).then_some(self.alt),
        }
    }
}

/// Reads the region in place.
#[derive(Clone, Copy)]
struct Region {
    bytes: &'static [u8],
    records: u32,
}

impl Region {
    fn new(bytes: &'static [u8]) -> Region {
        assert!(
            bytes.len() % SECTOR_LEN == 0 && bytes.len() >= 2 * SECTOR_LEN,
            "a track region is two or more whole sectors"
        );
        Region { bytes, records: (bytes.len() / RECORD_LEN) as u32 }
    }

    fn sectors(&self) -> u32 {
        self.records / SECTOR_RECORDS
    }

    fn raw(&self, i: u32) -> [u8; RECORD_LEN] {
        assert!(i < self.records);
        // SAFETY: in bounds (above); the region is flash read in place (or
        // the tests' model of it), changed by writes behind the reference,
        // hence volatile. `[u8; 32]` has alignment 1.
        unsafe { ptr::read_volatile(self.bytes.as_ptr().add(i as usize * RECORD_LEN).cast::<[u8; RECORD_LEN]>()) }
    }

    fn record(&self, i: u32) -> Record {
        Record::decode(&self.raw(i))
    }

    fn free(&self, i: u32) -> bool {
        self.record(i).t_free
    }

    /// Record `i` steps before `from`, around the ring.
    fn back(&self, from: u32, i: u32) -> u32 {
        (from + self.records - i % self.records) % self.records
    }

    /// Whether every byte from record `i` to the end of its sector is 0xFF.
    fn blank_to_sector_end(&self, i: u32) -> bool {
        let end = (i / SECTOR_RECORDS + 1) * SECTOR_RECORDS;
        let (from, to) = (i as usize * RECORD_LEN, end as usize * RECORD_LEN);
        // SAFETY: as in `raw`; word reads, the region being sector-aligned.
        (from..to).step_by(4).all(|o| unsafe { ptr::read_volatile(self.bytes.as_ptr().add(o).cast::<[u8; 4]>()) } == [0xFF; 4])
    }

    /// The first free record in sector `s`, whose first record is written
    /// and last free: records are written in order, so the written ones
    /// are a prefix (binary search).
    fn first_free_in(&self, s: u32) -> u32 {
        let (mut lo, mut hi) = (s * SECTOR_RECORDS + 1, (s + 1) * SECTOR_RECORDS - 1);
        // invariant: lo - 1 written, hi free
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.free(mid) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        hi
    }

    /// Whether records `from..to` (within one sector) are plausible with
    /// `t` strictly increasing.
    fn ordered(&self, from: u32, to: u32) -> bool {
        let mut last = None;
        for i in from..to {
            let r = self.record(i);
            if !r.plausible() || last.is_some_and(|l| r.t <= l) {
                return false;
            }
            last = Some(r.t);
        }
        true
    }

    /// Whether `h` is the head of a ring this firmware wrote: free from `h`
    /// to its sector's end, and the records before it in their sector
    /// written in order (the newest, `h - 1`, may be torn: see `scan`).
    fn head_candidate(&self, h: u32) -> bool {
        let newest = self.back(h, 1);
        let start = newest / SECTOR_RECORDS * SECTOR_RECORDS;
        self.blank_to_sector_end(h) && !self.free(newest) && self.ordered(start, newest)
    }
}

/// GPS fixes in a flash ring, read in place.
///
/// Records are written one after another; a record is free iff its `t`
/// bytes are all 0xFF. The *head* is the next free record. When a write
/// fills a sector the next sector is erased at once (if it isn't blank),
/// so the ring always has a free head with the rest of its sector free,
/// and the oldest records go a sector at a time: appending never stops.
///
/// At boot ([`scan`]) the head is found from each sector's first
/// and last records (a binary search within the head's sector) and the
/// oldest by walking back over sectors while `t` falls. The engine sees
/// only what is appended after boot ([`len`](Track::len)).
pub struct FlashTrack<F: TrackFlash> {
    flash: F,
    region: Region,
    /// The record the next fix goes to.
    head: u32,
    /// Records held, ending at `head`.
    held: u32,
    /// Where this boot's run began (the head at boot).
    run_start: u32,
    /// Fixes appended since boot.
    run: u32,
    /// The newest record's `t`.
    last_t: Option<i64>,
    /// The most fixes `gps` holds ([`config::GPS_MAX_ITEMS`](crate::config::GPS_MAX_ITEMS)).
    max_items: u32,
    scan: Scan,
}

impl Scan {
    /// The fix `back` records before the head (0: the newest) in `bytes`,
    /// the region scanned; `None` past what is held.
    pub fn newest(&self, bytes: &'static [u8], back: u32) -> Option<TrackFix> {
        let r = Region::new(bytes);
        (back < self.held).then(|| r.record(r.back(self.head, back + 1)).fix())
    }
}

/// Finds the ring in `bytes` (a [`FlashTrack`]'s region, read in place):
/// what [`FlashTrack::open`] does at boot, without changing anything.
pub fn scan(bytes: &'static [u8]) -> Scan {
    let r = Region::new(bytes);
    let sectors = r.sectors();
    let sector_start = |s: u32| s * SECTOR_RECORDS;

    // the head: in the one sector written then free (a binary search
    // finds where), or at the start of the free sector after a written
    // one
    let mut head = None;
    let mut any_written = false;
    for s in 0..sectors {
        let first = sector_start(s);
        let h = if !r.free(first) {
            any_written = true;
            if !r.free(first + SECTOR_RECORDS - 1) {
                continue;
            }
            r.first_free_in(s)
        } else if !r.free(sector_start((s + sectors - 1) % sectors)) {
            first
        } else {
            continue;
        };
        // More than one can't come from this firmware; the first is as
        // good a guess as any, and records past a wrong one are erased
        // as the head reaches them.
        if head.is_none() && r.head_candidate(h) {
            head = Some(h);
        }
    }

    let mut kind = ScanKind::Ring;
    let head = match head {
        Some(h) => h,
        None if !any_written => {
            return Scan { kind: ScanKind::Empty, head: 0, oldest: 0, held: 0, keep: (!r.blank_to_sector_end(0)).then_some(0) };
        }
        None => match full_ring_head(&r) {
            Some(h) => {
                kind = ScanKind::Full;
                h
            }
            None => return Scan { kind: ScanKind::Garbage, head: 0, oldest: 0, held: 0, keep: Some(0) },
        },
    };

    // a torn newest record becomes the head
    let newest = r.back(head, 1);
    let n = r.record(newest);
    let before = r.record(r.back(head, 2));
    let torn = kind == ScanKind::Ring && (!n.plausible() || (before.plausible() && n.t <= before.t));
    let head = if torn {
        kind = ScanKind::Torn;
        newest
    } else {
        head
    };
    let keep = (!r.blank_to_sector_end(head)).then_some(head % SECTOR_RECORDS);

    // the oldest: back over whole sectors while `t` falls
    let mut oldest = head / SECTOR_RECORDS * SECTOR_RECORDS;
    let mut next_t = if head == oldest { None } else { Some(r.record(oldest).t) };
    for _ in 1..sectors {
        let s = r.back(oldest, SECTOR_RECORDS);
        let (first, last) = (r.record(s), r.record(s + SECTOR_RECORDS - 1));
        let follows = next_t.is_none_or(|t| last.t < t);
        if !(first.plausible() && last.plausible() && first.t < last.t && follows) {
            break;
        }
        oldest = s;
        next_t = Some(first.t);
    }
    let held = (head + r.records - oldest) % r.records;
    Scan { kind, head, oldest: if held == 0 { head } else { oldest }, held, keep }
}

impl<F: TrackFlash> FlashTrack<F> {
    /// Opens the ring in `flash`: [`scan`]s it, makes the head's
    /// sector free from the head on (rewriting what comes before it, if
    /// the scan says to), and starts a run at the head. `max_items` bounds
    /// [`len`](Track::len).
    pub fn open(mut flash: F, max_items: u32) -> Result<Self, F::Error> {
        let region = Region::new(flash.bytes());
        let scan = scan(flash.bytes());
        if let Some(keep) = scan.keep {
            let s = scan.head / SECTOR_RECORDS;
            let mut saved = [0u8; SECTOR_LEN];
            let n = keep as usize * RECORD_LEN;
            for i in 0..keep {
                let at = i as usize * RECORD_LEN;
                saved[at..at + RECORD_LEN].copy_from_slice(&region.raw(s * SECTOR_RECORDS + i));
            }
            flash.erase_sector(s)?;
            if n > 0 {
                flash.write(s * SECTOR_LEN as u32, &saved[..n])?;
            }
        }
        let last_t = (scan.held > 0).then(|| region.record(region.back(scan.head, 1)).t);
        Ok(FlashTrack {
            flash,
            region,
            head: scan.head,
            held: scan.held,
            run_start: scan.head,
            run: 0,
            last_t,
            max_items,
            scan,
        })
    }

    /// What the boot scan found.
    pub fn boot_scan(&self) -> Scan {
        self.scan
    }

    /// The record the next fix goes to.
    pub fn head(&self) -> u32 {
        self.head
    }

    /// Where this boot's run began.
    pub fn run_start(&self) -> u32 {
        self.run_start
    }

    /// The oldest record held.
    pub fn oldest(&self) -> u32 {
        self.region.back(self.head, self.held)
    }

    /// Records held (this run's and earlier ones').
    pub fn held(&self) -> u32 {
        self.held
    }

    /// The region's size in records.
    pub fn capacity(&self) -> u32 {
        self.region.records
    }

    /// The fix `back` records before the head (0: the newest), this run's or
    /// earlier; `None` past what is held.
    pub fn newest(&self, back: u32) -> Option<TrackFix> {
        (back < self.held).then(|| self.region.record(self.region.back(self.head, back + 1)).fix())
    }

}

impl<F: TrackFlash> Track for FlashTrack<F> {
    fn len(&self) -> u32 {
        self.run.min(self.held).min(self.max_items)
    }

    fn get(&self, i: u32) -> Option<TrackFix> {
        let len = self.len();
        (i < len).then(|| self.region.record(self.region.back(self.head, len - i)).fix())
    }
}

impl<F: TrackFlash> TrackLog for FlashTrack<F> {
    /// One record write, `alt` absent (the GPS reports none); if that fills
    /// the sector, the next one is erased (unless blank), dropping the
    /// oldest records.
    ///
    /// A failed write (logged) stores nothing and leaves the head where it
    /// was; the `t` is returned all the same. A failed erase (logged) leaves
    /// the next sector as it was: writes into it may then not read back,
    /// and the next boot's scan finds the head there and erases it.
    fn append(&mut self, now: i64, lat: f64, lon: f64) -> i64 {
        let t = next_t(self.last_t, now);
        let record = Record::encode(t, lat, lon, f64::NAN);
        if let Err(e) = self.flash.write(self.head * RECORD_LEN as u32, &record) {
            error!("track: write of record {} failed: {:?}", self.head, crate::fmt::Dbg(&e));
            return t;
        }
        self.last_t = Some(t);
        self.head = (self.head + 1) % self.region.records;
        self.held = (self.held + 1).min(self.region.records);
        self.run += 1;
        if self.head % SECTOR_RECORDS == 0 && !self.region.blank_to_sector_end(self.head) {
            let s = self.head / SECTOR_RECORDS;
            if let Err(e) = self.flash.erase_sector(s) {
                error!("track: erase of sector {} failed: {:?}", s, crate::fmt::Dbg(&e));
            }
            self.held = self.held.min(self.region.records - SECTOR_RECORDS);
        }
        t
    }
}

/// The head of a ring with no free record (cut off between filling a sector
/// and erasing the next): the start of the one sector whose first `t` is
/// below the `t` before it, every sector holding plausible records in
/// order. `None` if that isn't what the region holds. (Records in order
/// round a ring fall exactly once.)
fn full_ring_head(r: &Region) -> Option<u32> {
    let sectors = r.sectors();
    let mut head = None;
    for s in 0..sectors {
        let first = r.record(s * SECTOR_RECORDS);
        let last = r.record(s * SECTOR_RECORDS + SECTOR_RECORDS - 1);
        if !(first.plausible() && last.plausible() && first.t < last.t) {
            return None;
        }
        let prev_last = r.record(r.back(s * SECTOR_RECORDS, 1));
        if first.t < prev_last.t {
            if head.is_some() {
                return None;
            }
            head = Some(s * SECTOR_RECORDS);
        }
    }
    head
}

// ---- in RAM ----

/// The newest `N` fixes, in RAM.
pub struct RamTrack<const N: usize> {
    fixes: [TrackFix; N],
    /// Where the next fix goes.
    head: usize,
    /// Fixes held, at most `N`.
    len: usize,
}

impl<const N: usize> RamTrack<N> {
    pub const fn new() -> Self {
        RamTrack {
            fixes: [TrackFix { t: 0, lat: 0.0, lon: 0.0, alt: None }; N],
            head: 0,
            len: 0,
        }
    }

    /// The newest fix.
    pub fn last(&self) -> Option<TrackFix> {
        (self.len > 0).then(|| self.fixes[(self.head + N - 1) % N])
    }
}

impl<const N: usize> Default for RamTrack<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Track for RamTrack<N> {
    fn len(&self) -> u32 {
        self.len as u32
    }

    fn get(&self, i: u32) -> Option<TrackFix> {
        let i = i as usize;
        (i < self.len).then(|| self.fixes[(self.head + N - self.len + i) % N])
    }
}

impl<const N: usize> TrackLog for RamTrack<N> {
    fn append(&mut self, now: i64, lat: f64, lon: f64) -> i64 {
        let t = next_t(self.last().map(|f| f.t), now);
        if N > 0 {
            self.fixes[self.head] = TrackFix { t, lat, lon, alt: None };
            self.head = (self.head + 1) % N;
            self.len = (self.len + 1).min(N);
        }
        t
    }
}

#[cfg(test)]
mod tests;
