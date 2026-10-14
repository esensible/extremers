//! The store on a RAM model of NOR flash (track/tests.rs's `Nor`), with
//! the real Race bundle (`target/packages/race.bundle`, which `cargo run -p
//! extreme-pack -- --bundle` writes) and copies of it under other names.

extern crate std;

use super::*;
use crate::{engine::LIMITS, track::tests::Nor};
use bundle::store::store_image;
use extreme_protocol::{Manifest, package_id};
use sha2::{Digest, Sha256};
use std::{vec, vec::Vec};

const RACE_BUNDLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../target/packages/race.bundle");
const SECTORS: usize = MAX_SECTORS;

fn race() -> Vec<u8> {
    std::fs::read(RACE_BUNDLE).unwrap_or_else(|e| panic!("{RACE_BUNDLE}: {e}: build it with `cargo run -p extreme-pack -- --bundle`"))
}

/// Race's archive as the app `name`, at `version`.
fn renamed(name: &str, version: u16) -> Vec<u8> {
    let race = race();
    let archive = bundle::Archive::parse(&race).unwrap();
    let mut manifest: Manifest = postcard::from_bytes(archive.file("manifest.bin").unwrap()).unwrap();
    manifest.id = package_id(name);
    manifest.name = name.try_into().unwrap();
    manifest.version = version;
    let manifest = postcard::to_stdvec(&manifest).unwrap();
    let mut files: Vec<(&str, &[u8])> = archive.files().filter(|(n, _)| *n != "manifest.bin").collect();
    files.push(("manifest.bin", &manifest));
    bundle::archive::write(&files)
}

/// A region holding `bundles` as `extreme-pack --store-image` writes it.
fn imaged(bundles: &[&[u8]]) -> Nor {
    let nor = Nor::blank(SECTORS);
    nor.program(0, &store_image(bundles, SECTORS * SECTOR));
    nor
}

fn open(nor: Nor) -> (BundleStore<Nor>, Vec<Package>) {
    let mut scratch = vec![0u16; LIMITS.code_bytes as usize];
    let (store, apps) = BundleStore::open(nor, &LIMITS, &mut scratch);
    (store, apps.into_iter().collect())
}

fn names(apps: &[Package]) -> Vec<&'static str> {
    apps.iter().map(|p| p.name).collect()
}

/// The extents, as (first sector, state, seq, id).
fn extents(store: &BundleStore<Nor>) -> Vec<(u16, u32, u32, PackageId)> {
    store
        .entries()
        .filter_map(|i| match i {
            Item::Extent { first, record } => Some((first, record.state, record.seq, record.id)),
            Item::Free { .. } => None,
        })
        .collect()
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn accept_all(_: &Package) -> Result<(), InstallError> {
    Ok(())
}

/// Uploads `bytes` as `id` in chunks of `chunk` bytes and commits it.
fn upload(store: &mut BundleStore<Nor>, id: PackageId, bytes: &[u8], chunk: usize) -> Result<Package, Error> {
    store.begin(id, bytes.len() as u32, &sha(bytes), true)?;
    for (i, c) in bytes.chunks(chunk).enumerate() {
        store.write((i * chunk) as u32, c)?;
    }
    let mut scratch = vec![0u16; LIMITS.code_bytes as usize];
    store.commit(&LIMITS, &mut scratch, &mut accept_all)
}

fn sectors_of(bytes: &[u8]) -> u16 {
    Record::sectors_for(bytes.len() as u32)
}

#[test]
fn an_erased_region_holds_nothing() {
    let (store, apps) = open(Nor::blank(SECTORS));
    assert!(apps.is_empty());
    assert_eq!(store.space(), Space { free_bytes: (SECTORS * SECTOR) as u32, largest_free: (SECTORS * SECTOR - RECORD_BYTES) as u32 });
    assert_eq!(store.entries().count(), SECTORS);
}

#[test]
fn a_store_image_opens_in_seq_order() {
    let (race, b) = (race(), renamed("beta", 1));
    let (store, apps) = open(imaged(&[&race, &b]));
    assert_eq!(names(&apps), ["race", "beta"]);
    let (r, bs) = (sectors_of(&race), sectors_of(&b));
    assert_eq!(extents(&store), [(0, COMMITTED, 1, package_id("race")), (r, COMMITTED, 2, package_id("beta"))]);
    // in place: the package's bytes are the region's
    assert_eq!(apps[0].archive.bytes().as_ptr(), unsafe { store.flash.bytes().as_ptr().add(RECORD_BYTES) });
    let free = SECTORS as u32 - (r + bs) as u32;
    assert_eq!(store.space().free_bytes, free * SECTOR as u32);
    assert_eq!(store.space().largest_free, free * SECTOR as u32 - RECORD_BYTES as u32);
    // the next upload's seq is past them
    assert_eq!(store.next_seq, 3);
}

#[test]
fn of_two_with_one_id_the_higher_seq_wins() {
    let (old, new) = (renamed("race", 1), renamed("race", 2));
    let nor = imaged(&[&new, &old]);
    // the first extent is the newer: seq 5
    let r = Record::parse(&nor.bytes()[..RECORD_BYTES]).unwrap();
    nor.mem[..RECORD_BYTES].iter().for_each(|c| c.set(0xFF));
    nor.program(0, &Record { seq: 5, ..r }.to_bytes());
    let (store, apps) = open(nor);
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].version, 2);
    // the loser is marked deleted in flash, and stays so
    let id = package_id("race");
    assert_eq!(extents(&store), [(0, COMMITTED, 5, id), (sectors_of(&new), DELETED, 2, id)]);
    let (store, apps) = open(nor);
    assert_eq!(apps.len(), 1);
    assert_eq!(extents(&store)[1].1, DELETED);
    assert_eq!(store.next_seq, 6);
}

#[test]
fn a_bundle_that_does_not_validate_is_skipped_and_kept() {
    let (race, b) = (race(), renamed("beta", 1));
    let nor = imaged(&[&race, &b]);
    // race's archive magic broken
    nor.mem[RECORD_BYTES].set(0);
    let (store, apps) = open(nor);
    assert_eq!(names(&apps), ["beta"]);
    // not erased, not marked: a later firmware may read it
    assert_eq!(extents(&store)[0].1, COMMITTED);
    assert_eq!(nor.erases.get(), 0);
    // still live: it can be deleted by id, not overwritten
    assert_eq!(store.space().free_bytes, (SECTORS as u32 - (sectors_of(&race) + sectors_of(&b)) as u32) * SECTOR as u32);
}

#[test]
fn an_upload_round_trips() {
    let nor = Nor::blank(SECTORS);
    let (mut store, _) = open(nor);
    let race = race();
    // chunks of 240, as over BLE at an MTU of 247, the last shorter
    let p = upload(&mut store, package_id("race"), &race, 240).unwrap();
    assert_eq!(p.name, "race");
    assert_eq!(p.archive.bytes(), &race[..]);
    assert!(store.uploading().is_none());
    let (_, apps) = open(nor);
    assert_eq!(names(&apps), ["race"]);
    assert_eq!(apps[0].archive.bytes(), &race[..]);
}

#[test]
fn an_unaligned_tail_is_padded() {
    // an archive is a multiple of 8 bytes, but the upload doesn't rely on
    // that: 4099 bytes, hashed as sent, written with one 0xFF pad byte
    let odd: Vec<u8> = (0..4099u32).map(|i| (i % 253) as u8).collect();
    let nor = Nor::blank(SECTORS);
    let (mut store, _) = open(nor);
    // the hash matches (the bytes were written and hashed exactly); it
    // just isn't a bundle
    assert!(matches!(upload(&mut store, 9, &odd, 4096), Err(Error::Invalid(_))));
    let at = RECORD_BYTES;
    assert_eq!(&nor.bytes()[at..at + odd.len()], &odd[..]);
    assert_eq!(nor.bytes()[at + odd.len()], 0xFF);
}

#[test]
fn chunks_go_in_order() {
    let race = race();
    let (mut store, _) = open(Nor::blank(SECTORS));
    let id = package_id("race");
    assert_eq!(store.write(0, &race[..4]), Err(Error::NoUpload));
    store.begin(id, race.len() as u32, &sha(&race), true).unwrap();
    assert_eq!(store.begin(id, race.len() as u32, &sha(&race), true), Err(Error::Busy));
    assert_eq!(store.write(0, &race[..240]), Ok(240));
    assert_eq!(store.write(480, &race[480..720]), Err(Error::Sequence { expected: 240 }));
    // only the last chunk may be other than a multiple of 4
    assert_eq!(store.write(240, &race[240..243]), Err(Error::Length));
    assert_eq!(store.write(240, &race), Err(Error::Length));
    let mut scratch = vec![0u16; LIMITS.code_bytes as usize];
    assert_eq!(store.commit(&LIMITS, &mut scratch, &mut accept_all).err(), Some(Error::Length));
    // still going after those
    assert_eq!(store.uploading().map(|u| u.written), Some(240));
    assert!(store.abort());
    assert!(!store.abort());
    assert_eq!(extents(&store)[0].1, DELETED);
}

#[test]
fn a_wrong_hash_or_a_refusal_ends_the_upload_deleted() {
    let race = race();
    let id = package_id("race");
    let (mut store, _) = open(Nor::blank(SECTORS));
    let mut scratch = vec![0u16; LIMITS.code_bytes as usize];

    store.begin(id, race.len() as u32, &[0; 32], true).unwrap();
    store.write(0, &race).unwrap();
    assert_eq!(store.commit(&LIMITS, &mut scratch, &mut accept_all).err(), Some(Error::Hash));
    assert!(store.uploading().is_none());

    // a manifest whose id isn't the one declared
    store.begin(package_id("other"), race.len() as u32, &sha(&race), true).unwrap();
    store.write(0, &race).unwrap();
    assert_eq!(store.commit(&LIMITS, &mut scratch, &mut accept_all).err(), Some(Error::Id));

    // not a bundle
    let junk = vec![0x5A; 1000];
    store.begin(id, 1000, &sha(&junk), true).unwrap();
    store.write(0, &junk).unwrap();
    assert!(matches!(store.commit(&LIMITS, &mut scratch, &mut accept_all), Err(Error::Invalid(_))));

    // the registry's refusal
    store.begin(id, race.len() as u32, &sha(&race), true).unwrap();
    store.write(0, &race).unwrap();
    assert_eq!(store.commit(&LIMITS, &mut scratch, &mut |_| Err(InstallError::Full)).err(), Some(Error::Install(InstallError::Full)));

    assert!(extents(&store).iter().all(|e| e.1 == DELETED), "{:?}", extents(&store));
    let (_, apps) = open(store.flash);
    assert!(apps.is_empty());
}

#[test]
fn an_upload_cut_off_is_free_at_the_next_boot() {
    let race = race();
    let nor = Nor::blank(SECTORS);
    let (mut store, _) = open(nor);
    store.begin(package_id("race"), race.len() as u32, &sha(&race), true).unwrap();
    store.write(0, &race[..9000]).unwrap();
    // reboot
    let (mut store, apps) = open(nor);
    assert!(apps.is_empty());
    assert_eq!(extents(&store), [(0, WRITING, 1, package_id("race"))]);
    assert_eq!(store.space().free_bytes, (SECTORS * SECTOR) as u32);
    // its sectors are taken again, erased as they are reached
    let erases = nor.erases.get();
    let p = upload(&mut store, package_id("race"), &race, 240).unwrap();
    assert_eq!(p.archive.bytes().as_ptr(), unsafe { nor.bytes().as_ptr().add(RECORD_BYTES) });
    // sector 0 (the record) and the two the 9000 bytes reached
    assert_eq!(nor.erases.get() - erases, 3);
    assert_eq!(extents(&store), [(0, COMMITTED, 2, package_id("race"))]);
}

#[test]
fn a_replacement_goes_beside_and_wins() {
    let (v1, v2) = (renamed("race", 1), renamed("race", 2));
    let nor = imaged(&[&v1]);
    let (mut store, _) = open(nor);
    let p = upload(&mut store, package_id("race"), &v2, 240).unwrap();
    assert_eq!(p.version, 2);
    let n = sectors_of(&v1);
    let id = package_id("race");
    assert_eq!(extents(&store), [(0, DELETED, 1, id), (n, COMMITTED, 2, id)]);
    // the old one is stale: something may still be reading it
    assert_eq!(store.stale, mask(0, n));
    let (_, apps) = open(nor);
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].version, 2);
}

#[test]
fn first_fit_and_delete_then_reuse() {
    let (a, b, c) = (renamed("alpha", 1), renamed("beta", 1), renamed("gamma", 1));
    let nor = imaged(&[&a, &b, &c]);
    let (mut store, _) = open(nor);
    let n = sectors_of(&a);
    assert_eq!(store.delete(package_id("beta")), Ok(()));
    assert_eq!(store.delete(package_id("beta")), Err(Error::NotFound));
    let free = SECTORS as u16 - 3 * n;
    // beta's sectors are free, but stale until the readers are gone
    assert_eq!(store.space().free_bytes, (free + n) as u32 * SECTOR as u32);
    assert_eq!(store.space().largest_free, free as u32 * SECTOR as u32 - RECORD_BYTES as u32);
    let d = renamed("delta", 1);
    assert_eq!(store.begin(package_id("delta"), d.len() as u32, &sha(&d), false), Err(Error::Leased));
    // with no reader it goes in the first run that holds it: beta's
    let erases = nor.erases.get();
    upload(&mut store, package_id("delta"), &d, 512).unwrap();
    assert_eq!(nor.erases.get() - erases, n as u32);
    assert_eq!(store.stale, 0);
    assert_eq!(
        extents(&store),
        [
            (0, COMMITTED, 1, package_id("alpha")),
            (n, COMMITTED, 4, package_id("delta")),
            (2 * n, COMMITTED, 3, package_id("gamma"))
        ]
    );
    // a deleted one from before this boot is not stale
    store.delete(package_id("alpha")).unwrap();
    let (mut store, apps) = open(nor);
    assert_eq!(names(&apps), ["gamma", "delta"]);
    assert_eq!(store.begin(package_id("alpha"), a.len() as u32, &sha(&a), false), Ok(()));
    assert_eq!(store.uploading().map(|u| u.target), Some(Target::Bundle(package_id("alpha"))));
}

#[test]
fn a_run_needs_contiguous_sectors() {
    // three bundles fill most of the region; the gaps between them don't
    // add up to a run
    let (a, b, c, d) = (renamed("alpha", 1), renamed("beta", 1), renamed("gamma", 1), renamed("delta", 1));
    let nor = imaged(&[&a, &b, &c, &d]);
    let (mut store, _) = open(nor);
    let n = sectors_of(&a) as u32;
    let tail = SECTORS as u32 - 4 * n;
    store.delete(package_id("beta")).unwrap();
    // a bundle a sector longer than beta's run or the tail
    let len = (n.max(tail) * SECTOR as u32) as u32;
    assert_eq!(store.begin(package_id("epsilon"), len, &[0; 32], true), Err(Error::NoSpace));
    // with alpha gone too, alpha's and beta's runs are one
    store.delete(package_id("alpha")).unwrap();
    assert_eq!(store.begin(package_id("epsilon"), len, &[0; 32], true), Ok(()));
    assert_eq!(extents(&store)[0], (0, WRITING, 5, package_id("epsilon")));
    // the walk after it lands in beta's old data: free single sectors
    assert_eq!(store.entries().find(|i| i.first() == Record::sectors_for(len)).map(|i| i.sectors()), Some(1));
}

#[test]
fn too_large_or_empty() {
    let (mut store, _) = open(Nor::blank(SECTORS));
    assert_eq!(store.begin(1, 0, &[0; 32], true), Err(Error::Length));
    assert_eq!(store.begin(1, (SECTORS * SECTOR) as u32, &[0; 32], true), Err(Error::NoSpace));
    assert_eq!(store.begin(1, (SECTORS * SECTOR - RECORD_BYTES) as u32, &[0; 32], true), Ok(()));
    assert_eq!(store.uploading().map(|u| u.len), Some((SECTORS * SECTOR - RECORD_BYTES) as u32));
}

#[test]
fn garbage_is_free_sectors() {
    let nor = Nor::new(SECTORS, |i| (i * 7 % 251) as u8);
    let (mut store, apps) = open(nor);
    assert!(apps.is_empty());
    assert_eq!(store.entries().count(), SECTORS);
    let race = race();
    upload(&mut store, package_id("race"), &race, 1024).unwrap();
    let (_, apps) = open(nor);
    assert_eq!(names(&apps), ["race"]);
}
