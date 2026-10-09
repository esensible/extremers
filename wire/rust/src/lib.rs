//! bitwire (working name): a compact binary codec whose message types are
//! written in a subset of JSON Schema. This crate is the Rust interpreter:
//! it loads a compiled **descriptor** (made by the JavaScript compiler,
//! `js/compile.js`), validates it once, and then decodes and encodes
//! messages by walking it. See `SPEC.md` for the format.
//!
//! `no_std`, no `alloc`, no dependencies. Values never pass through a
//! generic value tree: decoding pushes each value into a [`Sink`], encoding
//! pulls each value from a [`Source`], both in walk order (a depth-first
//! walk of the value in schema order). A typed host, such as an engine VM
//! holding values in slots, implements them directly.
//!
//! ```
//! use bitwire::{Descriptor, Error, Op, Sink};
//!
//! // {"type":"integer","format":"uint16"}
//! let desc = Descriptor::load(&[1, 0x56, 0x5d, 0x31, 0x8e, 0x2a, 0x45, 0x9d, 0x2c, 1, 4]).unwrap();
//!
//! struct Last(u64);
//! impl Sink for Last {
//!     type Error = Error;
//!     fn uint(&mut self, _op: Op, v: u64) -> Result<(), Error> { self.0 = v; Ok(()) }
//!     fn bool(&mut self, _: bool) -> Result<(), Error> { Ok(()) }
//!     fn int(&mut self, _: Op, _: i64) -> Result<(), Error> { Ok(()) }
//!     fn f32(&mut self, _: f32) -> Result<(), Error> { Ok(()) }
//!     fn f64(&mut self, _: f64) -> Result<(), Error> { Ok(()) }
//!     fn string(&mut self, _: &str) -> Result<(), Error> { Ok(()) }
//!     fn bytes(&mut self, _: &[u8]) -> Result<(), Error> { Ok(()) }
//!     fn enum_index(&mut self, _: u8) -> Result<(), Error> { Ok(()) }
//!     fn enter_object(&mut self) -> Result<(), Error> { Ok(()) }
//!     fn field(&mut self, _: u8, _: bool, _: bool) -> Result<(), Error> { Ok(()) }
//!     fn enter_array(&mut self, _: u32) -> Result<(), Error> { Ok(()) }
//!     fn variant(&mut self, _: u8) -> Result<(), Error> { Ok(()) }
//!     fn leave(&mut self) -> Result<(), Error> { Ok(()) }
//! }
//! let mut sink = Last(0);
//! bitwire::decode(&desc, &[0xac, 0x02], &mut sink).unwrap();
//! assert_eq!(sink.0, 300);
//! ```
#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod decode;
mod descriptor;
mod encode;
pub mod varint;

pub use decode::{Sink, decode};
pub use descriptor::{Children, Descriptor, FORMAT_VERSION, MAX_DEPTH, MAX_FIELDS, Names, Node};
pub use encode::{Source, encode};

use core::fmt;

/// Why a descriptor, message or value was refused. The first six are the
/// codes of SPEC §7; [`Error::Full`] is this crate's own (the caller's
/// output buffer is too small) and never a property of the value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Error {
    /// Load: the descriptor fails the checks of SPEC §6.3.
    Descriptor,
    /// Decode: the message ends before the value does.
    Truncated,
    /// Decode: bytes follow the end of the value.
    Trailing,
    /// Decode: malformed bytes (a non-canonical or too wide varint, a length
    /// above 2^32 − 1, an enum or union index ≥ n, non-zero unused bits in
    /// a bit byte, invalid UTF-8).
    Invalid,
    /// Encode: the value has the wrong type for its node, or a required
    /// field is missing. A [`Source`] reports wrong types itself; the walk
    /// reports a missing required field.
    Shape,
    /// Encode: the value has the right type but cannot be written: an
    /// integer outside its width, an enum index or union variant ≥ n.
    Range,
    /// Encode: the output buffer is too small for the message.
    Full,
}

impl Error {
    /// The SPEC §7 code (`"descriptor"`, `"truncated"`, ...), or `"full"`.
    pub const fn code(self) -> &'static str {
        match self {
            Error::Descriptor => "descriptor",
            Error::Truncated => "truncated",
            Error::Trailing => "trailing",
            Error::Invalid => "invalid",
            Error::Shape => "shape",
            Error::Range => "range",
            Error::Full => "full",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl core::error::Error for Error {}

/// A descriptor node's operation (SPEC §6.1). The discriminants are the op
/// bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Op {
    /// `boolean`: one bit.
    Bool = 1,
    /// `uint8`: one raw byte.
    U8 = 2,
    /// `int8`: one raw byte, two's complement.
    I8 = 3,
    /// `uint16`: unsigned varint.
    U16 = 4,
    /// `int16`: zigzag varint.
    I16 = 5,
    /// `uint32`: unsigned varint.
    U32 = 6,
    /// `int32`: zigzag varint.
    I32 = 7,
    /// `uint64`: unsigned varint.
    U64 = 8,
    /// `int64` (and `integer` without a format): zigzag varint.
    I64 = 9,
    /// `float`: binary32, little-endian.
    F32 = 10,
    /// `double` (and `number` without a format): binary64, little-endian.
    F64 = 11,
    /// `string`: length varint + UTF-8.
    String = 12,
    /// `string` with `contentEncoding: base64`: length varint + raw bytes.
    Bytes = 13,
    /// `enum` / `const`: an index in `bitLength(n − 1)` bits.
    Enum = 14,
    /// `object`: the fields in declared order.
    Object = 15,
    /// `array`: length varint + the items.
    Array = 16,
    /// `oneOf` tagged union: a branch index in `bitLength(n − 1)` bits + the
    /// branch's fields.
    Union = 17,
}

impl Op {
    /// The op for an op byte with the optional flag cleared, if valid.
    pub const fn from_u8(b: u8) -> Option<Op> {
        Some(match b {
            1 => Op::Bool,
            2 => Op::U8,
            3 => Op::I8,
            4 => Op::U16,
            5 => Op::I16,
            6 => Op::U32,
            7 => Op::I32,
            8 => Op::U64,
            9 => Op::I64,
            10 => Op::F32,
            11 => Op::F64,
            12 => Op::String,
            13 => Op::Bytes,
            14 => Op::Enum,
            15 => Op::Object,
            16 => Op::Array,
            17 => Op::Union,
            _ => return None,
        })
    }

    /// For the integer ops: the most bytes the varint may take (0 for the
    /// raw-byte ops and every other op).
    pub(crate) const fn varint_bytes(self) -> u32 {
        match self {
            Op::U16 | Op::I16 => 3,
            Op::U32 | Op::I32 => 5,
            Op::U64 | Op::I64 => 10,
            _ => 0,
        }
    }

    /// For the unsigned integer ops: the largest value.
    pub const fn uint_max(self) -> u64 {
        match self {
            Op::U8 => u8::MAX as u64,
            Op::U16 => u16::MAX as u64,
            Op::U32 => u32::MAX as u64,
            _ => u64::MAX,
        }
    }

    /// For the signed integer ops: the smallest and largest values.
    pub const fn int_range(self) -> (i64, i64) {
        match self {
            Op::I8 => (i8::MIN as i64, i8::MAX as i64),
            Op::I16 => (i16::MIN as i64, i16::MAX as i64),
            Op::I32 => (i32::MIN as i64, i32::MAX as i64),
            _ => (i64::MIN, i64::MAX),
        }
    }
}
