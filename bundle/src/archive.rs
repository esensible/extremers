//! The bundle archive (the plan's A2): files by name in one flat byte
//! string, little-endian:
//!
//! ```text
//! "XBND"  format u16 = 1  file_count u16
//! file_count × {name_off u32, name_len u16, 0 u16, data_off u32, data_len u32}
//! names (back to back, in table order)
//! data  (each file 8-aligned; zero padding after the names and after each file)
//! ```
//!
//! Offsets are from the archive's start. Names are UTF-8, sorted bytewise
//! and unique. The layout is canonical: [`Archive::parse`] accepts exactly
//! what [`write`] produces for the same files, so an archive's bytes are a
//! function of its files.

/// The archive's first four bytes.
pub const MAGIC: [u8; 4] = *b"XBND";
/// The format this crate reads and writes.
pub const FORMAT: u16 = 1;
const HEADER: usize = 8;
const ENTRY: usize = 16;

/// Why bytes aren't an archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not `"XBND"`.
    Magic,
    /// Another format version.
    Format,
    /// The bytes end before the table, a name or a file does.
    Truncated,
    /// A name isn't UTF-8, or names aren't sorted and unique.
    Names,
    /// Something isn't where the canonical layout puts it: a reserved field
    /// not zero, a name or file elsewhere, non-zero padding, bytes after
    /// the last file.
    Layout,
}

/// A parsed archive, borrowing its bytes.
#[derive(Clone, Copy, Debug)]
pub struct Archive<'a> {
    bytes: &'a [u8],
    count: u16,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

const fn align8(n: usize) -> usize {
    (n + 7) & !7
}

impl<'a> Archive<'a> {
    /// Checks every bound, the names' order, and the canonical layout.
    pub fn parse(bytes: &'a [u8]) -> Result<Archive<'a>, Error> {
        if bytes.len() < HEADER {
            return Err(if bytes.len() >= 4 && bytes[..4] != MAGIC { Error::Magic } else { Error::Truncated });
        }
        if bytes[..4] != MAGIC {
            return Err(Error::Magic);
        }
        if u16_at(bytes, 4) != FORMAT {
            return Err(Error::Format);
        }
        let count = u16_at(bytes, 6);
        let table_end = HEADER + ENTRY * count as usize;
        if bytes.len() < table_end {
            return Err(Error::Truncated);
        }
        let a = Archive { bytes, count };
        // Names follow the table back to back, then the data, file after file.
        let mut name_at = table_end;
        let mut prev: Option<&[u8]> = None;
        for i in 0..count as usize {
            let e = HEADER + ENTRY * i;
            let (off, len) = (u32_at(bytes, e) as usize, u16_at(bytes, e + 4) as usize);
            if u16_at(bytes, e + 6) != 0 || off != name_at {
                return Err(Error::Layout);
            }
            let name = bytes.get(off..off + len).ok_or(Error::Truncated)?;
            if core::str::from_utf8(name).is_err() || prev.is_some_and(|p| p >= name) {
                return Err(Error::Names);
            }
            prev = Some(name);
            name_at += len;
        }
        let mut data_at = align8(name_at);
        for i in 0..count as usize {
            let e = HEADER + ENTRY * i;
            let (off, len) = (u32_at(bytes, e + 8) as usize, u32_at(bytes, e + 12) as usize);
            if off != data_at {
                return Err(Error::Layout);
            }
            let end = off.checked_add(len).ok_or(Error::Truncated)?;
            if end > bytes.len() {
                return Err(Error::Truncated);
            }
            data_at = align8(end);
        }
        if bytes.len() < data_at {
            return Err(Error::Truncated);
        }
        if bytes.len() != data_at {
            return Err(Error::Layout);
        }
        // Every byte outside the names and the files is zero padding.
        let mut at = name_at;
        for i in 0..count as usize {
            let e = HEADER + ENTRY * i;
            let off = u32_at(bytes, e + 8) as usize;
            if bytes[at..off].iter().any(|&b| b != 0) {
                return Err(Error::Layout);
            }
            at = off + u32_at(bytes, e + 12) as usize;
        }
        if bytes[at..].iter().any(|&b| b != 0) {
            return Err(Error::Layout);
        }
        Ok(a)
    }

    /// The archive's bytes.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// How many files it holds.
    pub fn len(&self) -> usize {
        self.count as usize
    }

    /// Whether it holds no files.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn entry(&self, i: usize) -> (&'a str, &'a [u8]) {
        let b = self.bytes;
        let e = HEADER + ENTRY * i;
        let (no, nl) = (u32_at(b, e) as usize, u16_at(b, e + 4) as usize);
        let (d, dl) = (u32_at(b, e + 8) as usize, u32_at(b, e + 12) as usize);
        // checked by `parse`
        (core::str::from_utf8(&b[no..no + nl]).unwrap_or(""), &b[d..d + dl])
    }

    /// The file `name`, if the archive has it (a binary search: names are sorted).
    pub fn file(&self, name: &str) -> Option<&'a [u8]> {
        let (mut lo, mut hi) = (0, self.count as usize);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let (n, data) = self.entry(mid);
            match n.as_bytes().cmp(name.as_bytes()) {
                core::cmp::Ordering::Equal => return Some(data),
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
            }
        }
        None
    }

    /// Every file, `(name, bytes)`, in name order.
    pub fn files(&self) -> impl Iterator<Item = (&'a str, &'a [u8])> + 'a {
        let a = *self;
        (0..a.count as usize).map(move |i| a.entry(i))
    }
}

/// The archive of `files`, sorted by name. Deterministic: the same files
/// give the same bytes, whatever their order here.
///
/// Panics if two files have the same name, a name is longer than 65,535
/// bytes, or there are more than 65,535 files: the caller builds the list.
#[cfg(feature = "std")]
pub fn write(files: &[(&str, &[u8])]) -> std::vec::Vec<u8> {
    let mut files = files.to_vec();
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    assert!(files.windows(2).all(|w| w[0].0 != w[1].0), "a bundle's file names are unique");
    let count = u16::try_from(files.len()).expect("at most 65,535 files");
    let mut out = std::vec::Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FORMAT.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    let mut name_at = HEADER + ENTRY * files.len();
    let mut data_at = align8(name_at + files.iter().map(|f| f.0.len()).sum::<usize>());
    for (name, data) in &files {
        out.extend_from_slice(&(name_at as u32).to_le_bytes());
        out.extend_from_slice(&u16::try_from(name.len()).expect("a name of at most 65,535 bytes").to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(data_at as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        name_at += name.len();
        data_at = align8(data_at + data.len());
    }
    for (name, _) in &files {
        out.extend_from_slice(name.as_bytes());
    }
    for (_, data) in &files {
        out.resize(align8(out.len()), 0);
        out.extend_from_slice(data);
    }
    out.resize(align8(out.len()), 0);
    out
}
