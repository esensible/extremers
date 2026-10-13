//! The bundle store's record (the plan's A5): each bundle in the bundles
//! region (on the C6, the first 512 KiB of the inactive app slot) is a
//! sector-aligned extent, a 64-byte record then the archive at +64.
//! Little-endian:
//!
//! ```text
//!  0  "XBS1"
//!  4  state u32    FFFFFFFF writing → FFFF0000 committed → 00000000 deleted (bits only cleared)
//!  8  seq u32      install order
//! 12  id u32       the manifest's
//! 16  len u32      the archive's bytes
//! 20  sectors u16  the extent, record included
//! 22  FFFF
//! 24  sha256[32]   of the archive
//! 56  crc32        (IEEE) of bytes 0..4 and 8..56: everything but the state
//! 60  FFFFFFFF
//! ```

/// The record's first four bytes.
pub const MAGIC: [u8; 4] = *b"XBS1";
/// The record's size; the archive follows it.
pub const RECORD_BYTES: usize = 64;
/// A flash sector: extents are whole sectors.
pub const SECTOR: usize = 4096;

pub const WRITING: u32 = 0xFFFF_FFFF;
pub const COMMITTED: u32 = 0xFFFF_0000;
pub const DELETED: u32 = 0;
/// Where the state word is in a record: committing and deleting write these
/// four bytes alone (`state.to_le_bytes()`), each only clearing bits.
pub const STATE_OFFSET: usize = 4;

/// A store record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub state: u32,
    pub seq: u32,
    pub id: u32,
    pub len: u32,
    pub sectors: u16,
    pub sha256: [u8; 32],
}

/// Why bytes aren't a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Fewer than [`RECORD_BYTES`].
    Truncated,
    /// Not `"XBS1"` (e.g. an erased sector).
    Magic,
    /// The CRC, a fixed field, or the state isn't one of the three.
    Corrupt,
    /// The extent is too small for `len`.
    Extent,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl Record {
    /// The sectors an archive of `len` bytes needs, record included.
    pub fn sectors_for(len: u32) -> u16 {
        (RECORD_BYTES + len as usize).div_ceil(SECTOR) as u16
    }

    pub fn parse(b: &[u8]) -> Result<Record, Error> {
        let b = b.get(..RECORD_BYTES).ok_or(Error::Truncated)?;
        if b[..4] != MAGIC {
            return Err(Error::Magic);
        }
        let state = u32_at(b, 4);
        if u32_at(b, 56) != crc(b) || b[22..24] != [0xFF, 0xFF] || b[60..64] != [0xFF; 4] || !matches!(state, WRITING | COMMITTED | DELETED) {
            return Err(Error::Corrupt);
        }
        let mut sha256 = [0; 32];
        sha256.copy_from_slice(&b[24..56]);
        let r = Record { state, seq: u32_at(b, 8), id: u32_at(b, 12), len: u32_at(b, 16), sectors: u16::from_le_bytes([b[20], b[21]]), sha256 };
        if r.sectors < Record::sectors_for(r.len) {
            return Err(Error::Extent);
        }
        Ok(r)
    }

    pub fn to_bytes(&self) -> [u8; RECORD_BYTES] {
        let mut b = [0xFF; RECORD_BYTES];
        b[..4].copy_from_slice(&MAGIC);
        b[4..8].copy_from_slice(&self.state.to_le_bytes());
        b[8..12].copy_from_slice(&self.seq.to_le_bytes());
        b[12..16].copy_from_slice(&self.id.to_le_bytes());
        b[16..20].copy_from_slice(&self.len.to_le_bytes());
        b[20..22].copy_from_slice(&self.sectors.to_le_bytes());
        b[24..56].copy_from_slice(&self.sha256);
        let c = crc(&b);
        b[56..60].copy_from_slice(&c.to_le_bytes());
        b
    }
}

/// CRC-32 (IEEE 802.3, reflected, as zlib) of a record's bytes 0..4 and
/// 8..56.
fn crc(b: &[u8]) -> u32 {
    crc32(b[..4].iter().chain(&b[8..56]).copied())
}

fn crc32(bytes: impl Iterator<Item = u8>) -> u32 {
    let mut c = !0u32;
    for x in bytes {
        c ^= x as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// An image of the bundles region, `size` bytes, holding `bundles`
/// (archives) committed, in order: extents from sector 0, back to back,
/// `seq` 1 to n, each `id` from its manifest; the rest erased (0xFF). For
/// flashing a device directly.
///
/// Panics if an archive has no readable manifest or they don't fit.
#[cfg(feature = "std")]
pub fn store_image(bundles: &[&[u8]], size: usize) -> std::vec::Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut img = std::vec![0xFF; size];
    let mut at = 0;
    for (i, b) in bundles.iter().enumerate() {
        let a = crate::archive::Archive::parse(b).expect("a bundle archive");
        let m: extreme_protocol::Manifest = postcard::from_bytes(a.file("manifest.bin").expect("a manifest")).expect("a manifest");
        let len = u32::try_from(b.len()).expect("an archive under 4 GiB");
        let r = Record { state: COMMITTED, seq: i as u32 + 1, id: m.id, len, sectors: Record::sectors_for(len), sha256: Sha256::digest(b).into() };
        let end = at + r.sectors as usize * SECTOR;
        assert!(end <= size, "the bundles need more than {size} bytes");
        img[at..at + RECORD_BYTES].copy_from_slice(&r.to_bytes());
        img[at + RECORD_BYTES..at + RECORD_BYTES + b.len()].copy_from_slice(b);
        at = end;
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_is_ieee() {
        // CRC-32/ISO-HDLC's check value
        assert_eq!(crc32(b"123456789".iter().copied()), 0xCBF4_3926);
    }

    #[test]
    fn a_record_round_trips() {
        let r = Record { state: COMMITTED, seq: 3, id: 0x1234_5678, len: 5000, sectors: Record::sectors_for(5000), sha256: [7; 32] };
        assert_eq!(r.sectors, 2);
        let b = r.to_bytes();
        assert_eq!(Record::parse(&b), Ok(r));
        // committing and deleting only clear bits, and the CRC leaves the state out
        let mut w = Record { state: WRITING, ..r }.to_bytes();
        assert_eq!(&w[56..60], &b[56..60]);
        w[STATE_OFFSET..STATE_OFFSET + 4].copy_from_slice(&DELETED.to_le_bytes());
        assert_eq!(Record::parse(&w).map(|r| r.state), Ok(DELETED));
        let mut bad = b;
        bad[13] ^= 1;
        assert_eq!(Record::parse(&bad), Err(Error::Corrupt));
        assert_eq!(Record::parse(&[0xFF; 64]), Err(Error::Magic));
    }
}
