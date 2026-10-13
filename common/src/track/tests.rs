use std::cell::Cell;

use super::*;

/// NOR flash in RAM: a write ANDs bits in, an erase sets a sector to 0xFF.
/// `Copy`, so a test keeps a handle to the bytes a track owns, and reopens
/// them as a reboot would.
#[derive(Clone, Copy)]
struct Nor {
    mem: &'static [Cell<u8>],
    erases: &'static Cell<u32>,
}

impl Nor {
    fn new(sectors: usize, fill: impl Fn(usize) -> u8) -> Nor {
        let mem: Vec<Cell<u8>> = (0..sectors * SECTOR_LEN).map(|i| Cell::new(fill(i))).collect();
        Nor { mem: Vec::leak(mem), erases: Box::leak(Box::new(Cell::new(0))) }
    }

    fn blank(sectors: usize) -> Nor {
        Nor::new(sectors, |_| 0xFF)
    }

    /// Programs `bytes` at `offset` as the flash would: bits only cleared.
    fn program(&self, offset: usize, bytes: &[u8]) {
        for (c, b) in self.mem[offset..offset + bytes.len()].iter().zip(bytes) {
            c.set(c.get() & b);
        }
    }

    fn records(&self) -> u32 {
        (self.mem.len() / RECORD_LEN) as u32
    }
}

impl TrackFlash for Nor {
    type Error = ();

    fn erase_sector(&mut self, index: u32) -> Result<(), ()> {
        let at = index as usize * SECTOR_LEN;
        self.mem[at..at + SECTOR_LEN].iter().for_each(|c| c.set(0xFF));
        self.erases.set(self.erases.get() + 1);
        Ok(())
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), ()> {
        assert!(offset % 4 == 0 && bytes.len() % 4 == 0);
        self.program(offset as usize, bytes);
        Ok(())
    }

    fn bytes(&self) -> &'static [u8] {
        // `Cell<u8>` is a `u8`. The track only reads through this with
        // `read_volatile`, as on the device, where the flash changes under
        // the slice the same way.
        unsafe { core::slice::from_raw_parts(self.mem.as_ptr().cast::<u8>(), self.mem.len()) }
    }
}

const MAX: u32 = crate::config::GPS_MAX_ITEMS;

fn open(nor: Nor) -> FlashTrack<Nor> {
    FlashTrack::open(nor, MAX).unwrap()
}

/// Appends `n` fixes at 1 Hz from `t0`; fix `k`'s latitude is `k` scaled
/// into range, so records can be told apart.
fn sail(track: &mut impl TrackLog, t0: i64, n: u32) {
    for k in 0..n {
        let t = track.append(t0 + k as i64 * 1000, lat(k), 151.0);
        assert_eq!(t, t0 + k as i64 * 1000);
    }
}

fn lat(k: u32) -> f64 {
    -33.0 + (k % 1000) as f64 / 1000.0
}

fn ts(track: &impl Track) -> Vec<i64> {
    (0..track.len()).map(|i| track.get(i).unwrap().t).collect()
}

#[test]
fn a_blank_region_is_an_empty_ring() {
    let nor = Nor::blank(4);
    let mut track = open(nor);
    let scan = track.boot_scan();
    assert_eq!(scan, Scan { kind: ScanKind::Empty, head: 0, oldest: 0, held: 0, keep: None });
    assert_eq!(nor.erases.get(), 0);
    assert!(track.is_empty() && track.get(0).is_none());

    sail(&mut track, 1_000_000, 3);
    assert_eq!(ts(&track), [1_000_000, 1_001_000, 1_002_000]);
    assert_eq!(track.get(2), Some(TrackFix { t: 1_002_000, lat: lat(2), lon: 151.0, alt: None }));
    assert_eq!((track.head(), track.held(), track.oldest()), (3, 3, 0));
}

#[test]
fn t_strictly_increases() {
    let mut track = open(Nor::blank(2));
    assert_eq!(track.append(100, 1.0, 2.0), 100);
    // the clock did not move, or went back: one ms after the last
    assert_eq!(track.append(100, 3.0, 4.0), 101);
    assert_eq!(track.append(50, 5.0, 6.0), 102);
    assert_eq!(track.append(200, 7.0, 8.0), 200);
    assert_eq!(ts(&track), [100, 101, 102, 200]);
}

#[test]
fn a_reboot_starts_a_new_run_and_keeps_the_old_one() {
    let nor = Nor::blank(4);
    let mut track = open(nor);
    sail(&mut track, 1_000_000, 10);

    let mut track = open(nor);
    let scan = track.boot_scan();
    assert_eq!(scan, Scan { kind: ScanKind::Ring, head: 10, oldest: 0, held: 10, keep: None });
    // the engine sees this boot's fixes only
    assert_eq!(track.len(), 0);
    assert_eq!(track.run_start(), 10);
    assert_eq!(track.newest(0).unwrap().t, 1_009_000);
    // t carries on from the last run's, even with the clock behind it
    assert_eq!(track.append(5, 0.0, 0.0), 1_009_001);
    assert_eq!(ts(&track), [1_009_001]);
    assert_eq!(track.held(), 11);
}

#[test]
fn the_head_is_found_at_every_place_in_a_sector() {
    for n in 0..=3 * SECTOR_RECORDS {
        let nor = Nor::blank(4);
        sail(&mut open(nor), 5_000, n);
        let track = open(nor);
        let scan = track.boot_scan();
        assert_eq!((scan.head, scan.held, scan.oldest, scan.keep), (n, n, 0, None), "{n} fixes");
        assert_eq!(scan.kind, if n == 0 { ScanKind::Empty } else { ScanKind::Ring }, "{n} fixes");
    }
}

#[test]
fn the_ring_wraps_and_slides() {
    let nor = Nor::blank(4);
    let records = nor.records();
    let mut track = open(nor);
    let n = 3 * records + 77;
    sail(&mut track, 0, n);
    // a sector is erased as the one before it fills: `held` never reaches
    // the region's size
    assert_eq!(track.head(), n % records);
    assert!(track.held() < records && track.held() > records - SECTOR_RECORDS);
    assert_eq!(track.len(), track.held());
    let got = ts(&track);
    assert_eq!(got.len() as u32, track.held());
    assert!(got.windows(2).all(|w| w[1] == w[0] + 1000));
    assert_eq!(*got.last().unwrap(), (n as i64 - 1) * 1000);

    let (head, held) = (track.head(), track.held());
    let track = open(nor);
    let scan = track.boot_scan();
    assert_eq!((scan.kind, scan.head, scan.held, scan.keep), (ScanKind::Ring, head, held, None));
    assert_eq!(scan.oldest, (head / SECTOR_RECORDS + 1) * SECTOR_RECORDS % records);
    assert_eq!(track.newest(held - 1).unwrap().t, *got.first().unwrap());
}

#[test]
fn a_reboot_at_a_sector_boundary() {
    let nor = Nor::blank(4);
    let records = nor.records();
    // wrapped, and the head just moved into a sector (erased at once)
    sail(&mut open(nor), 0, records + SECTOR_RECORDS);
    let track = open(nor);
    let scan = track.boot_scan();
    assert_eq!((scan.kind, scan.head, scan.keep), (ScanKind::Ring, SECTOR_RECORDS, None));
    assert_eq!((scan.oldest, scan.held), (2 * SECTOR_RECORDS, records - SECTOR_RECORDS));
}

#[test]
fn the_run_is_bounded_by_gps_max_items() {
    let nor = Nor::blank(4);
    let mut track = FlashTrack::open(nor, 50).unwrap();
    sail(&mut track, 0, 70);
    assert_eq!(track.len(), 50);
    assert_eq!(track.get(0).unwrap().t, 20_000);
    assert_eq!(track.get(49).unwrap().t, 69_000);
    assert!(track.get(50).is_none());
}

#[test]
fn the_profiles_bound_is_the_tracks_region() {
    assert_eq!(MAX, 20_480);
    assert_eq!(crate::config::DATA_TRACKS_LEN as usize / RECORD_LEN, 20_480);
}

#[test]
fn a_run_longer_than_the_ring_keeps_its_newest() {
    let nor = Nor::blank(2);
    let records = nor.records();
    let mut track = open(nor);
    sail(&mut track, 0, 3 * records);
    // the oldest of this run were erased: the run is what is held
    assert_eq!(track.len(), track.held());
    assert_eq!(track.get(track.len() - 1).unwrap().t, (3 * records as i64 - 1) * 1000);
}

#[test]
fn a_torn_record_becomes_the_head() {
    let nor = Nor::blank(4);
    sail(&mut open(nor), 0, 140);
    // the 141st write stopped after `t`: lat, lon and alt still 0xFF
    nor.program(140 * RECORD_LEN, &140_000i64.to_le_bytes());
    let erases = nor.erases.get();

    let mut track = open(nor);
    let scan = track.boot_scan();
    assert_eq!((scan.kind, scan.head, scan.held, scan.keep), (ScanKind::Torn, 140, 140, Some(12)));
    // its sector was erased and the 12 records before it written back
    assert_eq!(nor.erases.get(), erases + 1);
    assert_eq!(track.newest(0).unwrap().t, 139_000);
    assert_eq!(track.newest(11).unwrap().t, 128_000);
    assert_eq!(track.newest(139).unwrap().t, 0);
    assert_eq!(track.append(150_000, -33.5, 151.5), 150_000);
    assert_eq!(track.get(0), Some(TrackFix { t: 150_000, lat: -33.5, lon: 151.5, alt: None }));

    let scan = open(nor).boot_scan();
    assert_eq!((scan.kind, scan.head, scan.held), (ScanKind::Ring, 141, 141));
}

#[test]
fn a_torn_t_becomes_the_head() {
    let nor = Nor::blank(4);
    sail(&mut open(nor), 0, SECTOR_RECORDS);
    // the first record of sector 1, its `t` only partly programmed (a
    // negative `t`), at a sector start: nothing to keep
    nor.program(SECTOR_LEN, &[0x00, 0x10, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    let track = open(nor);
    let scan = track.boot_scan();
    assert_eq!((scan.kind, scan.head, scan.held, scan.keep), (ScanKind::Torn, SECTOR_RECORDS, SECTOR_RECORDS, Some(0)));
    assert!(track.region.blank_to_sector_end(SECTOR_RECORDS));
}

/// Bytes like an old firmware image: no 0xFF `t` anywhere.
fn garbage(i: usize) -> u8 {
    let x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29);
    (x as u8) & 0xFE
}

#[test]
fn garbage_starts_a_ring_at_record_0() {
    let nor = Nor::new(4, garbage);
    let mut track = open(nor);
    let scan = track.boot_scan();
    assert_eq!(scan, Scan { kind: ScanKind::Garbage, head: 0, oldest: 0, held: 0, keep: Some(0) });
    assert_eq!(nor.erases.get(), 1);
    assert!(track.region.blank_to_sector_end(0));
    assert!(!track.region.blank_to_sector_end(SECTOR_RECORDS));

    // into sector 1: erased as sector 0 fills
    sail(&mut track, 0, SECTOR_RECORDS + 5);
    assert_eq!(nor.erases.get(), 2);
    assert_eq!(ts(&track).len() as u32, SECTOR_RECORDS + 5);

    // a reboot finds this ring, not the garbage after it
    let track = open(nor);
    let scan = track.boot_scan();
    assert_eq!((scan.kind, scan.head, scan.oldest, scan.held, scan.keep), (ScanKind::Ring, SECTOR_RECORDS + 5, 0, SECTOR_RECORDS + 5, None));
}

#[test]
fn garbage_with_a_blank_tail_is_not_a_ring() {
    // an image that ends half way through sector 1, the rest blank: the
    // head candidate after it has garbage before it
    let nor = Nor::new(4, |i| if i < SECTOR_LEN + 1000 { garbage(i) } else { 0xFF });
    let scan = open(nor).boot_scan();
    assert_eq!((scan.kind, scan.head, scan.held, scan.keep), (ScanKind::Garbage, 0, 0, Some(0)));
}

#[test]
fn a_full_ring_cut_off_before_the_erase() {
    let nor = Nor::blank(4);
    let records = nor.records();
    // every record written, the oldest at sector 2: as if the erase after
    // sector 1 filled never happened
    for k in 0..records {
        let at = (2 * SECTOR_RECORDS + k) % records;
        nor.program(at as usize * RECORD_LEN, &Record::encode(k as i64 * 1000, lat(k), 151.0, f64::NAN));
    }
    let track = open(nor);
    let scan = track.boot_scan();
    assert_eq!((scan.kind, scan.head, scan.keep), (ScanKind::Full, 2 * SECTOR_RECORDS, Some(0)));
    assert_eq!((scan.oldest, scan.held), (3 * SECTOR_RECORDS, records - SECTOR_RECORDS));
    assert!(track.region.blank_to_sector_end(2 * SECTOR_RECORDS));
    assert_eq!(track.newest(0).unwrap().t, (records as i64 - 1) * 1000);
}

#[test]
fn served_at_the_layouts_leaves() {
    let mut track = RamTrack::<4>::new();
    track.append(10, -33.0, 151.0);
    // Race's: alt 0, lat 1, lon 2, t 3
    let leaves = FixLeaves { t: 3, lat: 1, lon: 2, alt: 0 };
    let gps = GpsList { track: &track, leaves };
    let l: &dyn ListInput = &gps;
    assert_eq!(l.len(), 1);
    assert_eq!(
        [l.read(0, 0), l.read(0, 1), l.read(0, 2), l.read(0, 3)],
        [Value::Nil, Value::Float(-33.0), Value::Float(151.0), Value::Int(10)]
    );
    assert_eq!(l.read(1, 3), Value::Nil);

    // a flash record with an altitude serves it
    let nor = Nor::blank(2);
    nor.program(0, &Record::encode(10, -33.0, 151.0, 12.5));
    let mut flash = open(nor);
    flash.append(20, -33.1, 151.1);
    flash.run = 2;
    let gps = GpsList { track: &flash, leaves };
    assert_eq!([gps.read(0, 0), gps.read(1, 0)], [Value::Float(12.5), Value::Nil]);
}

#[test]
fn a_ram_ring_of_the_newest_fixes() {
    let mut track = RamTrack::<3>::new();
    assert!(track.len() == 0 && track.last().is_none());
    assert_eq!(track.append(100, 1.0, 2.0), 100);
    // the clock did not move: one ms after the last
    assert_eq!(track.append(100, 3.0, 4.0), 101);
    assert_eq!(track.append(50, 5.0, 6.0), 102);
    assert_eq!(track.append(200, 7.0, 8.0), 200);
    assert_eq!(track.len(), 3);
    assert_eq!(ts(&track), [101, 102, 200]);
    assert_eq!(track.last(), Some(TrackFix { t: 200, lat: 7.0, lon: 8.0, alt: None }));
    assert_eq!(track.get(3), None);
}
