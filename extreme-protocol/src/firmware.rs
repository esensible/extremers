//! A firmware image, as the C6's bootloader takes it (ESP-IDF's app image
//! format, what `tools/image` writes and the studio uploads): a 24-byte
//! header, up to 16 segments (an 8-byte header each, then its bytes), zero
//! padding up to the last byte of a 16-byte line, a checksum byte (every
//! segment byte XORed onto [`CHECKSUM_SEED`]), and the *appended digest*:
//! the SHA-256 of everything before it, 32 bytes. That digest is the
//! firmware's identity ([`BundleResponse::Firmware`]'s `sha256`).
//!
//! [`walk`] checks an image's structure, reading it through a callback (a
//! slice on a host, the flash on the C6); hashing is the caller's (no
//! SHA-256 here).
//!
//! [`BundleResponse::Firmware`]: crate::BundleResponse::Firmware

/// The header's first byte.
pub const IMAGE_MAGIC: u8 = 0xE9;
/// The header's chip id (bytes 12-13, u16 LE) for the ESP32-C6.
pub const CHIP_ID_C6: u16 = 13;
/// The header (the extended header included), bytes.
pub const HEADER_LEN: u32 = 24;
/// A segment's header (load address, length), bytes.
pub const SEGMENT_HEADER_LEN: u32 = 8;
/// The appended digest (SHA-256), bytes.
pub const DIGEST_LEN: u32 = 32;
/// Most segments an image has (the bootloader's limit).
pub const SEGMENTS_MAX: u8 = 16;
/// What the checksum byte's XOR starts from.
pub const CHECKSUM_SEED: u8 = 0xEF;

/// Where the header keeps what [`walk`] checks: the segment count (u8), the
/// entry point (u32 LE), the chip id (u16 LE), the appended-digest flag
/// (u8, 1).
pub const AT_SEGMENTS: usize = 1;
pub const AT_ENTRY: usize = 4;
pub const AT_CHIP_ID: usize = 12;
pub const AT_APPEND_DIGEST: usize = 23;

/// Why [`walk`] refused an image, with what its `detail` then says. On the
/// wire as `BundleError::Invalid`'s detail.
pub mod reason {
    /// A read failed: `detail` is the offset.
    pub const READ: u8 = 1;
    /// The first byte isn't [`IMAGE_MAGIC`](super::IMAGE_MAGIC): `detail` is it.
    pub const MAGIC: u8 = 2;
    /// Not an ESP32-C6 image: `detail` is the header's chip id.
    pub const CHIP: u8 = 3;
    /// The header doesn't say a digest is appended: `detail` is its flag.
    pub const NO_DIGEST: u8 = 4;
    /// No segments, or more than [`SEGMENTS_MAX`](super::SEGMENTS_MAX):
    /// `detail` is the count.
    pub const SEGMENTS: u8 = 5;
    /// A segment runs past the image, or its length isn't a multiple of 4:
    /// `detail` is its index.
    pub const SEGMENT: u8 = 6;
    /// The checksum byte isn't the segments': `detail` is the byte the
    /// segments give, then the image's byte above it.
    pub const CHECKSUM: u8 = 7;
    /// The image doesn't end with its digest: `detail` is where the digest
    /// ends.
    pub const LENGTH: u8 = 8;
}

/// What [`walk`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Image {
    /// Where the appended digest starts: the SHA-256 of the bytes before it.
    pub digest_at: u32,
    pub segments: u8,
    /// The entry point.
    pub entry: u32,
}

impl Image {
    /// The image's length: up to the end of its digest.
    pub fn len(&self) -> u32 {
        self.digest_at + DIGEST_LEN
    }
}

/// A read's buffer: word-aligned (the C6's flash reads straight into it).
#[repr(align(4))]
struct Buf([u8; 1024]);

/// Checks the structure of the image in the first `len` bytes that `read`
/// gives: the header (magic, chip, appended digest), every segment within
/// `len` and a multiple of 4 long, the checksum, the digest within `len`.
/// The image may end before `len` ([`Image::len`]); [`check`] wants it to
/// end there. Every `read(offset, buf)` is at a multiple of 4 and a
/// multiple of 4 long. `Err((reason, detail))`, per [`reason`].
pub fn walk<E>(len: u32, mut read: impl FnMut(u32, &mut [u8]) -> Result<(), E>) -> Result<Image, (u8, u32)> {
    let mut buf = Buf([0; 1024]);
    let mut get = |at: u32, n: usize, buf: &mut Buf| -> Result<(), (u8, u32)> {
        if at as u64 + n as u64 > len as u64 {
            return Err((reason::LENGTH, at + n as u32));
        }
        read(at, &mut buf.0[..n]).map_err(|_| (reason::READ, at))
    };

    get(0, HEADER_LEN as usize, &mut buf)?;
    let header = &buf.0[..HEADER_LEN as usize];
    if header[0] != IMAGE_MAGIC {
        return Err((reason::MAGIC, header[0] as u32));
    }
    let chip = u16::from_le_bytes([header[AT_CHIP_ID], header[AT_CHIP_ID + 1]]);
    if chip != CHIP_ID_C6 {
        return Err((reason::CHIP, chip as u32));
    }
    if header[AT_APPEND_DIGEST] != 1 {
        return Err((reason::NO_DIGEST, header[AT_APPEND_DIGEST] as u32));
    }
    let segments = header[AT_SEGMENTS];
    if segments == 0 || segments > SEGMENTS_MAX {
        return Err((reason::SEGMENTS, segments as u32));
    }
    let entry = u32::from_le_bytes(header[AT_ENTRY..AT_ENTRY + 4].try_into().unwrap_or_default());

    let mut checksum = CHECKSUM_SEED;
    let mut at = HEADER_LEN;
    for i in 0..segments {
        let overrun = (reason::SEGMENT, i as u32);
        get(at, SEGMENT_HEADER_LEN as usize, &mut buf).map_err(|e| if e.0 == reason::LENGTH { overrun } else { e })?;
        let n = u32::from_le_bytes(buf.0[4..8].try_into().unwrap_or_default());
        at += SEGMENT_HEADER_LEN;
        if n % 4 != 0 || at as u64 + n as u64 > len as u64 {
            return Err(overrun);
        }
        let end = at + n;
        while at < end {
            let k = ((end - at) as usize).min(buf.0.len());
            get(at, k, &mut buf)?;
            checksum = buf.0[..k].iter().fold(checksum, |c, b| c ^ b);
            at += k as u32;
        }
    }
    // padding, then the checksum as the last byte of its 16-byte line; the
    // digest starts the next line
    let digest_at = (at / 16 + 1) * 16;
    get(digest_at - 16, 16, &mut buf)?;
    let stored = buf.0[15];
    if stored != checksum {
        return Err((reason::CHECKSUM, checksum as u32 | (stored as u32) << 8));
    }
    if digest_at as u64 + DIGEST_LEN as u64 > len as u64 {
        return Err((reason::LENGTH, digest_at + DIGEST_LEN));
    }
    Ok(Image { digest_at, segments, entry })
}

/// [`walk`], and the image is exactly `len` bytes: it ends with its digest.
pub fn check<E>(len: u32, read: impl FnMut(u32, &mut [u8]) -> Result<(), E>) -> Result<Image, (u8, u32)> {
    let image = walk(len, read)?;
    if image.len() != len {
        return Err((reason::LENGTH, image.len()));
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    /// An image to test with: `segments` (each a multiple of 4 long), its
    /// checksum, and `digest` where the SHA-256 goes.
    fn synthetic(segments: &[&[u8]], digest: [u8; 32]) -> std::vec::Vec<u8> {
        let mut out = std::vec![0u8; HEADER_LEN as usize];
        out[0] = IMAGE_MAGIC;
        out[AT_SEGMENTS] = segments.len() as u8;
        out[AT_ENTRY..AT_ENTRY + 4].copy_from_slice(&0x4080_0000u32.to_le_bytes());
        out[AT_CHIP_ID..AT_CHIP_ID + 2].copy_from_slice(&CHIP_ID_C6.to_le_bytes());
        out[AT_APPEND_DIGEST] = 1;
        let mut checksum = CHECKSUM_SEED;
        for (i, s) in segments.iter().enumerate() {
            out.extend_from_slice(&(0x4200_0000u32 + 0x1_0000 * i as u32).to_le_bytes());
            out.extend_from_slice(&(s.len() as u32).to_le_bytes());
            out.extend_from_slice(s);
            checksum = s.iter().fold(checksum, |c, b| c ^ b);
        }
        while out.len() % 16 != 15 {
            out.push(0);
        }
        out.push(checksum);
        out.extend_from_slice(&digest);
        out
    }

    fn walk_bytes(bytes: &[u8]) -> Result<Image, (u8, u32)> {
        check(bytes.len() as u32, |at, buf: &mut [u8]| {
            assert!(at % 4 == 0 && buf.len() % 4 == 0, "unaligned read at {at}, {} B", buf.len());
            buf.copy_from_slice(&bytes[at as usize..at as usize + buf.len()]);
            Ok::<(), ()>(())
        })
    }

    fn good() -> std::vec::Vec<u8> {
        // the second segment takes more than one read
        let long: std::vec::Vec<u8> = (0..3000u32).map(|i| (i * 7) as u8).collect();
        synthetic(&[&[1, 2, 3, 4, 5, 6, 7, 8], &long], [0xAB; 32])
    }

    #[test]
    fn a_good_image() {
        let image = good();
        let found = walk_bytes(&image).unwrap();
        assert_eq!(found.len() as usize, image.len());
        assert_eq!(found.digest_at % 16, 0);
        assert_eq!(&image[found.digest_at as usize..], &[0xAB; 32]);
        assert_eq!((found.segments, found.entry), (2, 0x4080_0000));
        // a bound past its end: walk takes it, check doesn't
        let mut longer = image.clone();
        longer.extend_from_slice(&[0xFF; 64]);
        let read = |at: u32, buf: &mut [u8]| {
            buf.copy_from_slice(&longer[at as usize..at as usize + buf.len()]);
            Ok::<(), ()>(())
        };
        assert_eq!(walk(longer.len() as u32, read), Ok(found));
        assert_eq!(check(longer.len() as u32, read), Err((reason::LENGTH, image.len() as u32)));
    }

    #[test]
    fn refusals() {
        let image = good();
        let with = |at: usize, b: u8| {
            let mut i = image.clone();
            i[at] = b;
            i
        };
        assert_eq!(walk_bytes(&with(0, 0xE8)), Err((reason::MAGIC, 0xE8)));
        assert_eq!(walk_bytes(&with(AT_CHIP_ID, 5)), Err((reason::CHIP, 5)));
        assert_eq!(walk_bytes(&with(AT_APPEND_DIGEST, 0)), Err((reason::NO_DIGEST, 0)));
        assert_eq!(walk_bytes(&with(AT_SEGMENTS, 0)), Err((reason::SEGMENTS, 0)));
        assert_eq!(walk_bytes(&with(AT_SEGMENTS, 17)), Err((reason::SEGMENTS, 17)));
        // a segment byte changed: the checksum no longer holds
        let flipped = with(HEADER_LEN as usize + 8, 0x01 ^ 0x80);
        let Err((reason::CHECKSUM, detail)) = walk_bytes(&flipped) else { panic!() };
        assert_eq!(detail >> 8, (detail & 0xFF) ^ 0x80);
        // the second segment's length past the end, or not a multiple of 4
        let second = HEADER_LEN as usize + 8 + 8 + 4;
        let mut long = image.clone();
        long[second..second + 4].copy_from_slice(&100_000u32.to_le_bytes());
        assert_eq!(walk_bytes(&long), Err((reason::SEGMENT, 1)));
        let mut odd = image.clone();
        odd[second..second + 4].copy_from_slice(&2999u32.to_le_bytes());
        assert_eq!(walk_bytes(&odd), Err((reason::SEGMENT, 1)));
        // a segment header itself past the end
        assert_eq!(walk_bytes(&with(AT_SEGMENTS, 3)), Err((reason::SEGMENT, 2)));
        // truncated: the digest cut short; one byte too many
        assert_eq!(walk_bytes(&image[..image.len() - 1]), Err((reason::LENGTH, image.len() as u32)));
        let mut extra = image.clone();
        extra.push(0);
        assert_eq!(walk_bytes(&extra), Err((reason::LENGTH, image.len() as u32)));
        // a read that fails
        let failing = |at: u32, buf: &mut [u8]| {
            if at >= 32 {
                return Err(());
            }
            buf.copy_from_slice(&image[at as usize..at as usize + buf.len()]);
            Ok(())
        };
        assert_eq!(walk(image.len() as u32, failing), Err((reason::READ, 32)));
    }

    /// The real image (`target/xiaoc6.bin`, which `cargo run -p
    /// extreme-image -- <elf>` writes), when it has been built.
    #[test]
    fn the_real_image() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../target/xiaoc6.bin");
        let Ok(image) = std::fs::read(path) else {
            std::eprintln!("{path} not built (cargo run -p extreme-image -- <elf>): skipped");
            return;
        };
        let found = walk_bytes(&image).unwrap();
        assert_eq!(found.len() as usize, image.len());
        assert!(found.segments > 1);
        // a byte flipped in a segment: the checksum no longer holds
        let mut flipped = image.clone();
        flipped[HEADER_LEN as usize + SEGMENT_HEADER_LEN as usize + 1000] ^= 0x10;
        assert!(matches!(walk_bytes(&flipped), Err((reason::CHECKSUM, _))));
    }
}
