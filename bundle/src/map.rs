//! Node maps and enums (the plan's A4), and the bitwire [`Source`] and
//! [`Sink`] that use them to encode a published value straight from the
//! VM's output leaves and decode an event straight into its input leaves.
//!
//! A map (`view.map`, `event.map`) is `node_count u32`, then one entry per
//! descriptor node in pre-order, `{leaf u16, span u16, aux u16}`: the node's
//! leaf (relative to the value's first; item-relative inside an array;
//! [`NONE`] for none), its subtree's entries (itself included), and for an
//! enum its first string in `enums.bin`. bitwire's walk is positional, so
//! a cursor follows it: a stack of frames `{next}`; a field takes the entry
//! at `next` and moves `next` past its subtree; entering an object or array
//! pushes the entry after its own; a record's variant `k` skips `k`
//! subtrees.
//!
//! `enums.bin` is `count u16, 0 u16`, `count × {off u32, len u16, 0 u16}`
//! (`off` from the section's start), then the strings' bytes.

use bitwire::{Error, Op, Sink, Source};
use vm::{ListInput, ListRef, Value};

pub use crate::layout::NONE;

/// Deepest nesting the cursor follows (bitwire's limit).
const DEPTH: usize = bitwire::MAX_DEPTH as usize;

/// Why bytes aren't a map or an enum table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    /// The length isn't what the count says.
    Size,
    /// A string's bytes are outside the table, not UTF-8, or a reserved field isn't zero.
    Strings,
}

/// One node's entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub leaf: u16,
    pub span: u16,
    pub aux: u16,
}

/// A parsed `*.map` section (sized, not yet checked against a descriptor:
/// [`crate::validate`] does that).
#[derive(Clone, Copy, Debug)]
pub struct Map<'a> {
    entries: &'a [u8],
}

impl<'a> Map<'a> {
    pub fn parse(b: &'a [u8]) -> Result<Map<'a>, MapError> {
        let count = b.get(..4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).ok_or(MapError::Size)?;
        if !(b.len() - 4).is_multiple_of(6) || ((b.len() - 4) / 6) as u64 != count as u64 {
            return Err(MapError::Size);
        }
        Ok(Map { entries: &b[4..] })
    }

    /// The number of entries (the descriptor's nodes).
    pub fn len(&self) -> usize {
        self.entries.len() / 6
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entry `i`, if there is one.
    pub fn get(&self, i: usize) -> Option<Entry> {
        let e = self.entries.get(6 * i..6 * i + 6)?;
        let w = |k: usize| u16::from_le_bytes([e[k], e[k + 1]]);
        Some(Entry { leaf: w(0), span: w(2), aux: w(4) })
    }
}

/// A parsed `enums.bin`: every string is in bounds and UTF-8.
#[derive(Clone, Copy, Debug)]
pub struct Enums<'a> {
    b: &'a [u8],
    count: u16,
}

impl<'a> Enums<'a> {
    pub fn parse(b: &'a [u8]) -> Result<Enums<'a>, MapError> {
        if b.len() < 4 || b[2..4] != [0, 0] {
            return Err(MapError::Size);
        }
        let count = u16::from_le_bytes([b[0], b[1]]);
        if b.len() < 4 + 8 * count as usize {
            return Err(MapError::Size);
        }
        let e = Enums { b, count };
        for i in 0..count as usize {
            let r = &b[4 + 8 * i..12 + 8 * i];
            let off = u32::from_le_bytes([r[0], r[1], r[2], r[3]]) as usize;
            let len = u16::from_le_bytes([r[4], r[5]]) as usize;
            let s = b.get(off..off + len).ok_or(MapError::Strings)?;
            if r[6..8] != [0, 0] || core::str::from_utf8(s).is_err() {
                return Err(MapError::Strings);
            }
        }
        Ok(e)
    }

    pub fn len(&self) -> usize {
        self.count as usize
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// String `i`.
    pub fn get(&self, i: usize) -> Option<&'a [u8]> {
        if i >= self.count as usize {
            return None;
        }
        let r = &self.b[4 + 8 * i..12 + 8 * i];
        let off = u32::from_le_bytes([r[0], r[1], r[2], r[3]]) as usize;
        let len = u16::from_le_bytes([r[4], r[5]]) as usize;
        Some(&self.b[off..off + len])
    }
}

// ---- the cursor --------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Frame {
    /// The next field's (or the next variant's) entry.
    next: u32,
    /// Just past this object's or array's subtree.
    end: u32,
    /// An array: its list, and the item being walked (`item - 1`).
    list: Option<ListRef>,
    item: u32,
}

struct Cursor<'a> {
    map: Map<'a>,
    frames: [Frame; DEPTH],
    depth: usize,
    /// The entry of the value the walk asks for next.
    cur: u32,
}

impl<'a> Cursor<'a> {
    fn new(map: Map<'a>) -> Cursor<'a> {
        Cursor { map, frames: [Frame::default(); DEPTH], depth: 0, cur: 0 }
    }

    fn entry(&self, i: u32) -> Result<Entry, Error> {
        self.map.get(i as usize).ok_or(Error::Shape)
    }

    fn top(&mut self) -> Result<&mut Frame, Error> {
        match self.depth {
            0 => Err(Error::Shape),
            d => Ok(&mut self.frames[d - 1]),
        }
    }

    /// The entry of the value now asked for: the current one, or in an
    /// array the item's (the next item).
    fn at(&mut self) -> Result<(u32, Entry), Error> {
        if self.depth > 0 && self.frames[self.depth - 1].list.is_some() {
            let f = &mut self.frames[self.depth - 1];
            f.item += 1;
            self.cur = f.next;
        }
        Ok((self.cur, self.entry(self.cur)?))
    }

    /// The next field of the current object.
    fn field(&mut self) -> Result<Entry, Error> {
        let f = self.top()?;
        let cur = f.next;
        if cur >= f.end {
            return Err(Error::Shape);
        }
        let e = self.entry(cur)?;
        let f = self.top()?;
        f.next = cur + e.span.max(1) as u32;
        self.cur = cur;
        Ok(e)
    }

    /// Variant `k` of the current record (its plain fields done).
    fn variant(&mut self, k: u32) -> Result<Entry, Error> {
        let (mut i, end) = {
            let f = self.top()?;
            (f.next, f.end)
        };
        for _ in 0..k {
            i += self.entry(i)?.span.max(1) as u32;
        }
        if i >= end {
            return Err(Error::Shape);
        }
        self.cur = i;
        self.entry(i)
    }

    /// The variants of the current record: (entry index, entry), in order.
    fn variants(&self) -> impl Iterator<Item = (u32, Entry)> + '_ {
        let f = self.frames[self.depth.saturating_sub(1)];
        let mut i = f.next;
        core::iter::from_fn(move || {
            if self.depth == 0 || i >= f.end {
                return None;
            }
            let e = self.map.get(i as usize)?;
            let at = i;
            i += e.span.max(1) as u32;
            Some((at, e))
        })
    }

    fn push(&mut self, at: u32, e: Entry, list: Option<ListRef>) -> Result<(), Error> {
        if self.depth == DEPTH {
            return Err(Error::Shape);
        }
        self.frames[self.depth] = Frame { next: at + 1, end: at + e.span as u32, list, item: 0 };
        self.depth += 1;
        Ok(())
    }

    fn leave(&mut self) -> Result<(), Error> {
        self.top()?;
        self.depth -= 1;
        Ok(())
    }

    /// The innermost array's list and the item being walked.
    fn item(&self) -> Option<(ListRef, u32)> {
        self.frames[..self.depth].iter().rev().find_map(|f| f.list.map(|l| (l, f.item.saturating_sub(1))))
    }
}

// ---- encoding the published value ---------------------------------------------

/// The published value (the view, or the state), read in place from the
/// VM's output leaves for `bitwire::encode` with the view's descriptor.
///
/// `values` are the published value's leaves (the output from `pub_base`,
/// `pub_leaves` of them). A list in it is read through its handle: `locals`
/// for a list built by the program, `lists` for an input list (the same
/// buffers `vm::run` was given). `'a` is the bundle's (the map and the
/// enums), `'v` the buffers', `'l` the input lists' values'.
pub struct ViewSource<'a, 'v, 'l: 'v> {
    cursor: Cursor<'a>,
    enums: Option<Enums<'a>>,
    values: &'v [Value<'v>],
    locals: &'v [Value<'v>],
    lists: &'v [Option<&'v dyn ListInput<'l>>],
}

impl<'a, 'v, 'l: 'v> ViewSource<'a, 'v, 'l> {
    pub fn new(
        map: Map<'a>,
        enums: Option<Enums<'a>>,
        values: &'v [Value<'v>],
        locals: &'v [Value<'v>],
        lists: &'v [Option<&'v dyn ListInput<'l>>],
    ) -> ViewSource<'a, 'v, 'l> {
        ViewSource { cursor: Cursor::new(map), enums, values, locals, lists }
    }

    /// Leaf `leaf` of the value, or of the item being walked.
    fn read(&self, leaf: u16) -> Result<Value<'v>, Error> {
        if leaf == NONE {
            return Err(Error::Shape);
        }
        match self.cursor.item() {
            None => self.values.get(leaf as usize).copied().ok_or(Error::Shape),
            Some((ListRef::Local { base, len, stride }, i)) => {
                if i >= len as u32 || leaf >= stride {
                    return Err(Error::Shape);
                }
                self.locals.get(base as usize + i as usize * stride as usize + leaf as usize).copied().ok_or(Error::Shape)
            }
            Some((ListRef::Input(id), i)) => {
                let l = self.lists.get(id as usize).copied().flatten().ok_or(Error::Shape)?;
                if i >= l.len() { Err(Error::Shape) } else { Ok(l.read(i, leaf)) }
            }
        }
    }

    fn value(&mut self) -> Result<Value<'v>, Error> {
        let (_, e) = self.cursor.at()?;
        self.read(e.leaf)
    }

    fn list_len(&self, l: ListRef) -> Result<u32, Error> {
        match l {
            ListRef::Local { len, .. } => Ok(len as u32),
            ListRef::Input(id) => Ok(self.lists.get(id as usize).copied().flatten().ok_or(Error::Shape)?.len()),
        }
    }
}

impl Source for ViewSource<'_, '_, '_> {
    type Error = Error;

    fn bool(&mut self) -> Result<bool, Error> {
        match self.value()? {
            Value::Bool(b) => Ok(b),
            _ => Err(Error::Shape),
        }
    }

    fn uint(&mut self, _op: Op) -> Result<u64, Error> {
        match self.value()? {
            Value::Int(i) => u64::try_from(i).map_err(|_| Error::Range),
            _ => Err(Error::Shape),
        }
    }

    fn int(&mut self, _op: Op) -> Result<i64, Error> {
        match self.value()? {
            Value::Int(i) => Ok(i),
            _ => Err(Error::Shape),
        }
    }

    fn f32(&mut self) -> Result<f32, Error> {
        self.f64().map(|f| f as f32)
    }

    fn f64(&mut self) -> Result<f64, Error> {
        match self.value()? {
            Value::Float(f) => Ok(f),
            Value::Int(i) => Ok(i as f64),
            _ => Err(Error::Shape),
        }
    }

    fn string(&mut self) -> Result<&str, Error> {
        match self.value()? {
            Value::Str(s) => core::str::from_utf8(s).map_err(|_| Error::Shape),
            _ => Err(Error::Shape),
        }
    }

    fn bytes(&mut self) -> Result<&[u8], Error> {
        Err(Error::Shape)
    }

    /// The value's position among its enum's strings (from the entry's
    /// first): the walk refuses one past the enum's count.
    fn enum_index(&mut self) -> Result<u32, Error> {
        let (_, e) = self.cursor.at()?;
        let Value::Str(s) = self.read(e.leaf)? else { return Err(Error::Shape) };
        let enums = self.enums.as_ref().ok_or(Error::Shape)?;
        Ok((e.aux as usize..enums.len()).position(|i| enums.get(i) == Some(s)).map_or(u32::MAX, |k| k as u32))
    }

    fn enter_object(&mut self) -> Result<(), Error> {
        let (at, e) = self.cursor.at()?;
        self.cursor.push(at, e, None)
    }

    fn field(&mut self, _index: u8, optional: bool) -> Result<bool, Error> {
        let e = self.cursor.field()?;
        if !optional {
            return Ok(true);
        }
        Ok(self.read(e.leaf)? != Value::Nil)
    }

    fn enter_array(&mut self) -> Result<u32, Error> {
        let (at, e) = self.cursor.at()?;
        let Value::List(l) = self.read(e.leaf)? else { return Err(Error::Shape) };
        let len = self.list_len(l)?;
        self.cursor.push(at, e, Some(l))?;
        Ok(len)
    }

    /// The variant whose presence leaf isn't nil (the VM's output checks
    /// hold exactly one).
    fn variant(&mut self) -> Result<u32, Error> {
        let mut found = None;
        for (k, (_, e)) in self.cursor.variants().enumerate() {
            if self.read(e.leaf)? != Value::Nil {
                if found.is_some() {
                    return Err(Error::Shape);
                }
                found = Some(k as u32);
            }
        }
        let k = found.ok_or(Error::Shape)?;
        self.cursor.variant(k)?;
        Ok(k)
    }

    fn leave(&mut self) -> Result<(), Error> {
        self.cursor.leave()
    }
}

// ---- decoding an event --------------------------------------------------------

/// An author's event, decoded by `bitwire::decode` with the event's
/// descriptor straight into the event's input leaves (`values`: the inputs
/// from `in_event`, `event_leaves` of them), which it sets to nil first. A
/// variant and an optional record set their presence leaf; an enum is the
/// string in `enums.bin`. Events have no lists and no free strings (refused
/// at install).
pub struct EventSink<'a, 'v> {
    cursor: Cursor<'a>,
    enums: Option<Enums<'a>>,
    values: &'v mut [Value<'a>],
}

impl<'a, 'v> EventSink<'a, 'v> {
    pub fn new(map: Map<'a>, enums: Option<Enums<'a>>, values: &'v mut [Value<'a>]) -> EventSink<'a, 'v> {
        values.fill(Value::Nil);
        EventSink { cursor: Cursor::new(map), enums, values }
    }

    fn set(&mut self, v: Value<'a>) -> Result<(), Error> {
        let (_, e) = self.cursor.at()?;
        *self.values.get_mut(e.leaf as usize).ok_or(Error::Invalid)? = v;
        Ok(())
    }
}

impl<'a> Sink for EventSink<'a, '_> {
    type Error = Error;

    fn bool(&mut self, v: bool) -> Result<(), Error> {
        self.set(Value::Bool(v))
    }

    fn uint(&mut self, _op: Op, v: u64) -> Result<(), Error> {
        self.set(Value::Int(i64::try_from(v).map_err(|_| Error::Invalid)?))
    }

    fn int(&mut self, _op: Op, v: i64) -> Result<(), Error> {
        self.set(Value::Int(v))
    }

    fn f32(&mut self, v: f32) -> Result<(), Error> {
        self.set(Value::Float(v as f64))
    }

    fn f64(&mut self, v: f64) -> Result<(), Error> {
        self.set(Value::Float(v))
    }

    fn string(&mut self, _v: &str) -> Result<(), Error> {
        Err(Error::Invalid)
    }

    fn bytes(&mut self, _v: &[u8]) -> Result<(), Error> {
        Err(Error::Invalid)
    }

    fn enum_index(&mut self, i: u8) -> Result<(), Error> {
        let (_, e) = self.cursor.at()?;
        let s = self.enums.as_ref().and_then(|t| t.get(e.aux as usize + i as usize)).ok_or(Error::Invalid)?;
        *self.values.get_mut(e.leaf as usize).ok_or(Error::Invalid)? = Value::Str(s);
        Ok(())
    }

    fn enter_object(&mut self) -> Result<(), Error> {
        let (at, e) = self.cursor.at()?;
        if e.leaf != NONE {
            *self.values.get_mut(e.leaf as usize).ok_or(Error::Invalid)? = Value::Bool(true);
        }
        self.cursor.push(at, e, None)
    }

    fn field(&mut self, _index: u8, _optional: bool, _present: bool) -> Result<(), Error> {
        self.cursor.field().map(|_| ())
    }

    fn enter_array(&mut self, _len: u32) -> Result<(), Error> {
        Err(Error::Invalid)
    }

    fn variant(&mut self, i: u8) -> Result<(), Error> {
        self.cursor.variant(i as u32).map(|_| ())
    }

    fn leave(&mut self) -> Result<(), Error> {
        self.cursor.leave()
    }
}
