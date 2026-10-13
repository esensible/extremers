//! The bytecode format, shared by the compiler (which writes it) and the VM
//! (which loads and runs it). Nothing here evaluates anything: it is the
//! container layout, the instruction set and its encoding, and the checks a
//! program must pass before a VM will run it.
//!
//! `no_std`, no allocation. Validation needs a scratch buffer the size of the
//! code, which the caller supplies (a host can lend it the arena at load time).

#![no_std]

pub const MAGIC: [u8; 4] = *b"ENG1";
pub const VERSION: u16 = 1;

// ---------------------------------------------------------------------------
// Scalar kinds and the checks attached to inputs and the output
//
// Values are flattened into leaves (spec §7): a record is its fields' leaves
// in field-name order, and an optional record adds a presence leaf in front
// (true, or nil when the record is absent, in which case all its leaves are
// nil). Inputs and the output are lists of leaves, each with a Check.

/// The kind of a scalar value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool = 1,
    Int = 2,
    Float = 3,
    Str = 4,
    /// A list position in an input record: the leaf holds nothing (the
    /// host passes nil); the list itself is read with `PushInputList`.
    List = 5,
}

impl Kind {
    fn from_u8(b: u8) -> Option<Kind> {
        Some(match b {
            1 => Kind::Bool,
            2 => Kind::Int,
            3 => Kind::Float,
            4 => Kind::Str,
            5 => Kind::List,
            _ => return None,
        })
    }
}

/// What a value in one position (an input, or the output) must be.
///
/// - `Int`: `lo..=hi`.
/// - `Str`: at most `max_len` bytes; if `enum_count > 0`, one of the
///   `enum_count` string records starting at `enum_at` in the constant pool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Check {
    pub kind: Kind,
    pub nullable: bool,
    pub lo: i64,
    pub hi: i64,
    pub max_len: u16,
    pub enum_count: u16,
    pub enum_at: u32,
}

impl Check {
    pub const fn new(kind: Kind) -> Check {
        Check { kind, nullable: false, lo: i64::MIN, hi: i64::MAX, max_len: 0, enum_count: 0, enum_at: 0 }
    }
}

/// No guard: a group check applies always (see [`Group`]).
pub const NO_GUARD: u16 = u16::MAX;

/// A variant group's check (a schema `oneOf`): of the presence leaves
/// `members`, exactly one is non-nil. A group inside an optional record has
/// that record's presence leaf as its `guard`, and isn't checked when the
/// guard is nil (the record is absent, so all its leaves are nil).
#[derive(Clone, Copy, Debug)]
pub struct Group<'a> {
    pub guard: u16,
    members: &'a [u8],
}

impl<'a> Group<'a> {
    /// The members' presence leaves.
    #[inline]
    pub fn members(&self) -> impl Iterator<Item = u16> + 'a {
        self.members.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }
}

/// A table of group checks, read in order.
#[derive(Clone)]
pub struct Groups<'a> {
    r: Reader<'a>,
    left: u16,
}

impl<'a> Iterator for Groups<'a> {
    type Item = Group<'a>;

    #[inline]
    fn next(&mut self) -> Option<Group<'a>> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        let guard = self.r.u16().expect("parsed");
        let n = self.r.u16().expect("parsed") as usize;
        Some(Group { guard, members: self.r.take(2 * n).expect("parsed") })
    }
}

// ---------------------------------------------------------------------------
// Instructions

/// Builtins the VM implements itself (expr's math builtins).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    Abs = 1,
    Int = 2,
    Float = 3,
    Round = 4,
    Floor = 5,
    Ceil = 6,
    Min = 7,
    Max = 8,
}

impl Builtin {
    fn from_u8(b: u8) -> Option<Builtin> {
        Some(match b {
            1 => Builtin::Abs,
            2 => Builtin::Int,
            3 => Builtin::Float,
            4 => Builtin::Round,
            5 => Builtin::Floor,
            6 => Builtin::Ceil,
            7 => Builtin::Min,
            8 => Builtin::Max,
            _ => return None,
        })
    }

    /// Whether `argc` arguments is a valid call.
    pub fn arity_ok(self, argc: u8) -> bool {
        match self {
            Builtin::Min | Builtin::Max => argc >= 2,
            _ => argc == 1,
        }
    }
}

/// One instruction. Operands are little-endian. Jump targets are absolute
/// byte offsets into the code.
///
/// Stack effects (pops → pushes):
/// - `Push*`, `Input`, `Load`, `Dup`, `Peek`: → 1. `Peek(s)` pushes a copy
///   of stack slot `s`, counted from the bottom; `s` must be below the
///   height there (a value the compiler built in place on the stack, read
///   where it lies).
/// - `Store`, `Pop`: 1 →
/// - `Add` … `Ge`: 2 → 1. `Neg`, `Not`: 1 → 1.
/// - `Jump`: none. `JumpIfFalse`, `JumpIfNotNil`: pop the operand, then jump
///   or fall through.
/// - `Call(b, n)`: n → 1.
/// - `CallHost { import, args, results }`: pops `args` leaves, calls the
///   host builtin linked as import `import`, pushes its `results` leaves.
///   The counts must match the import's.
/// - Lists are handles (a reference to an input list, or to items laid out
///   in locals): `PushInputList(id)` → handle (nil if the host says the list
///   is absent); `MakeList { base, len, stride }` → a handle to `len` items of
///   `stride` leaves in locals from `base`; `ListLen`: handle → int;
///   `ListRead(leaf)`: handle, index → that item's leaf (`R-INDEX` out of range).
/// - A list the program builds (a `map`, `filter`, `concat`, slice or
///   `take` kept as a value) is written into a block of `cap` items of
///   `stride` leaves in locals from `base`: `ListStore { base, cap, stride }`
///   pops an item's `stride` leaves and then (on top) its index `i < cap`,
///   and writes them as item `i`; `ListMake { base, cap, stride }`: len →
///   a handle to the block's first `len ≤ cap` items. Either out of range is
///   `R-TYPE` (a compiler bug).
/// - A sorted block (`sortBy`, `sort`, `median`, a k-selection) is `cap`
///   items of `stride` leaves in locals from `base`, followed (when the key
///   isn't one of the item's leaves) by `cap` keys, one slot each.
///   `HoldInsert { base, cap, stride, mode }` pops the block's state (its
///   count, top), then the key (when kept apart), then the item's `stride`
///   leaves; inserts the item; pushes the new state. A `nil` key is `R-ARG`,
///   another non-number `R-TYPE`. See [`hold_mode`].
/// - `AssertNotNil`: pops a value; nil is `R-NIL`.
/// - `Return(n)`: pops the output's `n` leaves (the last leaf on top); the
///   stack must then be empty, and `n` must equal the output's leaf count.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    PushNil,
    PushTrue,
    PushFalse,
    PushInt(i64),
    PushFloat(f64),
    /// A string record in the constant pool, by byte offset.
    PushStr(u32),
    Input(u16),
    Load(u16),
    Store(u16),
    Pop,
    Dup,
    Peek(u16),
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Neg,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Not,
    Jump(u32),
    JumpIfFalse(u32),
    JumpIfNotNil(u32),
    Call(Builtin, u8),
    CallHost { import: u16, args: u16, results: u16 },
    PushInputList(u16),
    MakeList { base: u16, len: u16, stride: u16 },
    ListLen,
    ListRead(u16),
    ListStore { base: u16, cap: u16, stride: u16 },
    ListMake { base: u16, cap: u16, stride: u16 },
    /// Inserts an item into a sorted block, as Go's insertion sort does
    /// (see [`hold_mode`] for `mode`).
    HoldInsert { base: u16, cap: u16, stride: u16, mode: u16 },
    /// Fails the evaluation: 1 `R-NIL`, 2 `R-INDEX`, 3 `R-ARG`. Ends a path,
    /// like `Return`.
    Error(u8),
    AssertNotNil,
    Return(u16),
}

/// The most leaves a host builtin may return (the VM holds them in a fixed
/// buffer while the builtin runs).
pub const MAX_HOST_RESULTS: u16 = 16;

/// `HoldInsert`'s `mode`: descending.
pub const HOLD_DESC: u16 = 1;
/// `HoldInsert`'s `mode`: keep the last `cap` items of the sorted order
/// (else the first `cap`).
pub const HOLD_TAIL: u16 = 2;
/// `HoldInsert`'s `mode`: NaN comes before every number (Go's
/// `sort.Float64s`, for `median`); else keys compare with `<` alone, so
/// nothing passes a NaN (Go's `sort.Sort` with expr's `Less`).
pub const HOLD_TOTAL: u16 = 4;
/// A key leaf must be below this to be kept in `mode`.
pub const HOLD_KEY_MAX: u16 = 0x0FFF;

/// `HoldInsert`'s `mode`: the flags, and in bits 4–15 the item leaf that is
/// the key, plus one (0: the key is kept apart, after the items).
///
/// The insertion is Go's `insertionSort` (what `sort.Sort` runs on up to 12
/// items): the item goes in at the end and moves towards the front past
/// each item whose key it is strictly less than. With `count` items held:
/// - fewer than `cap`: it goes in, and the count grows;
/// - `cap`, keeping the first `cap`: it goes in if it moves into the block
///   (the last item drops out); one that stays past the end and that
///   nothing can pass (a NaN key) closes the block: the state becomes
///   `cap + 1`, and later items are ignored;
/// - `cap`, keeping the last `cap`: it goes in unless it would move past
///   the first item (the first item drops out).
pub const fn hold_mode(desc: bool, tail: bool, total: bool, key_leaf: Option<u16>) -> u16 {
    let key = match key_leaf {
        Some(l) => (l + 1) << 4,
        None => 0,
    };
    key | ((desc as u16) * HOLD_DESC) | ((tail as u16) * HOLD_TAIL) | ((total as u16) * HOLD_TOTAL)
}

/// The key leaf in a `HoldInsert` mode, or `None` when the key is kept apart.
pub const fn hold_key_leaf(mode: u16) -> Option<u16> {
    match mode >> 4 {
        0 => None,
        k => Some(k - 1),
    }
}

/// The longest encoded instruction (opcode + an 8-byte operand).
pub const MAX_OP_LEN: usize = 9;

mod code {
    pub const PUSH_NIL: u8 = 0x01;
    pub const PUSH_TRUE: u8 = 0x02;
    pub const PUSH_FALSE: u8 = 0x03;
    pub const PUSH_INT: u8 = 0x04;
    pub const PUSH_FLOAT: u8 = 0x05;
    pub const PUSH_STR: u8 = 0x06;
    pub const INPUT: u8 = 0x10;
    pub const LOAD: u8 = 0x11;
    pub const STORE: u8 = 0x12;
    pub const POP: u8 = 0x13;
    pub const DUP: u8 = 0x14;
    pub const PEEK: u8 = 0x15;
    pub const ADD: u8 = 0x20;
    pub const SUB: u8 = 0x21;
    pub const MUL: u8 = 0x22;
    pub const DIV: u8 = 0x23;
    pub const MOD: u8 = 0x24;
    pub const POW: u8 = 0x25;
    pub const NEG: u8 = 0x26;
    pub const EQ: u8 = 0x28;
    pub const NE: u8 = 0x29;
    pub const LT: u8 = 0x2A;
    pub const LE: u8 = 0x2B;
    pub const GT: u8 = 0x2C;
    pub const GE: u8 = 0x2D;
    pub const NOT: u8 = 0x2E;
    pub const JUMP: u8 = 0x30;
    pub const JUMP_IF_FALSE: u8 = 0x31;
    pub const JUMP_IF_NOT_NIL: u8 = 0x32;
    pub const CALL: u8 = 0x40;
    pub const ASSERT_NOT_NIL: u8 = 0x41;
    pub const CALL_HOST: u8 = 0x42;
    pub const PUSH_INPUT_LIST: u8 = 0x48;
    pub const MAKE_LIST: u8 = 0x49;
    pub const LIST_LEN: u8 = 0x4A;
    pub const LIST_READ: u8 = 0x4B;
    pub const LIST_STORE: u8 = 0x4C;
    pub const LIST_MAKE: u8 = 0x4D;
    pub const HOLD_INSERT: u8 = 0x4E;
    pub const ERROR: u8 = 0x4F;
    pub const RETURN: u8 = 0x50;
}

impl Op {
    /// Encodes into `buf`, returning the number of bytes written.
    pub fn encode(&self, buf: &mut [u8; MAX_OP_LEN]) -> usize {
        use code::*;
        fn with(buf: &mut [u8; MAX_OP_LEN], op: u8, operand: &[u8]) -> usize {
            buf[0] = op;
            buf[1..1 + operand.len()].copy_from_slice(operand);
            1 + operand.len()
        }
        match *self {
            Op::PushNil => with(buf, PUSH_NIL, &[]),
            Op::PushTrue => with(buf, PUSH_TRUE, &[]),
            Op::PushFalse => with(buf, PUSH_FALSE, &[]),
            Op::PushInt(v) => with(buf, PUSH_INT, &v.to_le_bytes()),
            Op::PushFloat(v) => with(buf, PUSH_FLOAT, &v.to_bits().to_le_bytes()),
            Op::PushStr(o) => with(buf, PUSH_STR, &o.to_le_bytes()),
            Op::Input(i) => with(buf, INPUT, &i.to_le_bytes()),
            Op::Load(i) => with(buf, LOAD, &i.to_le_bytes()),
            Op::Store(i) => with(buf, STORE, &i.to_le_bytes()),
            Op::Pop => with(buf, POP, &[]),
            Op::Dup => with(buf, DUP, &[]),
            Op::Peek(s) => with(buf, PEEK, &s.to_le_bytes()),
            Op::Add => with(buf, ADD, &[]),
            Op::Sub => with(buf, SUB, &[]),
            Op::Mul => with(buf, MUL, &[]),
            Op::Div => with(buf, DIV, &[]),
            Op::Mod => with(buf, MOD, &[]),
            Op::Pow => with(buf, POW, &[]),
            Op::Neg => with(buf, NEG, &[]),
            Op::Eq => with(buf, EQ, &[]),
            Op::Ne => with(buf, NE, &[]),
            Op::Lt => with(buf, LT, &[]),
            Op::Le => with(buf, LE, &[]),
            Op::Gt => with(buf, GT, &[]),
            Op::Ge => with(buf, GE, &[]),
            Op::Not => with(buf, NOT, &[]),
            Op::Jump(t) => with(buf, JUMP, &t.to_le_bytes()),
            Op::JumpIfFalse(t) => with(buf, JUMP_IF_FALSE, &t.to_le_bytes()),
            Op::JumpIfNotNil(t) => with(buf, JUMP_IF_NOT_NIL, &t.to_le_bytes()),
            Op::Call(b, n) => with(buf, CALL, &[b as u8, n]),
            Op::CallHost { import, args, results } => {
                let mut o = [0u8; 6];
                o[0..2].copy_from_slice(&import.to_le_bytes());
                o[2..4].copy_from_slice(&args.to_le_bytes());
                o[4..6].copy_from_slice(&results.to_le_bytes());
                with(buf, CALL_HOST, &o)
            }
            Op::PushInputList(id) => with(buf, PUSH_INPUT_LIST, &id.to_le_bytes()),
            Op::MakeList { base, len, stride } => with(buf, MAKE_LIST, &three(base, len, stride)),
            Op::ListStore { base, cap, stride } => with(buf, LIST_STORE, &three(base, cap, stride)),
            Op::ListMake { base, cap, stride } => with(buf, LIST_MAKE, &three(base, cap, stride)),
            Op::HoldInsert { base, cap, stride, mode } => {
                let mut o = [0u8; 8];
                o[0..6].copy_from_slice(&three(base, cap, stride));
                o[6..8].copy_from_slice(&mode.to_le_bytes());
                with(buf, HOLD_INSERT, &o)
            }
            Op::ListLen => with(buf, LIST_LEN, &[]),
            Op::ListRead(leaf) => with(buf, LIST_READ, &leaf.to_le_bytes()),
            Op::Error(code) => with(buf, ERROR, &[code]),
            Op::AssertNotNil => with(buf, ASSERT_NOT_NIL, &[]),
            Op::Return(n) => with(buf, RETURN, &n.to_le_bytes()),
        }
    }

    /// Decodes the instruction at `pc`, returning it and the offset of the next.
    ///
    /// Always inlined: it is the VM's dispatch, where the opcode match here
    /// and the VM's match on the `Op` fold into one.
    #[inline(always)]
    pub fn decode(code: &[u8], pc: usize) -> Result<(Op, usize), LoadError> {
        use code::*;
        let op = *code.get(pc).ok_or(LoadError::Truncated)?;
        let arg = |n: usize| code.get(pc + 1..pc + 1 + n).ok_or(LoadError::Truncated);
        let u16_at = || arg(2).map(|b| u16::from_le_bytes([b[0], b[1]]));
        let u32_at = || arg(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let u64_at = || arg(8).map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]));
        let (op, len) = match op {
            PUSH_NIL => (Op::PushNil, 0),
            PUSH_TRUE => (Op::PushTrue, 0),
            PUSH_FALSE => (Op::PushFalse, 0),
            PUSH_INT => (Op::PushInt(u64_at()? as i64), 8),
            PUSH_FLOAT => (Op::PushFloat(f64::from_bits(u64_at()?)), 8),
            PUSH_STR => (Op::PushStr(u32_at()?), 4),
            INPUT => (Op::Input(u16_at()?), 2),
            LOAD => (Op::Load(u16_at()?), 2),
            STORE => (Op::Store(u16_at()?), 2),
            POP => (Op::Pop, 0),
            DUP => (Op::Dup, 0),
            PEEK => (Op::Peek(u16_at()?), 2),
            ADD => (Op::Add, 0),
            SUB => (Op::Sub, 0),
            MUL => (Op::Mul, 0),
            DIV => (Op::Div, 0),
            MOD => (Op::Mod, 0),
            POW => (Op::Pow, 0),
            NEG => (Op::Neg, 0),
            EQ => (Op::Eq, 0),
            NE => (Op::Ne, 0),
            LT => (Op::Lt, 0),
            LE => (Op::Le, 0),
            GT => (Op::Gt, 0),
            GE => (Op::Ge, 0),
            NOT => (Op::Not, 0),
            JUMP => (Op::Jump(u32_at()?), 4),
            JUMP_IF_FALSE => (Op::JumpIfFalse(u32_at()?), 4),
            JUMP_IF_NOT_NIL => (Op::JumpIfNotNil(u32_at()?), 4),
            CALL => {
                let b = arg(2)?;
                let builtin = Builtin::from_u8(b[0]).ok_or(LoadError::BadOperand(pc))?;
                (Op::Call(builtin, b[1]), 2)
            }
            CALL_HOST => {
                let b = arg(6)?;
                let w = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
                (Op::CallHost { import: w(0), args: w(2), results: w(4) }, 6)
            }
            PUSH_INPUT_LIST => (Op::PushInputList(u16_at()?), 2),
            MAKE_LIST | LIST_STORE | LIST_MAKE => {
                let b = arg(6)?;
                let w = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
                let (base, n, stride) = (w(0), w(2), w(4));
                let op = match op {
                    MAKE_LIST => Op::MakeList { base, len: n, stride },
                    LIST_STORE => Op::ListStore { base, cap: n, stride },
                    _ => Op::ListMake { base, cap: n, stride },
                };
                (op, 6)
            }
            HOLD_INSERT => {
                let b = arg(8)?;
                let w = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
                (Op::HoldInsert { base: w(0), cap: w(2), stride: w(4), mode: w(6) }, 8)
            }
            LIST_LEN => (Op::ListLen, 0),
            LIST_READ => (Op::ListRead(u16_at()?), 2),
            ERROR => (Op::Error(arg(1)?[0]), 1),
            ASSERT_NOT_NIL => (Op::AssertNotNil, 0),
            RETURN => (Op::Return(u16_at()?), 2),
            _ => return Err(LoadError::BadOpcode(pc)),
        };
        Ok((op, pc + 1 + len))
    }

    /// (pops, pushes).
    pub fn stack_effect(&self) -> (u16, u16) {
        match *self {
            Op::PushNil | Op::PushTrue | Op::PushFalse | Op::PushInt(_) | Op::PushFloat(_) | Op::PushStr(_) => (0, 1),
            Op::Input(_) | Op::Load(_) | Op::Dup | Op::Peek(_) => (0, 1),
            Op::Store(_) | Op::Pop => (1, 0),
            Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod | Op::Pow => (2, 1),
            Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => (2, 1),
            Op::Neg | Op::Not => (1, 1),
            Op::Jump(_) => (0, 0),
            Op::JumpIfFalse(_) | Op::JumpIfNotNil(_) => (1, 0),
            Op::Call(_, n) => (n as u16, 1),
            Op::CallHost { args, results, .. } => (args, results),
            Op::PushInputList(_) | Op::MakeList { .. } => (0, 1),
            Op::ListLen => (1, 1),
            Op::ListRead(_) => (2, 1),
            Op::ListStore { stride, .. } => (stride + 1, 0),
            Op::ListMake { .. } => (1, 1),
            Op::HoldInsert { stride, mode, .. } => (stride + hold_key_leaf(mode).is_none() as u16 + 1, 1),
            Op::Error(_) => (0, 0),
            Op::AssertNotNil => (1, 0),
            Op::Return(n) => (n, 0),
        }
    }
}

fn three(a: u16, b: u16, c: u16) -> [u8; 6] {
    let mut o = [0u8; 6];
    o[0..2].copy_from_slice(&a.to_le_bytes());
    o[2..4].copy_from_slice(&b.to_le_bytes());
    o[4..6].copy_from_slice(&c.to_le_bytes());
    o
}

// ---------------------------------------------------------------------------
// The container
//
//   magic "ENG1" | version u16 | interface_hash [8]
//   max_stack u16 | locals u16 | input_count u16
//   input_count × Check            (one per input leaf)
//   output_count u16 | output_count × Check
//   input_group_count u16  | × (guard u16, member_count u16, member_count × leaf u16)
//   output_group_count u16 | × (the same)
//                      (variant groups: exactly one member's presence leaf is
//                       non-nil, unless the guard leaf is nil; NO_GUARD: always)
//   import_count u16 | import_count × (sig_len u16, signature bytes, args u16, results u16)
//   list_count u16   | list_count × (max_items u16, item_leaves u16, item_leaves × Check)
//                      (input lists, read through the host's accessors)
//                      (host builtins, linked by signature at load)
//   consts_len u32 | consts      (string records: [len u16][bytes])
//   code_len u32   | code
//   span_count u32 | span_count × (pc u32, start u32, end u32)   (source byte offsets; the VM ignores them)
//
// A Check is: kind u8 | nullable u8 | then, for Int: lo i64, hi i64;
// for Str: max_len u16, enum_count u16, enum_at u32; nothing otherwise.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    BadMagic,
    BadVersion(u16),
    Truncated,
    TrailingBytes,
    BadKind,
    BadOpcode(usize),
    BadOperand(usize),
    /// An import is malformed, or a call doesn't match it.
    BadImport(u16),
    /// The host has no builtin with an import's signature.
    Unlinked(u16),
    /// A jump lands outside the code or inside an instruction.
    BadJump(usize),
    /// Stack heights disagree where paths meet, underflow, or exceed `max_stack`.
    BadStack(usize),
    /// Code that no path reaches, or a path that runs off the end.
    BadFlow(usize),
    /// The scratch buffer given to `validate` is shorter than the code.
    Scratch,
    /// Over a limit the host set.
    OverLimit,
}

#[derive(Clone)]
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

// The VM reads the input and output checks on every evaluation, so these are
// inlined and read fixed-size arrays (no `copy_from_slice`, which becomes a
// `memcpy` call).
impl<'a> Reader<'a> {
    #[inline]
    fn take(&mut self, n: usize) -> Result<&'a [u8], LoadError> {
        let s = self.b.get(self.at..self.at + n).ok_or(LoadError::Truncated)?;
        self.at += n;
        Ok(s)
    }
    #[inline]
    fn array<const N: usize>(&mut self) -> Result<[u8; N], LoadError> {
        let s = self.b.get(self.at..self.at + N).ok_or(LoadError::Truncated)?;
        self.at += N;
        let mut a = [0u8; N];
        for (d, &b) in a.iter_mut().zip(s) {
            *d = b;
        }
        Ok(a)
    }
    #[inline]
    fn u8(&mut self) -> Result<u8, LoadError> {
        let b = *self.b.get(self.at).ok_or(LoadError::Truncated)?;
        self.at += 1;
        Ok(b)
    }
    #[inline]
    fn u16(&mut self) -> Result<u16, LoadError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    #[inline]
    fn u32(&mut self) -> Result<u32, LoadError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    #[inline]
    fn i64(&mut self) -> Result<i64, LoadError> {
        Ok(i64::from_le_bytes(self.array()?))
    }
    #[inline]
    fn check(&mut self) -> Result<Check, LoadError> {
        let kind = Kind::from_u8(self.u8()?).ok_or(LoadError::BadKind)?;
        let nullable = match self.u8()? {
            0 => false,
            1 => true,
            _ => return Err(LoadError::BadKind),
        };
        let mut c = Check { nullable, ..Check::new(kind) };
        match kind {
            Kind::Int => {
                c.lo = self.i64()?;
                c.hi = self.i64()?;
                if c.lo > c.hi {
                    return Err(LoadError::BadKind);
                }
            }
            Kind::Str => {
                c.max_len = self.u16()?;
                c.enum_count = self.u16()?;
                c.enum_at = self.u32()?;
            }
            Kind::Bool | Kind::Float | Kind::List => {}
        }
        Ok(c)
    }
}

/// A table of checks (a program's inputs or its output), read in order.
#[derive(Clone)]
pub struct Checks<'a> {
    r: Reader<'a>,
    left: u16,
}

impl Iterator for Checks<'_> {
    type Item = Check;

    #[inline]
    fn next(&mut self) -> Option<Check> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        Some(self.r.check().expect("parsed"))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left as usize, Some(self.left as usize))
    }
}

/// A host builtin a program calls, as recorded in its header.
#[derive(Clone, Copy, Debug)]
pub struct Import<'a> {
    /// e.g. `distance({lat:f,lon:f},{lat:f,lon:f})f`: the name and the
    /// flattened types of its parameters and result.
    pub signature: &'a [u8],
    pub args: u16,
    pub results: u16,
}

/// A parsed program. Borrowed from the bytes it was parsed from.
#[derive(Clone, Copy, Debug)]
pub struct Program<'a> {
    pub interface_hash: [u8; 8],
    pub max_stack: u16,
    pub locals: u16,
    /// Input leaves.
    pub input_count: u16,
    inputs: &'a [u8],
    /// Output leaves.
    pub output_count: u16,
    outputs: &'a [u8],
    /// Variant groups in the inputs and in the output.
    pub input_group_count: u16,
    input_groups: &'a [u8],
    pub output_group_count: u16,
    output_groups: &'a [u8],
    /// Host builtins the program calls.
    pub import_count: u16,
    imports: &'a [u8],
    /// Input lists (top-level list inputs and lists inside input records).
    pub list_count: u16,
    lists: &'a [u8],
    pub consts: &'a [u8],
    pub code: &'a [u8],
    spans: &'a [u8],
}

impl<'a> Program<'a> {
    /// Parses the container. Doesn't look inside the code: see `validate`.
    pub fn parse(bytes: &'a [u8]) -> Result<Program<'a>, LoadError> {
        let mut r = Reader { b: bytes, at: 0 };
        if r.take(4)? != MAGIC {
            return Err(LoadError::BadMagic);
        }
        let version = r.u16()?;
        if version != VERSION {
            return Err(LoadError::BadVersion(version));
        }
        let mut interface_hash = [0u8; 8];
        interface_hash.copy_from_slice(r.take(8)?);
        let max_stack = r.u16()?;
        let locals = r.u16()?;
        let input_count = r.u16()?;
        let inputs_at = r.at;
        for _ in 0..input_count {
            r.check()?;
        }
        let inputs = &bytes[inputs_at..r.at];
        let output_count = r.u16()?;
        let outputs_at = r.at;
        for _ in 0..output_count {
            r.check()?;
        }
        let outputs = &bytes[outputs_at..r.at];
        // Each group's leaves must lie within its leaves (inputs or output).
        let groups = |r: &mut Reader<'a>, leaves: u16| -> Result<(u16, &'a [u8]), LoadError> {
            let count = r.u16()?;
            let at = r.at;
            for _ in 0..count {
                let guard = r.u16()?;
                if guard != NO_GUARD && guard >= leaves {
                    return Err(LoadError::BadKind);
                }
                let n = r.u16()?;
                if n == 0 {
                    return Err(LoadError::BadKind);
                }
                for _ in 0..n {
                    if r.u16()? >= leaves {
                        return Err(LoadError::BadKind);
                    }
                }
            }
            Ok((count, &bytes[at..r.at]))
        };
        let (input_group_count, input_groups) = groups(&mut r, input_count)?;
        let (output_group_count, output_groups) = groups(&mut r, output_count)?;
        let import_count = r.u16()?;
        let imports_at = r.at;
        for i in 0..import_count {
            let n = r.u16()? as usize;
            r.take(n)?;
            r.u16()?;
            if r.u16()? > MAX_HOST_RESULTS {
                return Err(LoadError::BadImport(i));
            }
        }
        let imports = &bytes[imports_at..r.at];
        let list_count = r.u16()?;
        let lists_at = r.at;
        for _ in 0..list_count {
            r.u16()?; // max_items
            let leaves = r.u16()?;
            for _ in 0..leaves {
                r.check()?;
            }
        }
        let lists = &bytes[lists_at..r.at];
        let n = r.u32()? as usize;
        let consts = r.take(n)?;
        let n = r.u32()? as usize;
        let code = r.take(n)?;
        let n = r.u32()? as usize;
        let spans = r.take(n.checked_mul(12).ok_or(LoadError::Truncated)?)?;
        if r.at != bytes.len() {
            return Err(LoadError::TrailingBytes);
        }
        let p = Program {
            interface_hash, max_stack, locals, input_count, inputs, output_count, outputs, input_group_count, input_groups, output_group_count,
            output_groups, import_count, imports, list_count, lists, consts, code, spans,
        };
        for i in 0..input_count {
            p.check_strings(&p.input(i))?;
        }
        for i in 0..output_count {
            p.check_strings(&p.output(i))?;
        }
        Ok(p)
    }

    /// The check for input leaf `i` (`i < input_count`). Checks are
    /// variable-length, so this reads the `i` before it: to visit them all,
    /// use [`Program::input_checks`].
    pub fn input(&self, i: u16) -> Check {
        nth_check(self.inputs, i)
    }

    /// The check for output leaf `i` (`i < output_count`). Like
    /// [`Program::input`], it reads the `i` before it: see
    /// [`Program::output_checks`].
    pub fn output(&self, i: u16) -> Check {
        nth_check(self.outputs, i)
    }

    /// Every input leaf's check, in order, in one pass.
    pub fn input_checks(&self) -> Checks<'a> {
        Checks { r: Reader { b: self.inputs, at: 0 }, left: self.input_count }
    }

    /// Every output leaf's check, in order, in one pass.
    pub fn output_checks(&self) -> Checks<'a> {
        Checks { r: Reader { b: self.outputs, at: 0 }, left: self.output_count }
    }

    /// The variant groups in the inputs (leaf numbers are input leaves).
    pub fn input_groups(&self) -> Groups<'a> {
        Groups { r: Reader { b: self.input_groups, at: 0 }, left: self.input_group_count }
    }

    /// The variant groups in the output (leaf numbers are output leaves).
    pub fn output_groups(&self) -> Groups<'a> {
        Groups { r: Reader { b: self.output_groups, at: 0 }, left: self.output_group_count }
    }

    /// Input list `i` (`i < list_count`): its `maxItems` and its items' leaf count.
    pub fn list(&self, i: u16) -> (u16, u16) {
        let mut r = Reader { b: self.lists, at: 0 };
        for k in 0..=i {
            let max = r.u16().expect("parsed");
            let leaves = r.u16().expect("parsed");
            if k == i {
                return (max, leaves);
            }
            for _ in 0..leaves {
                r.check().expect("parsed");
            }
        }
        unreachable!()
    }

    /// The check for leaf `leaf` of input list `i`'s items.
    pub fn list_item_check(&self, i: u16, leaf: u16) -> Check {
        let mut r = Reader { b: self.lists, at: 0 };
        for k in 0..=i {
            r.u16().expect("parsed");
            let leaves = r.u16().expect("parsed");
            for l in 0..leaves {
                let c = r.check().expect("parsed");
                if k == i && l == leaf {
                    return c;
                }
            }
        }
        unreachable!()
    }

    /// Import `i` (`i < import_count`): its signature, and its argument and
    /// result leaf counts.
    pub fn import(&self, i: u16) -> Import<'a> {
        let mut r = Reader { b: self.imports, at: 0 };
        for k in 0..=i {
            let n = r.u16().expect("parsed") as usize;
            let signature = r.take(n).expect("parsed");
            let args = r.u16().expect("parsed");
            let results = r.u16().expect("parsed");
            if k == i {
                return Import { signature, args, results };
            }
        }
        unreachable!()
    }

    /// The string record at `offset` in the constant pool.
    #[inline]
    pub fn string(&self, offset: u32) -> Option<&'a [u8]> {
        let o = offset as usize;
        let len = self.consts.get(o..o + 2)?;
        let len = u16::from_le_bytes([len[0], len[1]]) as usize;
        self.consts.get(o + 2..o + 2 + len)
    }

    /// Whether `s` is one of a string check's enum values (true when it has none).
    #[inline]
    pub fn in_enum(&self, c: &Check, s: &[u8]) -> bool {
        if c.enum_count == 0 {
            return true;
        }
        let mut at = c.enum_at;
        for _ in 0..c.enum_count {
            let v = match self.string(at) {
                Some(v) => v,
                None => return false,
            };
            if v == s {
                return true;
            }
            at += 2 + v.len() as u32;
        }
        false
    }

    fn check_strings(&self, c: &Check) -> Result<(), LoadError> {
        if c.kind != Kind::Str {
            return Ok(());
        }
        let mut at = c.enum_at;
        for _ in 0..c.enum_count {
            let v = self.string(at).ok_or(LoadError::BadKind)?;
            at += 2 + v.len() as u32;
        }
        Ok(())
    }

    /// The source span `(start, end)` (byte offsets) recorded for the
    /// instruction at `pc`, if any.
    pub fn span(&self, pc: u32) -> Option<(u32, u32)> {
        self.spans.chunks_exact(12).find_map(|e| {
            let w = |i: usize| u32::from_le_bytes([e[i], e[i + 1], e[i + 2], e[i + 3]]);
            (w(0) == pc).then(|| (w(4), w(8)))
        })
    }

    /// Checks every instruction, every operand and the stack height along
    /// every path. `scratch` must be at least as long as the code; it holds
    /// the expected stack height at each offset.
    ///
    /// Rules: each jump lands on an instruction start; heights agree where
    /// paths meet; the stack never underflows or exceeds `max_stack`; a
    /// `Peek` reads a slot below the height there; every
    /// instruction is reachable by a forward pass (fall-through or a jump to
    /// it from earlier code); no path runs off the end; `Return` leaves the
    /// stack empty.
    pub fn validate(&self, scratch: &mut [u16]) -> Result<(), LoadError> {
        const UNSET: u16 = u16::MAX;
        let code = self.code;
        if scratch.len() < code.len() {
            return Err(LoadError::Scratch);
        }
        let heights = &mut scratch[..code.len()];
        heights.fill(UNSET);
        if code.is_empty() {
            return Err(LoadError::BadFlow(0));
        }
        heights[0] = 0;
        let mut pc = 0;
        while pc < code.len() {
            let (op, next) = Op::decode(code, pc)?;
            if next > code.len() {
                return Err(LoadError::Truncated);
            }
            // A height recorded inside this instruction means a jump into its middle.
            if heights[pc + 1..next].iter().any(|&h| h != UNSET) {
                return Err(LoadError::BadJump(pc));
            }
            let h = heights[pc];
            if h == UNSET {
                return Err(LoadError::BadFlow(pc));
            }
            self.check_operands(&op, pc)?;
            let (pops, pushes) = op.stack_effect();
            if h < pops {
                return Err(LoadError::BadStack(pc));
            }
            // A slot that isn't on the stack here.
            if let Op::Peek(s) = op {
                if s >= h {
                    return Err(LoadError::BadStack(pc));
                }
            }
            let after = h - pops + pushes;
            if after > self.max_stack {
                return Err(LoadError::BadStack(pc));
            }
            let arrive = |at: usize, height: u16, heights: &mut [u16]| -> Result<(), LoadError> {
                if at >= code.len() {
                    return Err(LoadError::BadJump(pc));
                }
                match heights[at] {
                    UNSET if at > pc => {
                        heights[at] = height;
                        Ok(())
                    }
                    UNSET => Err(LoadError::BadJump(pc)), // backwards into the middle of something
                    seen if seen == height => Ok(()),
                    _ => Err(LoadError::BadStack(pc)),
                }
            };
            match op {
                Op::Jump(t) => arrive(t as usize, after, heights)?,
                Op::JumpIfFalse(t) | Op::JumpIfNotNil(t) => {
                    arrive(t as usize, after, heights)?;
                    arrive(next, after, heights).map_err(|_| LoadError::BadFlow(pc))?;
                }
                Op::Return(_) => {
                    if after != 0 {
                        return Err(LoadError::BadStack(pc));
                    }
                }
                // Ends the path; whatever is on the stack is abandoned.
                Op::Error(_) => {}
                _ => arrive(next, after, heights).map_err(|e| match e {
                    LoadError::BadJump(_) => LoadError::BadFlow(pc),
                    e => e,
                })?,
            }
            pc = next;
        }
        Ok(())
    }

    fn check_operands(&self, op: &Op, pc: usize) -> Result<(), LoadError> {
        let ok = match *op {
            Op::PushStr(o) => self.string(o).is_some(),
            Op::Input(i) => i < self.input_count,
            Op::Load(i) | Op::Store(i) => i < self.locals,
            Op::Call(b, n) => b.arity_ok(n),
            Op::CallHost { import, args, results } => {
                import < self.import_count && {
                    let im = self.import(import);
                    im.args == args && im.results == results
                }
            }
            Op::Return(n) => n == self.output_count,
            Op::PushInputList(id) => id < self.list_count,
            Op::Error(code) => (1..=3).contains(&code),
            Op::MakeList { base, len: n, stride } | Op::ListStore { base, cap: n, stride } | Op::ListMake { base, cap: n, stride } => {
                (base as u32 + n as u32 * stride as u32) <= self.locals as u32 && stride < u16::MAX
            }
            // The block, and its keys when they're kept apart, lie in the
            // locals; a key leaf lies in the item.
            Op::HoldInsert { base, cap, stride, mode } => {
                let keys = match hold_key_leaf(mode) {
                    Some(l) => {
                        if l >= stride {
                            return Err(LoadError::BadOperand(pc));
                        }
                        0
                    }
                    None => cap as u32,
                };
                (1..u16::MAX).contains(&stride) && (base as u32 + cap as u32 * stride as u32 + keys) <= self.locals as u32
            }
            _ => true,
        };
        if ok { Ok(()) } else { Err(LoadError::BadOperand(pc)) }
    }
}

// ---------------------------------------------------------------------------
// Writing (used by the compiler; no allocation, so the bytes go to a sink)

/// Writes a container. `consts` and `code` are already encoded; `spans` are
/// `(pc, start, end)`. Groups are `(guard, members)`, with [`NO_GUARD`]
/// for a group that is always checked.
#[allow(clippy::too_many_arguments)]
pub fn write_program(
    out: &mut impl FnMut(&[u8]),
    interface_hash: [u8; 8],
    max_stack: u16,
    locals: u16,
    inputs: &[Check],
    outputs: &[Check],
    input_groups: &[(u16, &[u16])],
    output_groups: &[(u16, &[u16])],
    imports: &[Import],
    lists: &[(u16, &[Check])],
    consts: &[u8],
    code: &[u8],
    spans: &[(u32, u32, u32)],
) {
    out(&MAGIC);
    out(&VERSION.to_le_bytes());
    out(&interface_hash);
    out(&max_stack.to_le_bytes());
    out(&locals.to_le_bytes());
    out(&(inputs.len() as u16).to_le_bytes());
    for c in inputs {
        write_check(out, c);
    }
    out(&(outputs.len() as u16).to_le_bytes());
    for c in outputs {
        write_check(out, c);
    }
    for groups in [input_groups, output_groups] {
        out(&(groups.len() as u16).to_le_bytes());
        for (guard, members) in groups {
            out(&guard.to_le_bytes());
            out(&(members.len() as u16).to_le_bytes());
            for m in *members {
                out(&m.to_le_bytes());
            }
        }
    }
    out(&(imports.len() as u16).to_le_bytes());
    for im in imports {
        out(&(im.signature.len() as u16).to_le_bytes());
        out(im.signature);
        out(&im.args.to_le_bytes());
        out(&im.results.to_le_bytes());
    }
    out(&(lists.len() as u16).to_le_bytes());
    for (max, checks) in lists {
        out(&max.to_le_bytes());
        out(&(checks.len() as u16).to_le_bytes());
        for c in *checks {
            write_check(out, c);
        }
    }
    out(&(consts.len() as u32).to_le_bytes());
    out(consts);
    out(&(code.len() as u32).to_le_bytes());
    out(code);
    out(&(spans.len() as u32).to_le_bytes());
    for &(pc, s, e) in spans {
        out(&pc.to_le_bytes());
        out(&s.to_le_bytes());
        out(&e.to_le_bytes());
    }
}

fn nth_check(table: &[u8], i: u16) -> Check {
    let mut r = Reader { b: table, at: 0 };
    let mut c = r.check().expect("parsed");
    for _ in 0..i {
        c = r.check().expect("parsed");
    }
    c
}

fn write_check(out: &mut impl FnMut(&[u8]), c: &Check) {
    out(&[c.kind as u8, c.nullable as u8]);
    match c.kind {
        Kind::Int => {
            out(&c.lo.to_le_bytes());
            out(&c.hi.to_le_bytes());
        }
        Kind::Str => {
            out(&c.max_len.to_le_bytes());
            out(&c.enum_count.to_le_bytes());
            out(&c.enum_at.to_le_bytes());
        }
        Kind::Bool | Kind::Float | Kind::List => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(op: Op) {
        let mut buf = [0u8; MAX_OP_LEN];
        let n = op.encode(&mut buf);
        let (back, next) = Op::decode(&buf[..n], 0).unwrap();
        assert_eq!(next, n);
        match (op, back) {
            (Op::PushFloat(a), Op::PushFloat(b)) => assert_eq!(a.to_bits(), b.to_bits()),
            _ => assert_eq!(op, back),
        }
    }

    #[test]
    fn every_op_roundtrips() {
        for op in [
            Op::PushNil, Op::PushTrue, Op::PushFalse, Op::PushInt(i64::MIN), Op::PushFloat(f64::NAN),
            Op::PushStr(7), Op::Input(3), Op::Load(65535), Op::Store(1), Op::Pop, Op::Dup, Op::Peek(258), Op::Add,
            Op::Sub, Op::Mul, Op::Div, Op::Mod, Op::Pow, Op::Neg, Op::Eq, Op::Ne, Op::Lt, Op::Le,
            Op::Gt, Op::Ge, Op::Not, Op::Jump(9), Op::JumpIfFalse(10), Op::JumpIfNotNil(11),
            Op::Call(Builtin::Max, 3), Op::CallHost { import: 2, args: 4, results: 3 }, Op::PushInputList(4),
            Op::MakeList { base: 1, len: 2, stride: 3 }, Op::ListStore { base: 4, cap: 64, stride: 3 },
            Op::ListMake { base: 4, cap: 64, stride: 3 }, Op::ListLen, Op::ListRead(5), Op::AssertNotNil, Op::Return(3),
            Op::HoldInsert { base: 7, cap: 9, stride: 3, mode: hold_mode(true, false, false, Some(2)) },
            Op::HoldInsert { base: 7, cap: 9, stride: 1, mode: hold_mode(false, true, true, None) },
        ] {
            roundtrip(op);
        }
    }

    fn program(ops: &[Op], max_stack: u16) -> ([u8; 256], usize) {
        let mut code = [0u8; 128];
        let mut n = 0;
        for op in ops {
            let mut buf = [0u8; MAX_OP_LEN];
            let k = op.encode(&mut buf);
            code[n..n + k].copy_from_slice(&buf[..k]);
            n += k;
        }
        let mut out = [0u8; 256];
        let mut len = 0;
        write_program(
            &mut |b: &[u8]| {
                out[len..len + b.len()].copy_from_slice(b);
                len += b.len();
            },
            [0; 8], max_stack, 0, &[], &[Check::new(Kind::Int)], &[], &[], &[], &[], &[], &code[..n], &[],
        );
        (out, len)
    }

    #[test]
    fn group_checks_roundtrip_and_are_validated() {
        let code = {
            let mut buf = [0u8; MAX_OP_LEN];
            let n = Op::PushInt(1).encode(&mut buf);
            let mut c = [0u8; 16];
            c[..n].copy_from_slice(&buf[..n]);
            let m = Op::Return(1).encode(&mut buf);
            c[n..n + m].copy_from_slice(&buf[..m]);
            (c, n + m)
        };
        let write = |groups: &[(u16, &[u16])], out: &mut [u8; 256]| {
            let mut len = 0;
            let ins = [Check::new(Kind::Bool); 4];
            write_program(
                &mut |b: &[u8]| {
                    out[len..len + b.len()].copy_from_slice(b);
                    len += b.len();
                },
                [0; 8], 1, 0, &ins, &[Check::new(Kind::Int)], groups, &[], &[], &[], &[], &code.0[..code.1], &[],
            );
            len
        };
        let mut b = [0u8; 256];
        let n = write(&[(NO_GUARD, &[0, 2]), (1, &[3])], &mut b);
        let p = Program::parse(&b[..n]).unwrap();
        let gs: [(u16, [u16; 2], usize); 2] = {
            let mut it = p.input_groups();
            let mut r = [(0, [0; 2], 0); 2];
            for slot in r.iter_mut() {
                let g = it.next().unwrap();
                let mut ms = [0u16; 2];
                let mut k = 0;
                for m in g.members() {
                    ms[k] = m;
                    k += 1;
                }
                *slot = (g.guard, ms, k);
            }
            assert!(it.next().is_none());
            r
        };
        assert_eq!(gs, [(NO_GUARD, [0, 2], 2), (1, [3, 0], 1)]);
        assert_eq!(p.output_groups().count(), 0);
        // A member past the input leaves, and an empty group, are refused.
        let n = write(&[(NO_GUARD, &[4])], &mut b);
        assert!(matches!(Program::parse(&b[..n]), Err(LoadError::BadKind)));
        let n = write(&[(NO_GUARD, &[])], &mut b);
        assert!(matches!(Program::parse(&b[..n]), Err(LoadError::BadKind)));
    }

    #[test]
    fn validates_a_ternary() {
        // true ? 1 : 2
        // 0 PushTrue | 1 JumpIfFalse(20) | 6 PushInt(1) | 15 Jump(29) | 20 PushInt(2) | 29 Return
        let (b, n) = program(&[Op::PushTrue, Op::JumpIfFalse(20), Op::PushInt(1), Op::Jump(29), Op::PushInt(2), Op::Return(1)], 1);
        let p = Program::parse(&b[..n]).unwrap();
        let mut scratch = [0u16; 64];
        p.validate(&mut scratch).unwrap();
    }

    #[test]
    fn rejects_bad_stacks_and_jumps() {
        let mut scratch = [0u16; 64];
        // Underflow.
        let (b, n) = program(&[Op::Add, Op::Return(1)], 2);
        assert!(matches!(Program::parse(&b[..n]).unwrap().validate(&mut scratch), Err(LoadError::BadStack(0))));
        // Over max_stack.
        let (b, n) = program(&[Op::PushTrue, Op::PushTrue, Op::Pop, Op::Return(1)], 1);
        assert!(matches!(Program::parse(&b[..n]).unwrap().validate(&mut scratch), Err(LoadError::BadStack(1))));
        // A jump into the middle of PushInt.
        let (b, n) = program(&[Op::Jump(7), Op::PushInt(1), Op::Return(1)], 1);
        assert!(Program::parse(&b[..n]).unwrap().validate(&mut scratch).is_err());
        // Runs off the end.
        let (b, n) = program(&[Op::PushTrue], 1);
        assert!(matches!(Program::parse(&b[..n]).unwrap().validate(&mut scratch), Err(LoadError::BadFlow(0))));
        // Heights disagree where paths meet: one path pushes twice.
        let (b, n) = program(&[Op::PushTrue, Op::JumpIfFalse(8), Op::PushTrue, Op::PushTrue, Op::Pop, Op::Return(1)], 2);
        assert!(Program::parse(&b[..n]).unwrap().validate(&mut scratch).is_err());
    }

    #[test]
    fn peek_reads_only_below_the_height() {
        let mut scratch = [0u16; 64];
        // 0 PushInt | 9 Peek(0) | 12 Add | 13 Return: slot 0 is on the stack.
        let (b, n) = program(&[Op::PushInt(1), Op::Peek(0), Op::Add, Op::Return(1)], 2);
        Program::parse(&b[..n]).unwrap().validate(&mut scratch).unwrap();
        // Slot 1 isn't (the height is 1 at the Peek).
        let (b, n) = program(&[Op::PushInt(1), Op::Peek(1), Op::Add, Op::Return(1)], 2);
        assert!(matches!(Program::parse(&b[..n]).unwrap().validate(&mut scratch), Err(LoadError::BadStack(9))));
    }
}
