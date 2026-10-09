//! Temporary bridge between the engines' postcard bytes and the bitwire
//! wire (`wire/SPEC.md`), driven by the engine schema's descriptor: views
//! go postcard → wire on their way to clients, events wire → postcard on
//! their way in. Streaming, no allocation: one walk of the descriptor, each
//! value copied straight from one format to the other.
//!
//! It goes when the engines read and write the wire themselves (the engine
//! VM implementing bitwire's `Source`/`Sink` over its slots).
//!
//! # How the Rust types must match the schema
//!
//! Postcard writes values in Rust field order with no names or headers, and
//! the wire writes them in schema order, with the same byte encodings for
//! everything except booleans, enum and union indices and option presence
//! (SPEC §9). So the bridge is exact when the schema mirrors the Rust type
//! field for field and variant for variant, in order:
//!
//! | Rust (postcard) | schema (wire) |
//! |---|---|
//! | `bool` (byte 0/1) | `boolean` (1 bit) |
//! | `u8`/`i8` (raw byte) | `uint8`/`int8` (raw byte) |
//! | `u16`..`u64`, `i16`..`i64` (varint, zigzag) | same format (same bytes) |
//! | `f32`/`f64` (LE) | `float`/`double` (same bytes; NaN made canonical) |
//! | `String`, `&str`, `char` (len + UTF-8) | `string` |
//! | `Vec<u8>`, `&[u8]`, `[u8]` seq (len + bytes) | `string` + `contentEncoding: base64` |
//! | `Vec<T>`, `heapless::Vec<T, N>` (len + items) | `array` |
//! | struct, tuple (fields in order) | `object` (required fields) |
//! | `Option<T>` struct field (byte 0/1 + value) | optional property (presence bit) |
//! | enum of unit variants (varint index) | `enum` (index bits) |
//! | enum with data (varint index + fields) | `oneOf` union (index bits + fields); a unit variant is an empty branch, a newtype or tuple variant's contents are the branch's fields |
//!
//! Postcard has no counterpart for the rest of the schema subset, and the
//! wire none for maps, `u128`/`i128` or `Option` outside a struct field.
//!
//! Errors are [`bitwire::Error`]: malformed postcard input is refused like a
//! malformed wire message (`truncated`, `invalid`, `trailing`); a variant or
//! enum index the schema does not have is `range`; a too-small output
//! buffer is `full`.

use bitwire::{Descriptor, Error, Op, Sink, Source, varint};

/// Converts postcard bytes (an engine's view) into a wire message in `out`;
/// returns its length.
pub fn postcard_to_wire(desc: &Descriptor<'_>, postcard: &[u8], out: &mut [u8]) -> Result<usize, Error> {
    let mut src = PostcardSource { b: postcard, p: 0 };
    let n = bitwire::encode(desc, &mut src, out)?;
    if src.p != postcard.len() {
        return Err(Error::Trailing);
    }
    Ok(n)
}

/// Converts a wire message (a client event) into postcard bytes in `out`;
/// returns their length.
pub fn wire_to_postcard(desc: &Descriptor<'_>, wire: &[u8], out: &mut [u8]) -> Result<usize, Error> {
    let mut sink = PostcardSink { b: out, p: 0 };
    bitwire::decode(desc, wire, &mut sink)?;
    Ok(sink.p)
}

/// Reads postcard bytes in the order the wire encoder asks for values.
struct PostcardSource<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> PostcardSource<'a> {
    fn byte(&mut self) -> Result<u8, Error> {
        let v = *self.b.get(self.p).ok_or(Error::Truncated)?;
        self.p += 1;
        Ok(v)
    }

    /// A postcard `bool` or `Option` tag: exactly 0 or 1, as postcard's
    /// deserializer requires.
    fn flag(&mut self) -> Result<bool, Error> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::Invalid),
        }
    }

    /// A postcard varint of at most `max_bytes`. Read with the wire's
    /// strict rules (canonical, no overflow); serde-postcard only writes
    /// canonical varints.
    fn varint(&mut self, max_bytes: u32) -> Result<u64, Error> {
        varint::read(self.b, &mut self.p, max_bytes)
    }

    /// A `usize` length (postcard writes up to 10 bytes on a 64-bit host);
    /// the wire carries lengths below 2^32.
    fn length(&mut self) -> Result<usize, Error> {
        let v = self.varint(10)?;
        if v > u32::MAX as u64 {
            return Err(Error::Range);
        }
        Ok(v as usize)
    }

    fn blob(&mut self) -> Result<&'a [u8], Error> {
        let len = self.length()?;
        if len > self.b.len() - self.p {
            return Err(Error::Truncated);
        }
        let s = &self.b[self.p..self.p + len];
        self.p += len;
        Ok(s)
    }

    /// An enum discriminant: a `u32` varint.
    fn discriminant(&mut self) -> Result<u32, Error> {
        let v = self.varint(5)?;
        u32::try_from(v).map_err(|_| Error::Invalid)
    }
}

impl Source for PostcardSource<'_> {
    type Error = Error;

    fn bool(&mut self) -> Result<bool, Error> {
        self.flag()
    }

    fn uint(&mut self, op: Op) -> Result<u64, Error> {
        match op {
            Op::U8 => self.byte().map(u64::from),
            Op::U16 => self.varint(3).and_then(|v| if v > u16::MAX as u64 { Err(Error::Invalid) } else { Ok(v) }),
            Op::U32 => self.varint(5).and_then(|v| if v > u32::MAX as u64 { Err(Error::Invalid) } else { Ok(v) }),
            _ => self.varint(10),
        }
    }

    fn int(&mut self, op: Op) -> Result<i64, Error> {
        let z = match op {
            Op::I8 => return self.byte().map(|b| b as i8 as i64),
            Op::I16 => self.varint(3).and_then(|v| if v > u16::MAX as u64 { Err(Error::Invalid) } else { Ok(v) })?,
            Op::I32 => self.varint(5).and_then(|v| if v > u32::MAX as u64 { Err(Error::Invalid) } else { Ok(v) })?,
            _ => self.varint(10)?,
        };
        Ok(varint::unzigzag(z))
    }

    fn f32(&mut self) -> Result<f32, Error> {
        let b = self.b.get(self.p..self.p + 4).ok_or(Error::Truncated)?;
        self.p += 4;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f64(&mut self) -> Result<f64, Error> {
        let b = self.b.get(self.p..self.p + 8).ok_or(Error::Truncated)?;
        self.p += 8;
        Ok(f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    fn string(&mut self) -> Result<&str, Error> {
        let b = self.blob()?;
        core::str::from_utf8(b).map_err(|_| Error::Invalid)
    }

    fn bytes(&mut self) -> Result<&[u8], Error> {
        self.blob()
    }

    fn enum_index(&mut self) -> Result<u32, Error> {
        self.discriminant()
    }

    fn enter_object(&mut self) -> Result<(), Error> {
        Ok(())
    }

    fn field(&mut self, _index: u8, optional: bool) -> Result<bool, Error> {
        if optional { self.flag() } else { Ok(true) }
    }

    fn enter_array(&mut self) -> Result<u32, Error> {
        self.length().map(|n| n as u32)
    }

    fn variant(&mut self) -> Result<u32, Error> {
        self.discriminant()
    }

    fn leave(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

/// Writes postcard bytes as the wire decoder pushes values.
struct PostcardSink<'o> {
    b: &'o mut [u8],
    p: usize,
}

impl PostcardSink<'_> {
    fn put(&mut self, v: &[u8]) -> Result<(), Error> {
        let end = self.p + v.len();
        self.b.get_mut(self.p..end).ok_or(Error::Full)?.copy_from_slice(v);
        self.p = end;
        Ok(())
    }

    fn varint(&mut self, v: u64) -> Result<(), Error> {
        let mut tmp = [0; varint::MAX_BYTES];
        let n = varint::write(v, &mut tmp);
        self.put(&tmp[..n])
    }
}

impl Sink for PostcardSink<'_> {
    type Error = Error;

    fn bool(&mut self, v: bool) -> Result<(), Error> {
        self.put(&[v as u8])
    }

    fn uint(&mut self, op: Op, v: u64) -> Result<(), Error> {
        if op == Op::U8 { self.put(&[v as u8]) } else { self.varint(v) }
    }

    fn int(&mut self, op: Op, v: i64) -> Result<(), Error> {
        if op == Op::I8 { self.put(&[v as i8 as u8]) } else { self.varint(varint::zigzag(v)) }
    }

    fn f32(&mut self, v: f32) -> Result<(), Error> {
        self.put(&v.to_le_bytes())
    }

    fn f64(&mut self, v: f64) -> Result<(), Error> {
        self.put(&v.to_le_bytes())
    }

    fn string(&mut self, v: &str) -> Result<(), Error> {
        self.varint(v.len() as u64)?;
        self.put(v.as_bytes())
    }

    fn bytes(&mut self, v: &[u8]) -> Result<(), Error> {
        self.varint(v.len() as u64)?;
        self.put(v)
    }

    fn enum_index(&mut self, i: u8) -> Result<(), Error> {
        self.varint(i as u64)
    }

    fn enter_object(&mut self) -> Result<(), Error> {
        Ok(())
    }

    fn field(&mut self, _index: u8, optional: bool, present: bool) -> Result<(), Error> {
        if optional { self.put(&[present as u8]) } else { Ok(()) }
    }

    fn enter_array(&mut self, len: u32) -> Result<(), Error> {
        self.varint(len as u64)
    }

    fn variant(&mut self, i: u8) -> Result<(), Error> {
        self.varint(i as u64)
    }

    fn leave(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use serde::{Deserialize, Serialize};
    use serde_json::Value as Json;
    use std::string::{String, ToString};
    use std::vec::Vec;
    use std::{format, vec};

    const FIXTURES: &str = include_str!("../fixtures/wire-bridge/wire-bridge.json");

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{:02x}", x)).collect()
    }

    fn fixtures() -> Json {
        serde_json::from_str(FIXTURES).unwrap()
    }

    fn descriptor_bytes(f: &Json, ty: &str) -> Vec<u8> {
        unhex(f["descriptors"][ty].as_str().unwrap())
    }

    fn to_wire(desc: &Descriptor<'_>, pc: &[u8]) -> Result<Vec<u8>, Error> {
        let mut out = vec![0u8; 4096];
        let n = postcard_to_wire(desc, pc, &mut out)?;
        out.truncate(n);
        Ok(out)
    }

    fn to_postcard(desc: &Descriptor<'_>, w: &[u8]) -> Result<Vec<u8>, Error> {
        let mut out = vec![0u8; 4096];
        let n = wire_to_postcard(desc, w, &mut out)?;
        out.truncate(n);
        Ok(out)
    }

    /// Race and Tune: the Rust encoder's postcard bytes ↔ wire/js's wire
    /// bytes for the same value, both ways.
    #[test]
    fn race_and_tune_pairs() {
        let f = fixtures();
        let mut n = 0;
        for c in f["cases"].as_array().unwrap() {
            let Some(pc) = c["postcard"].as_str() else { continue };
            let ty = c["type"].as_str().unwrap();
            let what = c["what"].as_str().unwrap();
            let db = descriptor_bytes(&f, ty);
            let desc = Descriptor::load(&db).unwrap();
            let (pc, w) = (unhex(pc), unhex(c["wire"].as_str().unwrap()));
            assert_eq!(hex(&to_wire(&desc, &pc).unwrap()), hex(&w), "{ty} {what}: postcard → wire");
            assert_eq!(hex(&to_postcard(&desc, &w).unwrap()), hex(&pc), "{ty} {what}: wire → postcard");
            n += 1;
        }
        std::println!("wire bridge: {n} Race/Tune postcard ↔ wire pairs");
        assert_eq!(n, 27);
    }

    /// The fixture pairs come from the same Rust types the engines use.
    #[test]
    fn race_and_tune_postcard_is_the_engines() {
        let f = fixtures();
        for c in f["cases"].as_array().unwrap() {
            let Some(pc) = c["postcard"].as_str() else { continue };
            let pc = unhex(pc);
            let ty = c["type"].as_str().unwrap();
            let again = match ty {
                "RaceView" => postcard::to_stdvec(&postcard::from_bytes::<extreme_race::RaceView>(&pc).unwrap()),
                "RaceEvent" => postcard::to_stdvec(&postcard::from_bytes::<extreme_race::RaceEvent>(&pc).unwrap()),
                "TuneView" => postcard::to_stdvec(&postcard::from_bytes::<extreme_tune::TuneView>(&pc).unwrap()),
                _ => panic!("{ty}"),
            };
            assert_eq!(again.unwrap(), pc, "{ty} {}", c["what"]);
        }
    }

    // ---- KitchenSink: every kind the bridge maps --------------------------

    #[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
    enum Mode {
        Off,
        Slow,
        Fast,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Point {
        x: i32,
        y: i32,
        label: Option<String>,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Inner {
        on: bool,
        count: u16,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    enum Shape {
        Empty,
        Circle { r: f32 },
        Poly(Vec<Point>),
        Pair(u8, bool),
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct KitchenSink {
        flag: bool,
        small: u8,
        tiny: i8,
        short: i16,
        word: u32,
        big: i64,
        ratio: f64,
        speed: f32,
        name: String,
        note: Option<String>,
        mode: Mode,
        maybe_mode: Option<Mode>,
        blob: Vec<u8>,
        points: Vec<Point>,
        shape: Shape,
        nested: Option<Inner>,
    }

    fn p(x: i32, y: i32, label: Option<&str>) -> Point {
        Point { x, y, label: label.map(ToString::to_string) }
    }

    /// The same values as gen.js's SINK, in its order.
    fn kitchen_sink_cases() -> Vec<(&'static str, KitchenSink)> {
        vec![
            ("everything", KitchenSink {
                flag: true, small: 200, tiny: -5, short: -300, word: 70000, big: -1234567890123,
                ratio: 0.1, speed: 6.4, name: "héllo 😀".into(), note: Some("n".into()), mode: Mode::Fast,
                maybe_mode: Some(Mode::Slow), blob: vec![0, 1, 255], points: vec![p(1, -1, Some("a")), p(-70000, 0, None)],
                shape: Shape::Poly(vec![p(3, 4, None)]), nested: Some(Inner { on: true, count: 65535 }),
            }),
            ("minimal", KitchenSink {
                flag: false, small: 0, tiny: 0, short: 0, word: 0, big: 0, ratio: 0.0, speed: 0.0, name: "".into(),
                note: None, mode: Mode::Off, maybe_mode: None, blob: vec![], points: vec![], shape: Shape::Empty, nested: None,
            }),
            ("circle", KitchenSink {
                flag: true, small: 255, tiny: -128, short: 32767, word: 4294967295, big: 9007199254740991,
                ratio: -2.5, speed: -1e-3, name: "c".into(), note: None, mode: Mode::Slow, maybe_mode: Some(Mode::Off),
                blob: vec![7; 200], points: vec![p(0, 0, Some(""))], shape: Shape::Circle { r: 1.5 },
                nested: Some(Inner { on: false, count: 0 }),
            }),
            ("pair", KitchenSink {
                flag: false, small: 1, tiny: 127, short: -32768, word: 128, big: -9007199254740991,
                ratio: 1e300, speed: f32::MAX, name: "p".into(), note: Some("".into()), mode: Mode::Off, maybe_mode: None,
                blob: vec![9], points: vec![], shape: Shape::Pair(7, true), nested: None,
            }),
        ]
    }

    /// Rust value → postcard → bridge → the wire bytes wire/js wrote for the
    /// same value; and back: wire → bridge → postcard → the same Rust value.
    #[test]
    fn kitchen_sink() {
        let f = fixtures();
        let db = descriptor_bytes(&f, "KitchenSink");
        let desc = Descriptor::load(&db).unwrap();
        let wires: Vec<(&str, Vec<u8>)> = f["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["type"] == "KitchenSink")
            .map(|c| (c["what"].as_str().unwrap(), unhex(c["wire"].as_str().unwrap())))
            .collect();
        let cases = kitchen_sink_cases();
        assert_eq!(wires.len(), cases.len());
        for ((what, value), (wwhat, w)) in cases.iter().zip(&wires) {
            assert_eq!(what, wwhat);
            let pc = postcard::to_stdvec(value).unwrap();
            assert_eq!(hex(&to_wire(&desc, &pc).unwrap()), hex(w), "{what}: postcard → wire");
            let back = to_postcard(&desc, w).unwrap();
            assert_eq!(hex(&back), hex(&pc), "{what}: wire → postcard");
            assert_eq!(&postcard::from_bytes::<KitchenSink>(&back).unwrap(), value, "{what}: value");
        }
        std::println!("wire bridge: {} KitchenSink values both ways", cases.len());
    }

    /// Malformed or mismatched input is refused, never written.
    #[test]
    fn refusals() {
        let f = fixtures();
        let rv = descriptor_bytes(&f, "RaceView");
        let rv = Descriptor::load(&rv).unwrap();
        let re = descriptor_bytes(&f, "RaceEvent");
        let re = Descriptor::load(&re).unwrap();
        let ks = descriptor_bytes(&f, "KitchenSink");
        let ks = Descriptor::load(&ks).unwrap();

        // postcard → wire
        let both = unhex("cdcccc4000032fb4a1c3a28c34");
        for len in 0..both.len() {
            assert_eq!(to_wire(&rv, &both[..len]), Err(Error::Truncated), "postcard cut at {len}");
        }
        let mut long = both.clone();
        long.push(0);
        assert_eq!(to_wire(&rv, &long), Err(Error::Trailing));
        assert_eq!(to_wire(&re, &unhex("04")), Err(Error::Range), "variant 4 of 4");
        assert_eq!(to_wire(&rv, &unhex("cdcccc400004")), Err(Error::Range), "line variant 4 of 4");
        assert_eq!(to_wire(&re, &unhex("8000")), Err(Error::Invalid), "non-canonical discriminant");
        let mut out = [0u8; 4];
        assert_eq!(postcard_to_wire(&rv, &both, &mut out), Err(Error::Full));

        let sink = postcard::to_stdvec(&kitchen_sink_cases().remove(1).1).unwrap();
        let mut bad_bool = sink.clone();
        bad_bool[0] = 2;
        assert_eq!(to_wire(&ks, &bad_bool), Err(Error::Invalid), "bool byte 2");
        // minimal: flag u8 i8 i16 u32 i64 (1 byte each) f64 f32 name(len 0) note(None) mode
        let note = 1 + 1 + 1 + 1 + 1 + 1 + 8 + 4 + 1;
        assert_eq!(sink[note], 0);
        let mut bad_option = sink.clone();
        bad_option[note] = 2;
        assert_eq!(to_wire(&ks, &bad_option), Err(Error::Invalid), "option tag 2");
        let mut bad_mode = sink.clone();
        bad_mode[note + 1] = 3;
        assert_eq!(to_wire(&ks, &bad_mode), Err(Error::Range), "mode 3 of 3");
        let mut bad_utf8 = sink.clone();
        bad_utf8[note - 1] = 1; // name: length 1 ...
        bad_utf8.insert(note, 0xff); // ... of an invalid byte
        assert_eq!(to_wire(&ks, &bad_utf8), Err(Error::Invalid), "invalid UTF-8");

        // wire → postcard: the wire decoder's refusals
        assert_eq!(to_postcard(&re, &unhex("")), Err(Error::Truncated));
        assert_eq!(to_postcard(&re, &unhex("04")), Err(Error::Invalid), "unused bit set");
        assert_eq!(to_postcard(&re, &unhex("0000")), Err(Error::Trailing));
        let w = unhex("9a99b9410ddb80d4a28c342fb4a1c3a28c34");
        let mut out = [0u8; 8];
        assert_eq!(wire_to_postcard(&rv, &w, &mut out), Err(Error::Full));
    }

    /// NaN: postcard keeps any payload; the wire writes the canonical quiet
    /// NaN (SPEC §4.4), and decoding passes the wire's bits through.
    #[test]
    fn nan() {
        let f = fixtures();
        let tv = descriptor_bytes(&f, "TuneView");
        let tv = Descriptor::load(&tv).unwrap();
        let pc = unhex("0100c0ff0000000000000000");
        assert_eq!(hex(&to_wire(&tv, &pc).unwrap()), "0000c07f0000000000000000");
        assert_eq!(hex(&to_postcard(&tv, &pc).unwrap()), "0100c0ff0000000000000000");
    }
}
