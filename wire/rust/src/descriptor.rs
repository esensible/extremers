//! Descriptor loading and validation (SPEC §6), and a read-only view of the
//! validated node table.
//!
//! [`Descriptor::load`] runs every check of SPEC §6.3 once. After that the
//! node bytes are trusted: [`Node`] parses a node's header on demand with
//! no further checks, and the walk never needs to look at the names.

use crate::{Error, Op, varint};

/// The descriptor format version this crate reads.
pub const FORMAT_VERSION: u8 = 1;
/// Deepest nesting of nodes (root = 1).
pub const MAX_DEPTH: u32 = 32;
/// Most fields in an object.
pub const MAX_FIELDS: u32 = 256;

const OPTIONAL: u8 = 0x80;
const PROTO: &str = "__proto__";

/// A loaded, validated descriptor. Borrows the descriptor bytes; `Copy`.
#[derive(Clone, Copy, Debug)]
pub struct Descriptor<'a> {
    bytes: &'a [u8],
    node_count: u32,
    /// Offset of the root node.
    root: usize,
    /// Offset of the names section.
    names: usize,
}

impl<'a> Descriptor<'a> {
    /// Validates `bytes` as a descriptor (every check of SPEC §6.3) and
    /// returns it, or [`Error::Descriptor`].
    pub fn load(bytes: &'a [u8]) -> Result<Self, Error> {
        let mut l = Loader { b: bytes, p: 0, count: 0, names: 0 };
        if l.u8()? != FORMAT_VERSION {
            return Err(Error::Descriptor);
        }
        l.p = 1 + 8; // the hash
        if l.p > bytes.len() {
            return Err(Error::Descriptor);
        }
        let node_count = l.varint()?;
        let root = l.p;
        l.node(1, false)?;
        if l.count != node_count as u64 {
            return Err(Error::Descriptor);
        }
        let names = l.p;
        for _ in 0..l.names {
            let len = l.varint()? as usize;
            if len > bytes.len() - l.p {
                return Err(Error::Descriptor);
            }
            if core::str::from_utf8(&bytes[l.p..l.p + len]).is_err() {
                return Err(Error::Descriptor);
            }
            l.p += len;
        }
        if l.p != bytes.len() {
            return Err(Error::Descriptor);
        }
        let d = Descriptor { bytes, node_count, root, names };
        let mut cursor = d.names();
        check_names(d.root(), &mut cursor)?;
        Ok(d)
    }

    /// The descriptor bytes.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The format version (always [`FORMAT_VERSION`]).
    pub fn version(&self) -> u8 {
        self.bytes[0]
    }

    /// The 8-byte schema hash (an identity, never checked against content).
    pub fn hash(&self) -> [u8; 8] {
        let mut h = [0; 8];
        h.copy_from_slice(&self.bytes[1..9]);
        h
    }

    /// The number of nodes.
    pub fn node_count(&self) -> u32 {
        self.node_count
    }

    /// The root node.
    pub fn root(&self) -> Node<'a> {
        Node::at(self.bytes, self.root)
    }

    /// Every name, in the order the nodes consume them (SPEC §6.2): an
    /// enum's values; an object's field names (then its children's); a
    /// union's tag property name and tag values (then each branch's field
    /// names and children's). Only hosts that build named values need them.
    pub fn names(&self) -> Names<'a> {
        Names { b: self.bytes, p: self.names }
    }
}

// ---- load-time validation -------------------------------------------------

struct Loader<'a> {
    b: &'a [u8],
    p: usize,
    count: u64,
    names: u64,
}

impl Loader<'_> {
    fn u8(&mut self) -> Result<u8, Error> {
        let v = *self.b.get(self.p).ok_or(Error::Descriptor)?;
        self.p += 1;
        Ok(v)
    }

    /// Descriptor varint: <= 5 bytes, canonical, <= 2^32 − 1.
    fn varint(&mut self) -> Result<u32, Error> {
        let v = varint::read(self.b, &mut self.p, 5).map_err(|_| Error::Descriptor)?;
        u32::try_from(v).map_err(|_| Error::Descriptor)
    }

    /// Parses and checks one node and its children; returns its op, its
    /// optional flag and whether it is empty (encodes to nothing).
    fn node(&mut self, depth: u32, field: bool) -> Result<(Op, bool, bool), Error> {
        if depth > MAX_DEPTH {
            return Err(Error::Descriptor);
        }
        let byte = self.u8()?;
        let optional = byte & OPTIONAL != 0;
        if optional && !field {
            return Err(Error::Descriptor);
        }
        let op = Op::from_u8(byte & !OPTIONAL).ok_or(Error::Descriptor)?;
        self.count += 1;
        let empty = match op {
            Op::Enum => {
                let n = self.u8()? as u64 + 1;
                self.names += n;
                n == 1
            }
            Op::Object => {
                let fields = self.varint()?;
                if fields > MAX_FIELDS {
                    return Err(Error::Descriptor);
                }
                let size = self.varint()? as usize;
                self.names += fields as u64;
                let start = self.p;
                let mut empty = true;
                for _ in 0..fields {
                    let (_, opt, e) = self.node(depth + 1, true)?;
                    empty &= !opt && e;
                }
                if self.p - start != size {
                    return Err(Error::Descriptor);
                }
                empty
            }
            Op::Array => {
                let size = self.varint()? as usize;
                let start = self.p;
                let (_, _, item_empty) = self.node(depth + 1, false)?;
                if item_empty || self.p - start != size {
                    return Err(Error::Descriptor);
                }
                false
            }
            Op::Union => {
                let n = self.u8()? as u32 + 1;
                let size = self.varint()? as usize;
                self.names += 1 + n as u64;
                let start = self.p;
                let mut first_empty = false;
                for i in 0..n {
                    let (bop, _, e) = self.node(depth + 1, false)?;
                    if bop != Op::Object {
                        return Err(Error::Descriptor);
                    }
                    if i == 0 {
                        first_empty = e;
                    }
                }
                if self.p - start != size {
                    return Err(Error::Descriptor);
                }
                n == 1 && first_empty
            }
            _ => false,
        };
        Ok((op, optional, empty))
    }
}

/// SPEC §6.3 rule 11, walking the nodes and the names together.
fn check_names(n: Node<'_>, names: &mut Names<'_>) -> Result<(), Error> {
    match n.op {
        Op::Enum => distinct(names, n.len(), None, false),
        Op::Object => {
            distinct(names, n.len(), None, true)?;
            n.children().try_for_each(|f| check_names(f, names))
        }
        Op::Array => check_names(n.item(), names),
        Op::Union => {
            let tag = names.next().unwrap_or("");
            distinct(names, n.len(), None, false)?;
            for b in n.children() {
                distinct(names, b.len(), Some(tag), true)?;
                b.children().try_for_each(|f| check_names(f, names))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Consumes the next `count` names and checks they are distinct from each
/// other and from `extra`; with `fields`, also that none of them (nor
/// `extra`) is `__proto__`.
fn distinct(names: &mut Names<'_>, count: usize, extra: Option<&str>, fields: bool) -> Result<(), Error> {
    if fields && extra == Some(PROTO) {
        return Err(Error::Descriptor);
    }
    for i in 0..count {
        let a = names.next().unwrap_or("");
        if (fields && a == PROTO) || extra == Some(a) {
            return Err(Error::Descriptor);
        }
        let mut rest = names.clone();
        for _ in i + 1..count {
            if rest.next() == Some(a) {
                return Err(Error::Descriptor);
            }
        }
    }
    Ok(())
}

// ---- trusted views ------------------------------------------------------------

/// The names section, in node order. An iterator of `&str`.
#[derive(Clone, Debug)]
pub struct Names<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Iterator for Names<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        if self.p >= self.b.len() {
            return None;
        }
        let len = varint::read(self.b, &mut self.p, 5).ok()? as usize;
        let s = &self.b[self.p..self.p + len];
        self.p += len;
        core::str::from_utf8(s).ok()
    }
}

/// One node of a validated descriptor: its op and, for the compound ops,
/// where its children are. `Copy`; parsed from the descriptor on demand.
#[derive(Clone, Copy, Debug)]
pub struct Node<'a> {
    b: &'a [u8],
    op: Op,
    optional: bool,
    /// enum values, object fields, union branches; 1 for an array
    len: u16,
    /// index width in bits (enum, union)
    bits: u8,
    /// offset of the first child
    body: usize,
    /// offset just past the node
    end: usize,
}

/// Trusted varint read of a validated descriptor.
fn tv(b: &[u8], p: &mut usize) -> usize {
    varint::read(b, p, 5).unwrap_or(0) as usize
}

fn bit_length(x: u32) -> u8 {
    (32 - x.leading_zeros()) as u8
}

impl<'a> Node<'a> {
    fn at(b: &'a [u8], at: usize) -> Self {
        let byte = b[at];
        let op = Op::from_u8(byte & !OPTIONAL).unwrap_or(Op::Bool);
        let mut p = at + 1;
        let (len, bits, end);
        match op {
            Op::Enum | Op::Union => {
                let n = b[p] as u32 + 1;
                p += 1;
                len = n as u16;
                bits = bit_length(n - 1);
                end = if op == Op::Union { let size = tv(b, &mut p); p + size } else { p };
            }
            Op::Object => {
                len = tv(b, &mut p) as u16;
                bits = 0;
                let size = tv(b, &mut p);
                end = p + size;
            }
            Op::Array => {
                len = 1;
                bits = 0;
                let size = tv(b, &mut p);
                end = p + size;
            }
            _ => {
                len = 0;
                bits = 0;
                end = p;
            }
        }
        Node { b, op, optional: byte & OPTIONAL != 0, len, bits, body: p, end }
    }

    /// The op.
    #[inline]
    pub fn op(&self) -> Op {
        self.op
    }

    /// Whether this is an optional object field (it has a presence bit).
    #[inline]
    pub fn is_optional(&self) -> bool {
        self.optional
    }

    /// The number of enum values, object fields or union branches; 1 for an
    /// array (its item); 0 for the other ops.
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether [`Node::len`] is 0.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// For an enum or union: the index width in bits, `bitLength(n − 1)`
    /// (0 for a single value or branch).
    #[inline]
    pub fn bits(&self) -> u32 {
        self.bits as u32
    }

    /// The children: an object's fields, an array's item, a union's
    /// branches (each an object); none for the other ops.
    #[inline]
    pub fn children(&self) -> Children<'a> {
        let n = match self.op {
            Op::Object | Op::Array | Op::Union => self.len,
            _ => 0,
        };
        Children { b: self.b, p: self.body, left: n }
    }

    /// An array's item node. Only meaningful for [`Op::Array`].
    #[inline]
    pub fn item(&self) -> Node<'a> {
        Node::at(self.b, self.body)
    }

    /// A union's branch `i` (an object node), if `i` < [`Node::len`].
    pub fn branch(&self, i: usize) -> Option<Node<'a>> {
        if self.op != Op::Union {
            return None;
        }
        self.children().nth(i)
    }
}

/// Iterator over a node's children; skips each subtree by its size.
#[derive(Clone, Debug)]
pub struct Children<'a> {
    b: &'a [u8],
    p: usize,
    left: u16,
}

impl<'a> Iterator for Children<'a> {
    type Item = Node<'a>;

    #[inline]
    fn next(&mut self) -> Option<Node<'a>> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        let n = Node::at(self.b, self.p);
        self.p = n.end;
        Some(n)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left as usize, Some(self.left as usize))
    }
}

impl ExactSizeIterator for Children<'_> {}
