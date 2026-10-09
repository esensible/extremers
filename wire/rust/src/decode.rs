//! Decoding: walks the descriptor and the message together and pushes each
//! value into a [`Sink`] (SPEC §3, §4).

use crate::{Descriptor, Error, Node, Op, varint};

/// Receives a decoded message, value by value, in walk order.
///
/// The calls nest like the value:
///
/// - a scalar is one call: [`bool`](Sink::bool), [`uint`](Sink::uint),
///   [`int`](Sink::int), [`f32`](Sink::f32), [`f64`](Sink::f64),
///   [`string`](Sink::string), [`bytes`](Sink::bytes) or
///   [`enum_index`](Sink::enum_index);
/// - an object is [`enter_object`](Sink::enter_object), then for every
///   declared field in order [`field`](Sink::field) followed, when the field
///   is present, by its value, then [`leave`](Sink::leave);
/// - an array is [`enter_array`](Sink::enter_array)`(len)`, `len` values,
///   then [`leave`](Sink::leave);
/// - a union is [`variant`](Sink::variant)`(i)`, then branch `i`'s fields
///   exactly as an object's (its tag is the variant, not a field), then
///   [`leave`](Sink::leave).
///
/// Decoding streams: when [`decode`] fails (say on trailing bytes), the
/// sink has already seen the values before the fault and must discard
/// them. Any call may return an error, which stops the decode and is
/// returned from it.
pub trait Sink {
    /// The error type of the decode: the sink's own, or [`Error`].
    type Error: From<Error>;

    /// A `boolean`.
    fn bool(&mut self, v: bool) -> Result<(), Self::Error>;
    /// An unsigned integer; `op` is [`Op::U8`], `U16`, `U32` or `U64`, and
    /// `v` fits it.
    fn uint(&mut self, op: Op, v: u64) -> Result<(), Self::Error>;
    /// A signed integer; `op` is [`Op::I8`], `I16`, `I32` or `I64`, and `v`
    /// fits it.
    fn int(&mut self, op: Op, v: i64) -> Result<(), Self::Error>;
    /// A `float`, bits as on the wire (any NaN payload is passed through).
    fn f32(&mut self, v: f32) -> Result<(), Self::Error>;
    /// A `double`, bits as on the wire.
    fn f64(&mut self, v: f64) -> Result<(), Self::Error>;
    /// A `string` (validated UTF-8), borrowed from the message.
    fn string(&mut self, v: &str) -> Result<(), Self::Error>;
    /// A byte string, borrowed from the message.
    fn bytes(&mut self, v: &[u8]) -> Result<(), Self::Error>;
    /// An enum value, by its index (< the enum's count).
    fn enum_index(&mut self, i: u8) -> Result<(), Self::Error>;
    /// The start of an object.
    fn enter_object(&mut self) -> Result<(), Self::Error>;
    /// Field `index` of the object or union branch being decoded, in
    /// declared order, for every field. `optional` is the field's optional
    /// flag; `present` is always true for a required field. A present
    /// field's value follows.
    fn field(&mut self, index: u8, optional: bool, present: bool) -> Result<(), Self::Error>;
    /// The start of an array of `len` items.
    fn enter_array(&mut self, len: u32) -> Result<(), Self::Error>;
    /// The start of a union value: branch `i` (< the union's count).
    fn variant(&mut self, i: u8) -> Result<(), Self::Error>;
    /// The end of the innermost object, array or union.
    fn leave(&mut self) -> Result<(), Self::Error>;
}

/// Decodes `msg` with `desc`, pushing the value into `sink`.
///
/// Refuses malformed bytes with [`Error::Truncated`], [`Error::Trailing`] or
/// [`Error::Invalid`] (SPEC §7), converted to the sink's error type; a sink
/// error is returned as is.
pub fn decode<S: Sink + ?Sized>(desc: &Descriptor<'_>, msg: &[u8], sink: &mut S) -> Result<(), S::Error> {
    let mut r = Reader { b: msg, p: 0, bv: 0, used: 8 };
    r.node(desc.root(), sink)?;
    if r.bv >> r.used != 0 {
        return Err(Error::Invalid.into());
    }
    if r.p != msg.len() {
        return Err(Error::Trailing.into());
    }
    Ok(())
}

struct Reader<'m> {
    b: &'m [u8],
    p: usize,
    /// the current bit byte
    bv: u32,
    /// bits used in it; 8 = no room (also before the first)
    used: u32,
}

impl<'m> Reader<'m> {
    #[inline]
    fn bits(&mut self, w: u32) -> Result<u32, Error> {
        if self.used + w > 8 {
            if self.bv >> self.used != 0 {
                return Err(Error::Invalid);
            }
            self.bv = *self.b.get(self.p).ok_or(Error::Truncated)? as u32;
            self.p += 1;
            self.used = 0;
        }
        let v = (self.bv >> self.used) & ((1 << w) - 1);
        self.used += w;
        Ok(v)
    }

    #[inline]
    fn take(&mut self, n: usize) -> Result<&'m [u8], Error> {
        if n > self.b.len() - self.p {
            return Err(Error::Truncated);
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }

    #[inline]
    fn byte(&mut self) -> Result<u8, Error> {
        let v = *self.b.get(self.p).ok_or(Error::Truncated)?;
        self.p += 1;
        Ok(v)
    }

    /// An integer varint for `op`, checked against its width (for a signed
    /// op, the zigzag value).
    #[inline]
    fn varint(&mut self, op: Op) -> Result<u64, Error> {
        let v = varint::read(self.b, &mut self.p, op.varint_bytes())?;
        let max = match op {
            Op::U16 | Op::I16 => u16::MAX as u64,
            Op::U32 | Op::I32 => u32::MAX as u64,
            _ => u64::MAX,
        };
        if v > max {
            return Err(Error::Invalid);
        }
        Ok(v)
    }

    /// A string, byte-string or array length (< 2^32).
    #[inline]
    fn length(&mut self) -> Result<u32, Error> {
        let v = varint::read(self.b, &mut self.p, 5)?;
        u32::try_from(v).map_err(|_| Error::Invalid)
    }

    /// An enum or union index, checked against the count.
    #[inline]
    fn index(&mut self, n: &Node<'_>) -> Result<u8, Error> {
        let i = if n.bits() == 0 { 0 } else { self.bits(n.bits())? };
        if i as usize >= n.len() {
            return Err(Error::Invalid);
        }
        Ok(i as u8)
    }

    fn node<S: Sink + ?Sized>(&mut self, n: Node<'_>, s: &mut S) -> Result<(), S::Error> {
        match n.op() {
            Op::Bool => {
                let v = self.bits(1)?;
                s.bool(v == 1)
            }
            Op::U8 => {
                let v = self.byte()?;
                s.uint(Op::U8, v as u64)
            }
            Op::I8 => {
                let v = self.byte()?;
                s.int(Op::I8, v as i8 as i64)
            }
            op @ (Op::U16 | Op::U32 | Op::U64) => {
                let v = self.varint(op)?;
                s.uint(op, v)
            }
            op @ (Op::I16 | Op::I32 | Op::I64) => {
                let z = self.varint(op)?;
                s.int(op, varint::unzigzag(z))
            }
            Op::F32 => {
                let b = self.take(4)?;
                s.f32(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            }
            Op::F64 => {
                let b = self.take(8)?;
                s.f64(f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
            }
            Op::String => {
                let len = self.length()?;
                let b = self.take(len as usize)?;
                let v = core::str::from_utf8(b).map_err(|_| Error::Invalid)?;
                s.string(v)
            }
            Op::Bytes => {
                let len = self.length()?;
                let b = self.take(len as usize)?;
                s.bytes(b)
            }
            Op::Enum => {
                let i = self.index(&n)?;
                s.enum_index(i)
            }
            Op::Object => {
                s.enter_object()?;
                self.fields(n, s)?;
                s.leave()
            }
            Op::Array => {
                let len = self.length()?;
                s.enter_array(len)?;
                let item = n.item();
                for _ in 0..len {
                    self.node(item, s)?;
                }
                s.leave()
            }
            Op::Union => {
                let i = self.index(&n)?;
                s.variant(i)?;
                if let Some(b) = n.branch(i as usize) {
                    self.fields(b, s)?;
                }
                s.leave()
            }
        }
    }

    fn fields<S: Sink + ?Sized>(&mut self, obj: Node<'_>, s: &mut S) -> Result<(), S::Error> {
        for (i, f) in obj.children().enumerate() {
            let optional = f.is_optional();
            let present = !optional || self.bits(1)? == 1;
            s.field(i as u8, optional, present)?;
            if present {
                self.node(f, s)?;
            }
        }
        Ok(())
    }
}
