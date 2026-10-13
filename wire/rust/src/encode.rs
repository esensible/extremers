//! Encoding: walks the descriptor and pulls each value from a [`Source`]
//! (SPEC §3, §4).

use crate::{Descriptor, Error, Node, Op, varint};

/// Supplies a value to encode, value by value, in walk order. The mirror of
/// [`Sink`](crate::Sink): the walk asks, the source answers.
///
/// - a scalar is one call; the walk says which ([`uint`](Source::uint) and
///   [`int`](Source::int) get the exact op);
/// - an object is [`enter_object`](Source::enter_object), then for every
///   declared field in order [`field`](Source::field) (is it present?)
///   followed, when present, by its value, then [`leave`](Source::leave);
/// - an array is [`enter_array`](Source::enter_array) (the length), that
///   many values, then [`leave`](Source::leave);
/// - a union is [`variant`](Source::variant) (the branch index), then the
///   branch's fields as an object's, then [`leave`](Source::leave);
/// - a record with variants ([`Op::Variants`]) is
///   [`enter_object`](Source::enter_object), its plain fields as an
///   object's, then [`variant`](Source::variant) (which variant), then that
///   variant's value (an object or a record with variants, with its own
///   enter and leave), then [`leave`](Source::leave).
///
/// The source reports a value of the wrong type for what is asked as
/// [`Error::Shape`] (SPEC §7), and so a record value with no variant or more
/// than one when asked [`variant`](Source::variant). The walk itself refuses what cannot be
/// written: an integer outside the op's width, an enum index or variant ≥
/// the count ([`Error::Range`]; a source that cannot map a name returns an
/// out-of-range index, e.g. `u32::MAX`), and a required field reported
/// absent ([`Error::Shape`]).
pub trait Source {
    /// The error type of the encode: the source's own, or [`Error`].
    type Error: From<Error>;

    /// A `boolean`.
    fn bool(&mut self) -> Result<bool, Self::Error>;
    /// An unsigned integer for `op` ([`Op::U8`], `U16`, `U32` or `U64`); the
    /// walk refuses a value above the op's width.
    fn uint(&mut self, op: Op) -> Result<u64, Self::Error>;
    /// A signed integer for `op` ([`Op::I8`], `I16`, `I32` or `I64`); the
    /// walk refuses a value outside the op's width.
    fn int(&mut self, op: Op) -> Result<i64, Self::Error>;
    /// A `float`. NaN is written as the canonical quiet NaN.
    fn f32(&mut self) -> Result<f32, Self::Error>;
    /// A `double`. NaN is written as the canonical quiet NaN.
    fn f64(&mut self) -> Result<f64, Self::Error>;
    /// A `string`.
    fn string(&mut self) -> Result<&str, Self::Error>;
    /// A byte string.
    fn bytes(&mut self) -> Result<&[u8], Self::Error>;
    /// An enum value's index.
    fn enum_index(&mut self) -> Result<u32, Self::Error>;
    /// The start of an object.
    fn enter_object(&mut self) -> Result<(), Self::Error>;
    /// Whether field `index` of the current object or union branch is
    /// present; asked for every field in declared order. `optional` is the
    /// field's optional flag.
    fn field(&mut self, index: u8, optional: bool) -> Result<bool, Self::Error>;
    /// The start of an array: its length.
    fn enter_array(&mut self) -> Result<u32, Self::Error>;
    /// The start of a union value: the branch index; or, in a record with
    /// variants, after its plain fields: the variant index.
    fn variant(&mut self) -> Result<u32, Self::Error>;
    /// The end of the innermost object, array, union or record.
    fn leave(&mut self) -> Result<(), Self::Error>;
}

/// Encodes the value `src` supplies with `desc` into `out`, returning the
/// message length. Refuses with [`Error::Shape`] / [`Error::Range`] (SPEC
/// §7) or [`Error::Full`] when `out` is too small, converted to the
/// source's error type; a source error is returned as is.
pub fn encode<S: Source + ?Sized>(desc: &Descriptor<'_>, src: &mut S, out: &mut [u8]) -> Result<usize, S::Error> {
    let mut w = Writer { b: out, p: 0, bb: 0, used: 8 };
    w.node(desc.root(), src)?;
    Ok(w.p)
}

struct Writer<'o> {
    b: &'o mut [u8],
    p: usize,
    /// position of the current bit byte
    bb: usize,
    /// bits used in it; 8 = no room
    used: u32,
}

impl Writer<'_> {
    #[inline]
    fn bits(&mut self, v: u32, w: u32) -> Result<(), Error> {
        if self.used + w > 8 {
            self.byte(0)?;
            self.bb = self.p - 1;
            self.used = 0;
        }
        self.b[self.bb] |= (v << self.used) as u8;
        self.used += w;
        Ok(())
    }

    #[inline]
    fn byte(&mut self, v: u8) -> Result<(), Error> {
        *self.b.get_mut(self.p).ok_or(Error::Full)? = v;
        self.p += 1;
        Ok(())
    }

    #[inline]
    fn put(&mut self, v: &[u8]) -> Result<(), Error> {
        let end = self.p.checked_add(v.len()).ok_or(Error::Full)?;
        self.b.get_mut(self.p..end).ok_or(Error::Full)?.copy_from_slice(v);
        self.p = end;
        Ok(())
    }

    #[inline]
    fn varint(&mut self, v: u64) -> Result<(), Error> {
        if v < 0x80 {
            return self.byte(v as u8);
        }
        let mut tmp = [0; varint::MAX_BYTES];
        let n = varint::write(v, &mut tmp);
        self.put(&tmp[..n])
    }

    /// A string or byte-string length then the bytes.
    #[inline]
    fn blob(&mut self, v: &[u8]) -> Result<(), Error> {
        if v.len() > u32::MAX as usize {
            return Err(Error::Range);
        }
        self.varint(v.len() as u64)?;
        self.put(v)
    }

    /// An enum, union or variant index, checked against the count.
    #[inline]
    fn index(&mut self, n: &Node<'_>, i: u32) -> Result<(), Error> {
        if i as usize >= n.choices() {
            return Err(Error::Range);
        }
        if n.bits() != 0 {
            self.bits(i, n.bits())?;
        }
        Ok(())
    }

    fn node<S: Source + ?Sized>(&mut self, n: Node<'_>, s: &mut S) -> Result<(), S::Error> {
        match n.op() {
            Op::Bool => {
                let v = s.bool()?;
                self.bits(v as u32, 1)?;
            }
            Op::U8 => {
                let v = s.uint(Op::U8)?;
                let v = u8::try_from(v).map_err(|_| Error::Range)?;
                self.byte(v)?;
            }
            Op::I8 => {
                let v = s.int(Op::I8)?;
                let v = i8::try_from(v).map_err(|_| Error::Range)?;
                self.byte(v as u8)?;
            }
            op @ (Op::U16 | Op::U32 | Op::U64) => {
                let v = s.uint(op)?;
                if v > op.uint_max() {
                    return Err(Error::Range.into());
                }
                self.varint(v)?;
            }
            op @ (Op::I16 | Op::I32 | Op::I64) => {
                let v = s.int(op)?;
                let (min, max) = op.int_range();
                if v < min || v > max {
                    return Err(Error::Range.into());
                }
                self.varint(varint::zigzag(v))?;
            }
            Op::F32 => {
                let v = s.f32()?;
                let bits = if v.is_nan() { 0x7fc0_0000 } else { v.to_bits() };
                self.put(&bits.to_le_bytes())?;
            }
            Op::F64 => {
                let v = s.f64()?;
                let bits = if v.is_nan() { 0x7ff8_0000_0000_0000 } else { v.to_bits() };
                self.put(&bits.to_le_bytes())?;
            }
            Op::String => {
                let v = s.string()?;
                self.blob(v.as_bytes())?;
            }
            Op::Bytes => {
                let v = s.bytes()?;
                self.blob(v)?;
            }
            Op::Enum => {
                let i = s.enum_index()?;
                self.index(&n, i)?;
            }
            Op::Object => {
                s.enter_object()?;
                self.fields(n, s)?;
                s.leave()?;
            }
            Op::Array => {
                let len = s.enter_array()?;
                self.varint(len as u64)?;
                let item = n.item();
                for _ in 0..len {
                    self.node(item, s)?;
                }
                s.leave()?;
            }
            Op::Union => {
                let i = s.variant()?;
                self.index(&n, i)?;
                if let Some(b) = n.branch(i as usize) {
                    self.fields(b, s)?;
                }
                s.leave()?;
            }
            Op::Variants => {
                s.enter_object()?;
                self.fields(n, s)?;
                let i = s.variant()?;
                self.index(&n, i)?;
                if let Some(v) = n.branch(i as usize) {
                    self.node(v, s)?;
                }
                s.leave()?;
            }
        }
        Ok(())
    }

    fn fields<S: Source + ?Sized>(&mut self, obj: Node<'_>, s: &mut S) -> Result<(), S::Error> {
        for (i, f) in obj.children().enumerate() {
            let optional = f.is_optional();
            let present = s.field(i as u8, optional)?;
            if optional {
                self.bits(present as u32, 1)?;
            } else if !present {
                return Err(Error::Shape.into());
            }
            if present {
                self.node(f, s)?;
            }
        }
        Ok(())
    }
}
