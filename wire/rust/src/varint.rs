//! LEB128 varints and zigzag, as the wire writes them (SPEC §4.1). Public
//! so that bridges to formats with the same byte values (postcard) can use
//! the same strict reader.

use crate::Error;

/// The most bytes a 64-bit varint takes.
pub const MAX_BYTES: usize = 10;

/// Reads an unsigned LEB128 varint of at most `max_bytes` (1 to 10) bytes at
/// `*pos`, advancing `*pos` past it.
///
/// Refuses as [`Error::Truncated`] when the bytes end inside the varint, and
/// as [`Error::Invalid`] when it is longer than `max_bytes`, does not fit 64
/// bits (a 10th byte above 1), or is not canonical (more than one byte and
/// a final `0x00`). The caller checks the value against narrower widths.
#[inline]
pub fn read(b: &[u8], pos: &mut usize, max_bytes: u32) -> Result<u64, Error> {
    let mut p = *pos;
    // one-byte fast path
    match b.get(p) {
        Some(&x) if x < 0x80 => {
            *pos = p + 1;
            return Ok(x as u64);
        }
        Some(_) => {}
        None => return Err(Error::Truncated),
    }
    let mut v: u64 = 0;
    let mut i: u32 = 0;
    loop {
        let byte = *b.get(p).ok_or(Error::Truncated)?;
        p += 1;
        let g = (byte & 0x7f) as u64;
        if i == 9 && g > 1 {
            return Err(Error::Invalid);
        }
        v |= g << (7 * i);
        i += 1;
        if byte < 0x80 {
            if byte == 0 {
                // i > 1 here: a lone 0x00 took the fast path
                return Err(Error::Invalid);
            }
            *pos = p;
            return Ok(v);
        }
        if i == max_bytes {
            return Err(Error::Invalid);
        }
    }
}

/// Writes `v` as an unsigned LEB128 varint into `out`, returning the number
/// of bytes written (1 to 10).
#[inline]
pub fn write(mut v: u64, out: &mut [u8; MAX_BYTES]) -> usize {
    let mut i = 0;
    while v >= 0x80 {
        out[i] = (v as u8) | 0x80;
        v >>= 7;
        i += 1;
    }
    out[i] = v as u8;
    i + 1
}

/// Zigzag of a signed value: `2v` for `v ≥ 0`, `−2v − 1` for `v < 0`.
#[inline]
pub const fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// Inverse of [`zigzag`].
#[inline]
pub const fn unzigzag(z: u64) -> i64 {
    ((z >> 1) as i64) ^ -((z & 1) as i64)
}
