//! The sink on a RAM model of NOR flash (track/tests.rs's `Nor`), with
//! synthetic images and, when it has been built, the real one
//! (`target/xiaoc6.bin`, which `cargo run -p extreme-image -- <elf>` writes).

extern crate std;

use super::*;
use crate::track::tests::Nor;
use extreme_protocol::firmware::{AT_APPEND_DIGEST, AT_CHIP_ID, AT_ENTRY, AT_SEGMENTS, CHECKSUM_SEED, CHIP_ID_C6, HEADER_LEN, IMAGE_MAGIC, reason};
use std::{vec, vec::Vec};

const IMAGE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../target/xiaoc6.bin");

/// An image of one segment of `n` pseudo-random bytes (a multiple of 4),
/// its checksum and its appended digest.
pub(crate) fn image(n: usize) -> Vec<u8> {
    let mut x = 0x2545_f491u32;
    let segment: Vec<u8> = (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    let mut out = vec![0u8; HEADER_LEN as usize];
    out[0] = IMAGE_MAGIC;
    out[AT_SEGMENTS] = 1;
    out[AT_ENTRY..AT_ENTRY + 4].copy_from_slice(&0x4080_0000u32.to_le_bytes());
    out[AT_CHIP_ID..AT_CHIP_ID + 2].copy_from_slice(&CHIP_ID_C6.to_le_bytes());
    out[AT_APPEND_DIGEST] = 1;
    out.extend_from_slice(&0x4200_0000u32.to_le_bytes());
    out.extend_from_slice(&(n as u32).to_le_bytes());
    out.extend_from_slice(&segment);
    while out.len() % 16 != 15 {
        out.push(0);
    }
    out.push(segment.iter().fold(CHECKSUM_SEED, |c, b| c ^ b));
    let digest: [u8; 32] = Sha256::digest(&out).into();
    out.extend_from_slice(&digest);
    out
}

pub(crate) fn digest(image: &[u8]) -> [u8; 32] {
    image[image.len() - 32..].try_into().unwrap()
}

/// Writes `image` from `from`, `chunk` bytes at a time; the last answer.
fn send(sink: &mut FirmwareSink<Nor>, image: &[u8], from: usize, chunk: usize) -> Result<u32, Error> {
    let mut last = Ok(from as u32);
    for at in (from..image.len()).step_by(chunk) {
        last = sink.write(at as u32, &image[at..(at + chunk).min(image.len())]);
        last?;
    }
    last
}

fn bytes(nor: &Nor, range: core::ops::Range<usize>) -> Vec<u8> {
    nor.bytes()[range].to_vec()
}

/// In order, 240 bytes at a time: every sector the image reaches erased
/// first (they held something), the image byte for byte, the rest of its
/// last sector 0xFF, the sectors past it untouched; then the digest.
#[test]
fn an_image_is_written_and_checked() {
    let img = image(9000);
    let nor = Nor::new(4, |i| (i * 7) as u8);
    let mut sink = FirmwareSink::new(nor);
    assert_eq!(sink.max_len(), 4 * SECTOR_LEN as u32);
    sink.begin(img.len() as u32, &digest(&img)).unwrap();
    assert_eq!(nor.erases.get(), 0, "begin writes nothing");
    assert_eq!(sink.uploading(), Some(Uploading { target: Target::Firmware, len: img.len() as u32, sha256: digest(&img), written: 0 }));
    assert_eq!(send(&mut sink, &img, 0, 240), Ok(img.len() as u32));
    // 9056 bytes: sectors 0..=2
    assert_eq!(nor.erases.get(), 3);
    assert_eq!(bytes(&nor, 0..img.len()), img);
    assert!(bytes(&nor, img.len()..3 * SECTOR_LEN).iter().all(|&b| b == 0xFF));
    assert_eq!(bytes(&nor, 3 * SECTOR_LEN..4 * SECTOR_LEN), (3 * SECTOR_LEN..4 * SECTOR_LEN).map(|i| (i * 7) as u8).collect::<Vec<_>>());
    assert_eq!(sink.finish(), Ok(digest(&img)));
    assert_eq!(sink.uploading(), None);
    assert_eq!(sink.finish(), Err(Error::NoUpload));
}

/// A sector that is blank already is not erased.
#[test]
fn only_sectors_that_are_not_blank_are_erased() {
    let img = image(9000);
    let nor = Nor::new(4, |i| if i / SECTOR_LEN == 1 { 0xFF } else { 0 });
    let mut sink = FirmwareSink::new(nor);
    sink.begin(img.len() as u32, &digest(&img)).unwrap();
    send(&mut sink, &img, 0, 240).unwrap();
    assert_eq!(nor.erases.get(), 2);
    assert_eq!(sink.finish(), Ok(digest(&img)));
}

/// Bytes past where it is up to are refused with that offset; bytes it has
/// are too (the host ignores those); it goes on from the offset named, and
/// a new begin starts afresh.
#[test]
fn gaps_and_resuming() {
    let img = image(9000);
    let nor = Nor::blank(4);
    let mut sink = FirmwareSink::new(nor);
    sink.begin(img.len() as u32, &digest(&img)).unwrap();
    assert_eq!(sink.write(0, &img[..240]), Ok(240));
    assert_eq!(sink.write(480, &img[480..720]), Err(Error::Sequence { expected: 240 }));
    assert_eq!(sink.write(0, &img[..240]), Err(Error::Sequence { expected: 240 }));
    // a chunk that isn't words, other than the last
    assert_eq!(sink.write(240, &img[240..243]), Err(Error::Length));
    assert_eq!(sink.uploading().map(|u| u.written), Some(240));
    // too early to finish: the upload goes on
    assert_eq!(sink.finish(), Err(Error::Length));
    assert_eq!(send(&mut sink, &img, 240, 244), Ok(img.len() as u32));
    // past the end
    assert_eq!(sink.write(img.len() as u32, &[0; 4]), Err(Error::Length));
    assert_eq!(sink.finish(), Ok(digest(&img)));
    // aborted half way; nothing to write to after
    sink.begin(img.len() as u32, &digest(&img)).unwrap();
    send(&mut sink, &img[..4800], 0, 240).unwrap();
    assert!(sink.abort());
    assert!(!sink.abort());
    assert_eq!(sink.write(4800, &img[4800..5040]), Err(Error::NoUpload));
}

/// A wrong digest, a flipped byte, an image that doesn't check out: refused
/// at the finish, and the upload is over.
#[test]
fn the_finish_says_why_it_refuses() {
    let img = image(9000);
    let run = |img: &[u8], digest: [u8; 32]| {
        let mut sink = FirmwareSink::new(Nor::blank(4));
        sink.begin(img.len() as u32, &digest).unwrap();
        send(&mut sink, img, 0, 240).unwrap();
        let r = sink.finish();
        assert_eq!(sink.uploading(), None);
        r
    };
    // one byte flipped in the segment: its hash isn't the digest
    let mut flipped = img.clone();
    flipped[100] ^= 1;
    assert_eq!(run(&flipped, digest(&img)), Err(Error::Hash));
    // a digest the image doesn't end with (begin said another)
    let mut other = digest(&img);
    other[0] ^= 1;
    assert_eq!(run(&img, other), Err(Error::Hash));
    // hashed right, but not an image: the magic, then the checksum
    let mut magic = img.clone();
    magic[0] = 0xE8;
    let n = magic.len() - 32;
    let d: [u8; 32] = Sha256::digest(&magic[..n]).into();
    magic[n..].copy_from_slice(&d);
    assert_eq!(run(&magic, d), Err(Error::Image { reason: reason::MAGIC, detail: 0xE8 }));
    let mut sum = img.clone();
    sum[200] ^= 0x80;
    let d: [u8; 32] = Sha256::digest(&sum[..n]).into();
    sum[n..].copy_from_slice(&d);
    assert!(matches!(run(&sum, d), Err(Error::Image { reason: reason::CHECKSUM, .. })));
    // lengths it can't take
    let mut sink = FirmwareSink::new(Nor::blank(4));
    assert_eq!(sink.begin(4 * SECTOR_LEN as u32 + 16, &[0; 32]), Err(Error::NoSpace));
    assert_eq!(sink.begin(32, &[0; 32]), Err(Error::Length));
    assert_eq!(sink.uploading(), None);
}

/// The real image, as the studio uploads it, into a slot the size of the
/// C6's: when it has been built.
#[test]
fn the_real_image() {
    let Ok(img) = std::fs::read(IMAGE) else {
        std::eprintln!("{IMAGE} not built (cargo run -p extreme-image -- <elf>): skipped");
        return;
    };
    let nor = Nor::new(480, |_| 0);
    let mut sink = FirmwareSink::new(nor);
    sink.begin(img.len() as u32, &digest(&img)).unwrap();
    assert_eq!(send(&mut sink, &img, 0, 240), Ok(img.len() as u32));
    assert_eq!(nor.erases.get() as usize, img.len().div_ceil(SECTOR_LEN));
    assert_eq!(sink.finish(), Ok(digest(&img)));
}
