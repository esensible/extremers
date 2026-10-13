//! The archive format: `write` then `parse` gives the files back, in name
//! order, whatever order they were given in; every truncation and every
//! departure from the canonical layout is refused.

use bundle::archive::{Archive, Error, write};

fn files() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("web/index.html", b"<!doctype html>"),
        ("engine.bin", b"ENG1\x01\x00"),
        ("manifest.bin", b"\x01\x02\x03"),
        ("empty", b""),
        ("web/bundle.js", b"(function(){})()"),
    ]
}

#[test]
fn round_trips() {
    let bytes = write(&files());
    let a = Archive::parse(&bytes).unwrap();
    let mut want = files();
    want.sort_by(|a, b| a.0.cmp(b.0));
    let got: Vec<(&str, &[u8])> = a.files().collect();
    assert_eq!(got, want);
    for (name, data) in &want {
        assert_eq!(a.file(name), Some(*data));
    }
    assert_eq!(a.file("web"), None);
    assert_eq!(a.file("zzz"), None);
    assert_eq!(a.len(), 5);
    // the same files in another order: the same bytes
    let mut rev = files();
    rev.reverse();
    assert_eq!(write(&rev), bytes);
    // re-packing what was read: the same bytes
    assert_eq!(write(&got), bytes);
    assert_eq!(bytes.len() % 8, 0);
    for (_, data) in a.files() {
        assert_eq!((data.as_ptr() as usize - bytes.as_ptr() as usize) % 8, 0, "8-aligned");
    }
}

#[test]
fn an_empty_archive() {
    let bytes = write(&[]);
    assert_eq!(bytes, b"XBND\x01\x00\x00\x00");
    assert!(Archive::parse(&bytes).unwrap().is_empty());
}

#[test]
fn every_truncation_is_refused() {
    let bytes = write(&files());
    for n in 0..bytes.len() {
        assert!(Archive::parse(&bytes[..n]).is_err(), "{n} bytes");
    }
}

#[test]
fn every_flipped_bit_in_the_table_and_names_is_refused_or_harmless() {
    // Any change to the header, table or names either is refused or still
    // parses into an archive whose every file lies inside it (no panic, no
    // out-of-bounds read).
    let bytes = write(&files());
    let names_end = 8 + 16 * 5 + files().iter().map(|f| f.0.len()).sum::<usize>();
    for i in 0..names_end {
        for bit in 0..8 {
            let mut b = bytes.clone();
            b[i] ^= 1 << bit;
            if let Ok(a) = Archive::parse(&b) {
                assert!(a.files().all(|(_, d)| d.len() <= b.len()));
            }
        }
    }
}

#[test]
fn non_canonical_archives_are_refused() {
    let bytes = write(&files());
    let mut b = bytes.clone();
    b[0] = b'Y';
    assert_eq!(Archive::parse(&b).unwrap_err(), Error::Magic);
    let mut b = bytes.clone();
    b[4] = 2;
    assert_eq!(Archive::parse(&b).unwrap_err(), Error::Format);
    // trailing bytes
    let mut b = bytes.clone();
    b.extend_from_slice(&[0; 8]);
    assert_eq!(Archive::parse(&b).unwrap_err(), Error::Layout);
    // non-zero padding (after the last file)
    let mut b = bytes.clone();
    *b.last_mut().unwrap() = 1;
    assert_eq!(Archive::parse(&b).unwrap_err(), Error::Layout);
    // a reserved field
    let mut b = bytes.clone();
    b[8 + 6] = 1;
    assert_eq!(Archive::parse(&b).unwrap_err(), Error::Layout);
    // names out of order: swap the first two names' bytes ("empty" ↔ "engin")
    let names = 8 + 16 * 5;
    let mut b = bytes.clone();
    b[names..names + 5].copy_from_slice(b"zmpty");
    assert_eq!(Archive::parse(&b).unwrap_err(), Error::Names);
}

#[test]
#[should_panic(expected = "unique")]
fn duplicate_names_are_a_mistake() {
    write(&[("a", b"1"), ("a", b"2")]);
}
