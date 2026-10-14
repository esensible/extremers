//! Emits bytecode from the typed tree.
//!
//! Values are flattened into leaves (`types::leaves`): a record is its
//! fields' leaves in name order, an optional record is a presence leaf then
//! its fields. Records are values, never references: binding one copies its
//! leaves into locals, and reading a field loads that field's leaves. When
//! the `let` body's value starts with the binding itself (`let r = …; {a: r,
//! b: r.x}`), the binding is built there instead, on the stack, and read
//! where it lies (`Peek`): no locals and no copy. So the stack depth and the
//! locals in use are known exactly at every point, and the header's
//! `max_stack` and `locals` are the true maxima.
//!
//! The emitter owns all slot numbering. Locals are allocated like a stack:
//! a `let` takes its value's leaves for the length of its body, then gives
//! them back, so sibling scopes reuse the same slots.
//!
//! Lists are lowered from the pipeline IR (plan.rs): `consume` plans a
//! sink's list, and `lower` emits it by one routine: each hold's loop into
//! its block first, then each stage's state, then one loop per list or
//! range the items come from (backwards under a `reverse`; nested at a
//! `flatten`), with the stages inlined and the sink last (spec §7.6).

use crate::check::{Ptr, Sink, TExpr, TKind, children, is_pipeline};
use crate::diag::{Diag, Span};
use crate::plan::{self, HoldKind, Pipeline, Source, Stage, may_be_negative};
use crate::schema::{GroupCheck, Interface, LeafCheck, group_checks, leaf_checks};
use crate::syntax::{BinOp, Step, UnOp};
use crate::types::{Ty, field_offset, int_to_float, leaves, list_converts, same_layout, unopt};
use bytecode::{Check, Import, Kind, MAX_OP_LEN, Op};
use std::collections::{BTreeMap, HashMap, HashSet};

pub struct Emitted {
    pub bytes: Vec<u8>,
    pub max_stack: u16,
    pub locals: u16,
    pub output_leaves: u16,
    /// The locals reserved for list literals and held lists (part of `locals`).
    pub held_slots: u16,
}

#[derive(Clone, Copy)]
enum Root {
    Input(u16),
    Local(u16),
    /// Stack slots from this one (counted from the bottom): a binding built
    /// in place where its body's value is being built (see `let_`).
    Stack(u16),
    /// Item `i` of list `h`: the handle and the index are in these locals.
    Item { h: u16, i: u16 },
}

/// A predicate being emitted: what `#`, `#index` and `#acc` read.
#[derive(Clone)]
struct Ctx {
    item: Place,
    index: Option<u16>,
    acc: Option<(u16, Ty)>,
}

/// Where a value's leaves are: an input's or a local's leaves, from `off`.
#[derive(Clone, Copy)]
struct Place {
    root: Root,
    off: u16,
}

impl Place {
    fn at(self, d: u16) -> Place {
        Place { root: self.root, off: self.off + d }
    }
}

struct Emitter<'i> {
    iface: &'i Interface,
    code: Vec<u8>,
    consts: Vec<u8>,
    strings: HashMap<String, u32>,
    spans: Vec<(u32, u32, u32)>,
    depth: u16,
    max_depth: u16,
    next_local: u16,
    max_locals: u16,
    /// `let` binding id → where its leaves are: its first local, or its
    /// first stack slot.
    bindings: HashMap<u32, Root>,
    /// Reads of a binding that is already where they'd put it (the leading
    /// leaves of its `let` body's value): they emit nothing.
    placed: HashSet<*const TExpr>,
    /// Host builtins called, in import order (indices into the interface's builtins).
    imports: Vec<usize>,
    /// Predicates being emitted, innermost last.
    ctx: Vec<Ctx>,
    /// Input leaf → input list id, for the leaves that are lists.
    list_leaf: HashMap<u16, u16>,
    /// The blocks of locals reserved for the whole evaluation, one for each
    /// list literal and each held list, from local 0 (so a handle to one
    /// can be copied freely, and stays good after the evaluation): the
    /// next free one. Everything else is allocated above them.
    perm_next: u32,
    /// Each block, for the diagnostics.
    held: Vec<Held>,
    /// Each list literal's block, by node.
    lits: HashMap<*const TExpr, u16>,
    /// Each hold's block, by node and capacity.
    hold_blocks: HashMap<(*const TExpr, u32), u16>,
    /// The state of each stage of the sink being emitted.
    stages: HashMap<*const TExpr, StageState>,
    /// The jumps that leave each pipeline's loops (a window past its end;
    /// for a sink's whole pipeline, also the sink leaving early), by level.
    levels: Vec<Vec<usize>>,
    /// While a list is held: convert an int leaf written to a float one
    /// (the output converts scalars itself, but a list's items are stored
    /// as they are).
    floats: bool,
    /// The input and output leaves' checks, made before the code (their
    /// enum strings come first in the constants).
    checks: Option<(Vec<Check>, Vec<Check>)>,
    /// The program's text (for messages that quote it).
    src: &'i str,
    /// Inside a replay (a pipeline run again only to count its items): its
    /// lowering isn't checked for skips, as the one it counts for is.
    replay: u32,
    /// `E-SKIPPED`: a body a lowering evaluates for fewer items than Go
    /// does (or a predicate for more), which can fail (spec §2.7).
    skipped: Vec<Diag>,
}

/// The most slots of locals a program's list literals and held lists may
/// reserve between them.
pub const MAX_HELD_SLOTS: u32 = 32_768;

/// Writing a sink's items into a held list's block of locals.
struct Collect<'e> {
    base: u16,
    cap: u16,
    /// The items' type, as the block lays them out.
    item: Ty,
    /// How items are kept: in arrival order, sorted, or first occurrences.
    mode: HoldKind<'e>,
}

/// A stage's state, in locals.
#[derive(Clone, Debug)]
enum StageState {
    /// The mapped value (a flat-map has none), and the `#index` counter if
    /// the body reads it; `down`: it counts down (the items arrive backwards).
    Map { tmp: u16, counter: Option<u16>, down: bool },
    /// A slice or `take`: the items before `start` are skipped, and those
    /// from `end` on leave. `counter` counts the items that reached it; a
    /// window straight on a list has none: its loop visits only the window.
    Window { counter: Option<u16>, start: Option<u16>, end: Option<u16> },
}

/// A window's bounds for `direct`, each in a local (and whether it may be
/// negative).
enum Cut {
    Slice(Option<(u16, bool)>, Option<(u16, bool)>),
    Take(u16),
}

/// Where `direct` reads items: a list (its handle in `pre`) or a range (its
/// start and count from `pre`), or a `concat`'s parts.
enum Addr<'e> {
    /// `maps`: the `map` nodes the item goes through once read, in order.
    Leaf { pre: u16, range: bool, maps: Vec<&'e TExpr> },
    Concat(Vec<Part>),
}

/// A `concat` part `direct` reads: its list or range, stored as `item`,
/// the span `(lo, hi, rev)` of it the part sees (`[lo, hi)`, backwards when
/// `rev`; `None`: all of it, forwards), and its count, in `m`.
struct Part {
    pre: u16,
    range: bool,
    item: Ty,
    span: Option<(u16, u16, bool)>,
    m: u16,
}

/// A stage an item goes through after its own pipeline's: one of an
/// enclosing pipeline's (with that pipeline's level), or a `concat` part's
/// conversion to the whole's layout.
#[derive(Clone)]
enum Down<'e> {
    Node(Stage<'e>, usize),
    Convert(Ty, Ty, u16),
}

/// A sink's locals (see `consume`).
struct SinkLocals {
    sink: Sink,
    r: u16,
    aux: u16,
    pos: u16,
    target: u16,
    acc_ty: Ty,
    /// The level of the sink's whole pipeline.
    top: usize,
    /// `median`'s block of values, kept sorted: its first local and capacity.
    block: Option<(u16, u16)>,
    /// A backwards `findLast`/`findLastIndex` (plan.rs): it stops at the
    /// first match, and `findLastIndex`'s position counts from the local
    /// holding the count less one.
    backward: Option<u16>,
}

/// Emits the program. The first pass measures the blocks of locals the
/// list literals and held lists reserve (its code is dropped); the second
/// emits again with everything else allocated above them. Emitting is
/// deterministic, so the second pass reserves the same blocks.
pub fn emit(e: &TExpr, iface: &Interface, src: &str) -> Result<Emitted, Vec<Diag>> {
    let first = pass(e, iface, 0, src);
    if !first.skipped.is_empty() {
        return Err(first.skipped);
    }
    if first.perm_next > MAX_HELD_SLOTS {
        let h = first.held.iter().max_by_key(|h| h.slots).unwrap();
        let (what, help) = h.why.words();
        return Err(vec![Diag::error(
            "E-HOLD",
            h.span,
            format!(
                "{what}: up to {} items of {} slot{}, {} slots ({} bytes); a program's lists can hold at most {MAX_HELD_SLOTS} slots in all, and these need {}",
                h.bound,
                h.stride,
                if h.stride == 1 { "" } else { "s" },
                h.slots,
                h.slots as u64 * vm::SLOT_BYTES as u64,
                first.perm_next
            ),
        )
        .help(help)]);
    }
    let m = pass(e, iface, first.perm_next as u16, src);
    assert_eq!(m.perm_next, first.perm_next, "the passes reserve the same blocks");
    Ok(m.finish_program(e, iface))
}

/// One pass over the program, with `perm` locals reserved below the rest.
fn pass<'i>(e: &TExpr, iface: &'i Interface, perm: u16, src: &'i str) -> Emitter<'i> {
    let mut m = Emitter {
        iface,
        code: Vec::new(),
        consts: Vec::new(),
        strings: HashMap::new(),
        spans: Vec::new(),
        depth: 0,
        max_depth: 0,
        next_local: perm,
        max_locals: perm,
        bindings: HashMap::new(),
        placed: HashSet::new(),
        imports: Vec::new(),
        ctx: Vec::new(),
        list_leaf: iface.lists.iter().enumerate().map(|(id, l)| (l.leaf, id as u16)).collect(),
        perm_next: 0,
        held: Vec::new(),
        lits: HashMap::new(),
        hold_blocks: HashMap::new(),
        stages: HashMap::new(),
        levels: Vec::new(),
        floats: false,
        checks: None,
        src,
        replay: 0,
        skipped: Vec::new(),
    };
    // The checks' enum strings go into the constants first, as before.
    m.checks = Some(m.make_checks());
    m.emit_as(e, &iface.output);
    m
}

impl<'i> Emitter<'i> {
    fn make_checks(&mut self) -> (Vec<Check>, Vec<Check>) {
        let iface = self.iface;
        let mut ins = Vec::new();
        for i in &iface.inputs {
            leaf_checks(&i.ty, false, &mut ins);
        }
        let mut outs = Vec::new();
        leaf_checks(&iface.output, false, &mut outs);
        (ins.iter().map(|c| self.check(c)).collect(), outs.iter().map(|c| self.check(c)).collect())
    }

    fn finish_program(mut self, e: &TExpr, iface: &'i Interface) -> Emitted {
        let m = &mut self;
        let (inputs, outputs) = m.checks.take().unwrap();
        let n = leaves(&iface.output);
        m.spans.push((m.pc(), e.span.start, e.span.end));
        m.op(Op::Return(n));
        let imports: Vec<Import> = m
            .imports
            .iter()
            .map(|&d| {
                let decl = &iface.builtins[d];
                Import {
                    signature: decl.signature.as_bytes(),
                    args: decl.params.iter().map(|(_, t, _)| leaves(t)).sum(),
                    results: leaves(&decl.result),
                }
            })
            .collect();
        let list_checks: Vec<(u16, Vec<Check>)> = iface
            .lists
            .iter()
            .map(|l| {
                let mut cs = Vec::new();
                leaf_checks(&l.item, false, &mut cs);
                (l.max as u16, cs.iter().map(|c| m.check(c)).collect())
            })
            .collect();
        let lists: Vec<(u16, &[Check])> = list_checks.iter().map(|(n, cs)| (*n, cs.as_slice())).collect();
        // Variant groups: exactly one variant present (spec §7.3).
        let mut in_groups: Vec<GroupCheck> = Vec::new();
        for i in &iface.inputs {
            group_checks(&i.ty, i.base, None, &mut in_groups);
        }
        let mut out_groups: Vec<GroupCheck> = Vec::new();
        group_checks(&iface.output, 0, None, &mut out_groups);
        let as_table = |gs: &[GroupCheck]| -> Vec<(u16, Vec<u16>)> { gs.iter().map(|g| (g.guard.unwrap_or(bytecode::NO_GUARD), g.members.clone())).collect() };
        let (in_groups, out_groups) = (as_table(&in_groups), as_table(&out_groups));
        let in_groups: Vec<(u16, &[u16])> = in_groups.iter().map(|(g, m)| (*g, m.as_slice())).collect();
        let out_groups: Vec<(u16, &[u16])> = out_groups.iter().map(|(g, m)| (*g, m.as_slice())).collect();
        let mut bytes = Vec::new();
        bytecode::write_program(
            &mut |b: &[u8]| bytes.extend_from_slice(b),
            iface.hash,
            m.max_depth,
            m.max_locals,
            &inputs,
            &outputs,
            &in_groups,
            &out_groups,
            &imports,
            &lists,
            &m.consts,
            &m.code,
            &m.spans,
        );
        Emitted { bytes, max_stack: m.max_depth, locals: m.max_locals, output_leaves: n, held_slots: m.perm_next as u16 }
    }
}

impl Emitter<'_> {
    fn string(&mut self, s: &str) -> u32 {
        if let Some(&o) = self.strings.get(s) {
            return o;
        }
        let o = self.consts.len() as u32;
        self.consts.extend_from_slice(&(s.len() as u16).to_le_bytes());
        self.consts.extend_from_slice(s.as_bytes());
        self.strings.insert(s.to_string(), o);
        o
    }

    fn check(&mut self, c: &LeafCheck) -> Check {
        match c {
            LeafCheck::List => Check::new(Kind::List),
            LeafCheck::Bool { nullable } => Check { nullable: *nullable, ..Check::new(Kind::Bool) },
            LeafCheck::Float { nullable } => Check { nullable: *nullable, ..Check::new(Kind::Float) },
            LeafCheck::Int { range, nullable } => Check { lo: range.lo as i64, hi: range.hi as i64, nullable: *nullable, ..Check::new(Kind::Int) },
            LeafCheck::Str { max_len, values, nullable } => {
                let mut c = Check { max_len: *max_len as u16, nullable: *nullable, ..Check::new(Kind::Str) };
                if let Some(vs) = values {
                    // Enum values are laid out back to back, so the VM can walk them.
                    c.enum_at = self.consts.len() as u32;
                    c.enum_count = vs.len() as u16;
                    for v in vs {
                        self.consts.extend_from_slice(&(v.len() as u16).to_le_bytes());
                        self.consts.extend_from_slice(v.as_bytes());
                    }
                }
                c
            }
        }
    }

    fn pc(&self) -> u32 {
        self.code.len() as u32
    }

    fn op(&mut self, op: Op) {
        let (pops, pushes) = op.stack_effect();
        self.depth = self.depth - pops + pushes;
        self.max_depth = self.max_depth.max(self.depth);
        let mut buf = [0u8; MAX_OP_LEN];
        let n = op.encode(&mut buf);
        self.code.extend_from_slice(&buf[..n]);
    }

    /// An instruction that can fail at run time: record its source span.
    fn op_at(&mut self, op: Op, e: &TExpr) {
        self.spans.push((self.pc(), e.span.start, e.span.end));
        self.op(op);
    }

    /// Emits a jump with a placeholder target, returning where to patch it.
    fn jump(&mut self, op: fn(u32) -> Op, e: &TExpr) -> usize {
        let at = self.code.len();
        self.op_at(op(0), e);
        at
    }

    fn patch(&mut self, at: usize, target: u32) {
        // Every jump is opcode + u32 target.
        self.code[at + 1..at + 5].copy_from_slice(&target.to_le_bytes());
    }

    fn alloc(&mut self, n: u16) -> u16 {
        let base = self.next_local;
        self.next_local += n;
        self.max_locals = self.max_locals.max(self.next_local);
        base
    }

    fn free(&mut self, n: u16) {
        self.next_local -= n;
    }

    /// Pops a value of `n` leaves (last leaf on top) into fresh locals.
    fn store_new(&mut self, n: u16) -> u16 {
        let base = self.alloc(n);
        for i in (0..n).rev() {
            self.op(Op::Store(base + i));
        }
        base
    }

    fn push_nils(&mut self, n: u16) {
        for _ in 0..n {
            self.op(Op::PushNil);
        }
    }

    fn load_leaf(&mut self, p: Place) {
        match p.root {
            Root::Input(b) => match self.list_leaf.get(&(b + p.off)) {
                Some(&id) => self.op(Op::PushInputList(id)),
                None => self.op(Op::Input(b + p.off)),
            },
            Root::Local(b) => self.op(Op::Load(b + p.off)),
            Root::Stack(b) => self.op(Op::Peek(b + p.off)),
            Root::Item { h, i } => {
                self.op(Op::Load(h));
                self.op(Op::Load(i));
                self.op(Op::ListRead(p.off));
            }
        }
    }

    fn mark(&self) -> u16 {
        self.next_local
    }

    fn release(&mut self, mark: u16) {
        self.next_local = mark;
    }

    /// `local += 1`.
    fn incr(&mut self, l: u16) {
        self.op(Op::Load(l));
        self.op(Op::PushInt(1));
        self.op(Op::Add);
        self.op(Op::Store(l));
    }

    /// The int on top of the stack as a float (a nil stays nil when `nullable`).
    fn to_float(&mut self, nullable: bool) {
        if !nullable {
            self.op(Op::Call(bytecode::Builtin::Float, 1));
            return;
        }
        // v; Dup; JumpIfNotNil C; Jump E; C: float(v); E:
        self.op(Op::Dup);
        let have = self.jump_plain(Op::JumpIfNotNil);
        let end = self.jump_plain(Op::Jump);
        let here = self.pc();
        self.patch(have, here);
        self.op(Op::Call(bytecode::Builtin::Float, 1));
        let here = self.pc();
        self.patch(end, here);
    }

    /// A jump with a placeholder target and no span (it can't fail).
    fn jump_plain(&mut self, op: fn(u32) -> Op) -> usize {
        let at = self.code.len();
        self.op(op(0));
        at
    }

    /// Pops a value of type `t` into the leaves at `base`.
    fn store_into(&mut self, base: u16, t: &Ty) {
        for i in (0..leaves(t)).rev() {
            self.op(Op::Store(base + i));
        }
    }

    fn load_all(&mut self, base: u16, t: &Ty) {
        for i in 0..leaves(t) {
            self.op(Op::Load(base + i));
        }
    }

    /// Loads a value of type `src` at `p`, laid out as `dst`.
    fn load_as(&mut self, p: Place, src: &Ty, dst: &Ty) {
        if same_layout(src, dst) && !list_converts(src, dst) && !(self.floats && int_to_float(src, dst)) {
            for i in 0..leaves(src) {
                self.load_leaf(p.at(i));
            }
            return;
        }
        match (src, dst) {
            (Ty::Nil, _) => self.push_nils(leaves(dst)),
            // An int leaf into a float one, in a held list's item.
            _ if matches!(unopt(src), Ty::Int(_)) && matches!(unopt(dst), Ty::Float) => {
                self.load_leaf(p);
                self.to_float(src.may_be_nil());
            }
            // A list whose items are laid out differently: rewritten.
            _ if matches!(unopt(src), Ty::List(..)) => {
                self.load_leaf(p);
                let at = TExpr { kind: TKind::Nil, ty: src.clone(), span: Span { start: 0, end: 0 } };
                self.relist(src, dst, &at);
            }
            (Ty::Rec(sf, _), Ty::Rec(df, _)) => self.load_fields(p, sf, df),
            (Ty::Rec(sf, _), Ty::Opt(d)) => {
                let Ty::Rec(df, _) = &**d else { unreachable!("{src:?} as {dst:?}") };
                self.op(Op::PushTrue);
                self.load_fields(p, sf, df);
            }
            (Ty::Opt(s), Ty::Opt(d)) => {
                let (Ty::Rec(sf, _), Ty::Rec(df, _)) = (&**s, &**d) else { unreachable!("{src:?} as {dst:?}") };
                self.load_leaf(p);
                self.load_fields(p.at(1), sf, df);
            }
            _ => unreachable!("load {src:?} as {dst:?}"),
        }
    }

    fn load_fields(&mut self, p: Place, sf: &BTreeMap<String, Ty>, df: &BTreeMap<String, Ty>) {
        for (k, dt) in df {
            match sf.get(k) {
                Some(st) => self.load_as(p.at(field_offset(sf, k)), st, dt),
                // An optional field the value doesn't have: absent.
                None => self.push_nils(leaves(dt)),
            }
        }
    }

    /// Where `e`'s leaves already are, if it's an input or a binding.
    fn place_of(&self, e: &TExpr) -> Option<Place> {
        match &e.kind {
            TKind::Input(i) => Some(Place { root: Root::Input(self.iface.inputs[*i].base), off: 0 }),
            TKind::Local(id) => Some(Place { root: self.bindings[id], off: 0 }),
            TKind::Pointer { which: Ptr::Item, .. } => Some(self.ctx.last().unwrap().item),
            TKind::Pointer { which: Ptr::Acc, .. } => Some(Place { root: Root::Local(self.ctx.last().unwrap().acc.as_ref().unwrap().0), off: 0 }),
            // A path that needs no nil checks (every optional record on it is
            // narrowed) is just an offset into its base: its leaves are there,
            // laid out as its type.
            TKind::Path { base, steps, nonnil } => {
                let p = self.place_of(base)?;
                let (cur, off) = walk(&base.ty, steps, nonnil)?;
                let (_, off) = strip(&cur, off, *nonnil.last().unwrap());
                Some(p.at(off))
            }
            _ => None,
        }
    }

    /// Runs `f` with a place holding `e`'s value: where it already is, or
    /// in temporary locals for the duration.
    fn with_place(&mut self, e: &TExpr, f: impl FnOnce(&mut Self, Place)) {
        if let Some(p) = self.place_of(e) {
            f(self, p);
            return;
        }
        self.emit(e);
        let n = leaves(&e.ty);
        let base = self.store_new(n);
        f(self, Place { root: Root::Local(base), off: 0 });
        self.free(n);
    }

    /// Emits `e` laid out as `dst` (which `e.ty` unifies into).
    fn emit_as(&mut self, e: &TExpr, dst: &Ty) {
        if self.placed.contains(&(e as *const TExpr)) {
            return;
        }
        let floats = self.floats && int_to_float(&e.ty, dst);
        if same_layout(&e.ty, dst) && !list_converts(&e.ty, dst) && !floats {
            return self.emit(e);
        }
        match &e.kind {
            TKind::Nil => self.push_nils(leaves(dst)),
            // An int as a float, into a list's item (`floats`).
            _ if floats && matches!(unopt(&e.ty), Ty::Int(_)) => {
                self.emit(e);
                self.to_float(e.ty.may_be_nil());
            }
            // A built list is held laid out as `dst` wants it, in one go.
            _ if is_pipeline(e) => {
                let Ty::List(item, _) = unopt(dst) else { unreachable!("list as {dst:?}") };
                self.hold(e, item);
            }
            TKind::Record(fields) => {
                let df = match dst {
                    Ty::Rec(df, _) => df,
                    Ty::Opt(d) => {
                        self.op(Op::PushTrue);
                        let Ty::Rec(df, _) = &**d else { unreachable!() };
                        df
                    }
                    _ => unreachable!("record as {dst:?}"),
                };
                for (k, dt) in df {
                    match fields.iter().find(|(f, _)| f == k) {
                        Some((_, fe)) => self.emit_as(fe, dt),
                        None => self.push_nils(leaves(dt)),
                    }
                }
                // Fields the destination doesn't have (a record argument
                // with more than its parameter): Go evaluates them too.
                for (k, fe) in fields {
                    if !df.contains_key(k) {
                        self.evaluate_for_failure(fe);
                    }
                }
            }
            TKind::Cond(c, a, b) => self.cond(c, a, b, e, dst),
            TKind::Let { id, value, body } => self.let_(*id, value, body, dst),
            TKind::Coalesce(a, b) => self.coalesce(a, b, e, dst),
            TKind::Input(_) | TKind::Local(_) | TKind::Pointer { which: Ptr::Item | Ptr::Acc, .. } => {
                let p = self.place_of(e).unwrap();
                self.load_as(p, &e.ty, dst);
            }
            _ => self.with_place(e, |m, p| m.load_as(p, &e.ty, dst)),
        }
    }

    /// Emits `e` in its own type's layout.
    fn emit(&mut self, e: &TExpr) {
        if self.placed.contains(&(e as *const TExpr)) {
            return;
        }
        match &e.kind {
            TKind::Nil => self.op(Op::PushNil),
            TKind::Bool(true) => self.op(Op::PushTrue),
            TKind::Bool(false) => self.op(Op::PushFalse),
            TKind::Int(v) => self.op(Op::PushInt(*v)),
            TKind::Float(v) => self.op(Op::PushFloat(*v)),
            TKind::Str(s) => {
                let o = self.string(s);
                self.op(Op::PushStr(o));
            }
            TKind::Input(_) | TKind::Local(_) | TKind::Pointer { which: Ptr::Item | Ptr::Acc, .. } => {
                let p = self.place_of(e).unwrap();
                for i in 0..leaves(&e.ty) {
                    self.load_leaf(p.at(i));
                }
            }
            TKind::Pointer { which: Ptr::Index, .. } => {
                let l = self.ctx.last().unwrap().index.unwrap();
                self.op(Op::Load(l));
            }
            // Each literal fills its own block, then pushes a handle to it.
            TKind::ListLit(items) => {
                let Ty::List(item, _) = &e.ty else { unreachable!() };
                if items.is_empty() || matches!(**item, Ty::Never) {
                    self.op(Op::MakeList { base: 0, len: 0, stride: 1 });
                    return;
                }
                let stride = leaves(item);
                // One block per literal, also when a replay emits it again.
                let base = match self.lits.get(&(e as *const TExpr)) {
                    Some(&b) => b,
                    None => {
                        let b = self.perm_block(items.len() as u32, stride, e.span);
                        self.lits.insert(e as *const TExpr, b);
                        b
                    }
                };
                // A list's items are stored as they are: an int item of a
                // list of floats is converted here.
                let floats = std::mem::replace(&mut self.floats, true);
                for (k, it) in items.iter().enumerate() {
                    self.emit_as(it, item);
                    self.store_into(base.wrapping_add(k as u16 * stride), item);
                }
                self.floats = floats;
                self.op(Op::MakeList { base, len: items.len() as u16, stride });
            }
            // A list the program builds, where a value is needed: held.
            _ if is_pipeline(e) => {
                let Some(Ty::List(item, _)) = e.ty.non_nil() else { unreachable!("{:?}", e.ty) };
                self.hold(e, &item);
            }
            TKind::Sink { sink, src, f, init } => self.sink(*sink, src, f.as_deref(), init.as_deref(), e),
            TKind::Unary(op, a) => {
                self.emit(a);
                self.op_at(if *op == UnOp::Neg { Op::Neg } else { Op::Not }, e);
            }
            TKind::Binary(op, a, b) => {
                self.emit(a);
                self.emit(b);
                self.op_at(binop(*op), e);
            }
            // a && b:  a; JumpIfFalse F; b; Jump E; F: PushFalse; E:
            TKind::And(a, b) => {
                self.emit(a);
                let f = self.jump(Op::JumpIfFalse, a);
                self.emit(b);
                let end = self.jump(Op::Jump, e);
                self.depth -= 1;
                let f_pc = self.pc();
                self.op(Op::PushFalse);
                let e_pc = self.pc();
                self.patch(f, f_pc);
                self.patch(end, e_pc);
            }
            // a || b:  a; JumpIfFalse R; PushTrue; Jump E; R: b; E:
            TKind::Or(a, b) => {
                self.emit(a);
                let r = self.jump(Op::JumpIfFalse, a);
                self.op(Op::PushTrue);
                let end = self.jump(Op::Jump, e);
                self.depth -= 1;
                let r_pc = self.pc();
                self.emit(b);
                let e_pc = self.pc();
                self.patch(r, r_pc);
                self.patch(end, e_pc);
            }
            TKind::Coalesce(a, b) => self.coalesce(a, b, e, &e.ty),
            TKind::Cond(c, a, b) => self.cond(c, a, b, e, &e.ty),
            TKind::Let { id, value, body } => self.let_(*id, value, body, &e.ty),
            // a < b < c:  a; b; Store t; Load t; Lt; JumpIfFalse F; Load t; c; Lt; Jump E; F: PushFalse; E:
            TKind::Chain { first, rest } => {
                let temps = self.alloc(rest.len() as u16 - 1);
                self.emit(first);
                let mut falses = Vec::new();
                for (i, (op, operand)) in rest.iter().enumerate() {
                    if i > 0 {
                        self.op(Op::Load(temps + i as u16 - 1));
                    }
                    self.emit(operand);
                    let last = i == rest.len() - 1;
                    if !last {
                        self.op(Op::Store(temps + i as u16));
                        self.op(Op::Load(temps + i as u16));
                    }
                    self.op_at(binop(*op), operand);
                    if !last {
                        falses.push(self.jump(Op::JumpIfFalse, e));
                    }
                }
                let end = self.jump(Op::Jump, e);
                self.depth -= 1;
                let f_pc = self.pc();
                self.op(Op::PushFalse);
                let e_pc = self.pc();
                for f in falses {
                    self.patch(f, f_pc);
                }
                self.patch(end, e_pc);
                self.free(rest.len() as u16 - 1);
            }
            TKind::Call(b, args) => {
                for a in args {
                    self.emit(a);
                }
                self.op_at(Op::Call(*b, args.len() as u8), e);
            }
            TKind::Record(fields) => {
                for (_, fe) in fields {
                    self.emit(fe);
                }
            }
            TKind::Path { base, steps, nonnil } => self.path(base, steps, nonnil, e, false),
            // Each argument laid out as its parameter (a record argument
            // keeps only the parameter's fields); left-out optional
            // parameters are nil.
            TKind::HostCall { decl, args } => {
                let iface = self.iface;
                let d = &iface.builtins[*decl];
                let mut nargs = 0;
                for (i, (_, pty, _)) in d.params.iter().enumerate() {
                    match args.get(i) {
                        Some(a) => self.emit_as(a, pty),
                        None => self.push_nils(leaves(pty)),
                    }
                    nargs += leaves(pty);
                }
                let import = match self.imports.iter().position(|&x| x == *decl) {
                    Some(i) => i,
                    None => {
                        self.imports.push(*decl);
                        self.imports.len() - 1
                    }
                } as u16;
                self.op_at(Op::CallHost { import, args: nargs, results: leaves(&d.result) }, e);
            }
            TKind::IsNil { e: rec, negate } => {
                if let (Ty::Opt(_), TKind::Path { base, steps, nonnil }, None) = (&rec.ty, &rec.kind, self.place_of(rec)) {
                    // A path with checks on the way: follow it, but load
                    // only the presence leaf at its end.
                    self.path(base, steps, nonnil, rec, true);
                    self.op(Op::PushNil);
                    self.op(if *negate { Op::Ne } else { Op::Eq });
                } else if matches!(rec.ty, Ty::Opt(_)) {
                    // The presence leaf is nil exactly when the record is.
                    self.with_place(rec, |m, p| {
                        m.load_leaf(p);
                        m.op(Op::PushNil);
                        m.op(if *negate { Op::Ne } else { Op::Eq });
                    });
                } else {
                    // A record that can't be nil (Go still evaluates it).
                    self.evaluate_for_failure(rec);
                    self.op(if *negate { Op::PushTrue } else { Op::PushFalse });
                }
            }
            TKind::Map { .. }
            | TKind::Filter { .. }
            | TKind::Slice { .. }
            | TKind::Take { .. }
            | TKind::Concat(_)
            | TKind::Range { .. }
            | TKind::Reverse { .. }
            | TKind::SortBy { .. }
            | TKind::Uniq { .. }
            | TKind::Flatten { .. } => unreachable!("a pipeline is held above"),
        }
    }

    /// Evaluates `e` and drops its value, if it can fail: an expression Go
    /// evaluates whose value isn't needed (so a failure there fails here
    /// too, spec §2.7). One that can't fail emits nothing.
    fn evaluate_for_failure(&mut self, e: &TExpr) {
        if crate::check::can_fail(e) {
            self.emit(e);
            for _ in 0..leaves(&e.ty) {
                self.op(Op::Pop);
            }
        }
    }

    /// Emits `f` as a predicate over `item` (what `#` and friends read).
    fn in_ctx(&mut self, f: &TExpr, ctx: Ctx, dst: Option<&Ty>) {
        self.ctx.push(ctx);
        match dst {
            Some(t) => self.emit_as(f, t),
            None => self.emit(f),
        }
        self.ctx.pop();
    }

    /// A builtin that consumes a list. Straight on a list, `len`, `first`,
    /// `last`, `xs[i]` and `get` read what they need; everything else (and
    /// anything over a pipeline) is one loop per list the items come from:
    /// the list's handle, length and position in locals, the stages inlined,
    /// and the sink's state in locals. Nothing is held: a stage's value
    /// lives only for the item being processed.
    fn sink(&mut self, sink: Sink, src: &TExpr, f: Option<&TExpr>, init: Option<&TExpr>, e: &TExpr) {
        self.consume(sink, src, f, init, e, None, None);
    }

    /// A list the program builds, held where a value is needed: its items
    /// are written, laid out as `item`, into a block of its own (`bound`
    /// items) reserved for the whole evaluation, and the value is a handle
    /// to the block. The loop that writes them is a sink like any other.
    fn hold(&mut self, e: &TExpr, item: &Ty) {
        let stride = leaves(item);
        let Some(Ty::List(src_item, bound)) = e.ty.non_nil() else { unreachable!("hold {:?}", e.ty) };
        if matches!(*src_item, Ty::Never) || bound == 0 {
            self.op(Op::MakeList { base: 0, len: 0, stride });
            return;
        }
        let cap = bound.min(u16::MAX as u32) as u16;
        let mut pipe = plan::for_write(e);
        self.check_skips(&pipe, None, None);
        // A held list with no stages after it (`sortBy(…)` kept as a
        // value): its block is the list's.
        if pipe.stages.is_empty()
            && let Source::Hold(h) = pipe.source
        {
            let base = self.perm_block_why(bound, stride + key_slots(&h.kind, &h.inner.out_item(), item), e.span, HoldWhy::of(&h.kind));
            let c = Collect { base, cap, item: item.clone(), mode: h.kind };
            return self.lower_hold(*h, Some(&c));
        }
        let base = self.perm_block(bound, stride, e.span);
        let c = Collect { base, cap, item: item.clone(), mode: HoldKind::Plain };
        pipe.pre = None;
        self.lower(pipe, e, Sink::Len, None, None, e, Some(&c));
    }

    /// The list handle on top of the stack, typed `src`, as a handle to a
    /// list laid out as `dst`: its items rewritten into a block of their own
    /// (a nil handle stays nil).
    fn relist(&mut self, src: &Ty, dst: &Ty, at: &TExpr) {
        let Ty::List(si, bound) = unopt(src).clone() else { unreachable!("relist {src:?}") };
        let Ty::List(di, _) = unopt(dst) else { unreachable!("relist as {dst:?}") };
        let h = self.store_new(1);
        let mut end = None;
        if src.may_be_nil() {
            self.op(Op::Load(h));
            let have = self.jump(Op::JumpIfNotNil, at);
            self.op(Op::PushNil);
            end = Some(self.jump(Op::Jump, at));
            self.depth -= 1;
            let here = self.pc();
            self.patch(have, here);
        }
        let stride = leaves(di);
        if matches!(*si, Ty::Never) || bound == 0 {
            self.op(Op::MakeList { base: 0, len: 0, stride });
        } else {
            let base = self.perm_block(bound, stride, at.span);
            let c = Collect { base, cap: bound.min(u16::MAX as u32) as u16, item: (**di).clone(), mode: HoldKind::Plain };
            let list = TExpr { kind: TKind::Nil, ty: Ty::List(si, bound), span: at.span };
            self.consume(Sink::Len, &list, None, None, &list, Some(&c), Some(h));
        }
        if let Some(j) = end {
            let here = self.pc();
            self.patch(j, here);
        }
        self.free(1);
    }

    /// Reserves a block of `bound × stride` locals for the whole evaluation
    /// (a list literal's items, or a held list's), giving its first local.
    fn perm_block(&mut self, bound: u32, stride: u16, span: Span) -> u16 {
        self.perm_block_why(bound, stride, span, HoldWhy::Value)
    }

    fn perm_block_why(&mut self, bound: u32, stride: u16, span: Span, why: HoldWhy) -> u16 {
        let slots = (bound as u64 * stride as u64).min(u32::MAX as u64) as u32;
        let base = self.perm_next;
        self.perm_next = self.perm_next.saturating_add(slots);
        self.held.push(Held { span, bound, stride, slots, why });
        // Past the limit only in the measuring pass, whose code is dropped.
        base.min(u16::MAX as u32) as u16
    }

    /// `sink` over `src`, or the items written into a held list's block
    /// (`collect`, with `sink` `Len`: the count is the list's length).
    /// `handle`: the source list's handle, already in that local.
    ///
    /// `src` is planned (plan.rs: its source, stages and holds, for this
    /// sink), then lowered by `lower`.
    #[allow(clippy::too_many_arguments)]
    fn consume(&mut self, sink: Sink, src: &TExpr, f: Option<&TExpr>, init: Option<&TExpr>, e: &TExpr, collect: Option<&Collect>, handle: Option<u16>) {
        let mut pipe = match collect {
            Some(_) => plan::for_write(src),
            None => plan::for_sink(sink, f, src),
        };
        if collect.is_none() {
            self.check_skips(&pipe, Some(sink), f);
        }
        pipe.pre = handle;
        self.lower(pipe, src, sink, f, init, e, collect);
    }

    /// `E-SKIPPED` (spec §2.7): what the lowering of `pipe` for `sink`
    /// (`None`: a block write) evaluates for fewer items than Go does, or a
    /// predicate for more, where it can fail. Not for a replay: the lowering
    /// it counts for is checked.
    fn check_skips(&mut self, pipe: &Pipeline, sink: Option<Sink>, f: Option<&TExpr>) {
        if self.replay > 0 {
            return;
        }
        for s in plan::skips(pipe, sink, f) {
            let Some(fail) = crate::check::failures(s.node).into_iter().next() else { continue };
            // One for each expression: the first reason found.
            if !self.skipped.iter().any(|x| x.span == Some(s.node.span)) {
                self.skipped.push(skip_diag(&s, &fail, self.src));
            }
        }
    }

    /// Lowers a pipeline: its holds first (each a loop of its own, writing
    /// its block), then one loop per list its items come from, with the
    /// stages inlined and the sink's state in locals. Nothing else is held:
    /// a stage's value lives only for the item being processed.
    #[allow(clippy::too_many_arguments)]
    fn lower(&mut self, mut pipe: Pipeline, src: &TExpr, sink: Sink, f: Option<&TExpr>, init: Option<&TExpr>, e: &TExpr, collect: Option<&Collect>) {
        let mark = self.mark();
        let out = leaves(&e.ty);
        let mut end_jumps = Vec::new();

        // A hold's block is the list the rest of the pipeline reads.
        let held = self.resolve_hold(&mut pipe);
        // A plain list's handle, read once.
        if let (Source::List(source), None) = (&pipe.source, pipe.pre) {
            self.emit(source);
            pipe.pre = Some(self.store_new(1));
        }
        // A range's start and count, worked out once.
        if let (Source::Range(r), None) = (&pipe.source, pipe.pre) {
            pipe.pre = Some(self.range_bounds(r));
            if pipe.stages.is_empty() && sink == Sink::Len && collect.is_none() {
                self.op(Op::Load(pipe.pre.unwrap() + 1));
                return self.finish(mark, end_jumps);
            }
        }
        let pre = pipe.pre;
        if let (Source::List(_), Some(h), None) = (&pipe.source, pre, collect) {
            let item0 = pipe.item.clone();
            // These give nil (or false) for a nil list straight on it; the
            // rest fail, as in expr, and so does any list builtin between
            // (`first(map(xs, …))`, `x in uniq(xs)`, even when it is left
            // out). (A hold's block is never nil.)
            if !held && matches!(sink, Sink::First | Sink::Last | Sink::Get | Sink::In) && pipe.stages.is_empty() && pipe.dropped.is_empty() {
                // Go evaluates the index or the needle whether or not the
                // list is nil: so does this, where it can fail.
                if let Some(x) = f
                    && matches!(sink, Sink::Get | Sink::In)
                    && crate::check::can_fail(x)
                {
                    self.emit(x);
                    self.op(Op::Pop);
                }
                self.op(Op::Load(h));
                let have = self.jump(Op::JumpIfNotNil, e);
                if sink == Sink::In {
                    self.op(Op::PushFalse);
                } else {
                    self.push_nils(out);
                }
                end_jumps.push(self.jump(Op::Jump, e));
                self.depth -= out;
                let here = self.pc();
                self.patch(have, here);
            }
            if pipe.stages.is_empty() && self.fast(sink, f, e, h, &item0, &mut end_jumps) {
                return self.finish(mark, end_jumps);
            }
        }
        // Through windows and reverses only: the items are still addressable.
        if collect.is_none() && self.direct(&mut pipe, sink, f, e) {
            return self.finish(mark, end_jumps);
        }

        // How many items have reached the sink: only where a position is read.
        let pos = if matches!(sink, Sink::FindIndex | Sink::FindLastIndex | Sink::At | Sink::Get | Sink::Reduce) {
            let p = self.alloc(1);
            self.op(Op::PushInt(0));
            self.op(Op::Store(p));
            p
        } else {
            u16::MAX
        };
        // The sink's state: its result (or accumulator), and only what else it needs.
        let acc_ty = match sink {
            Sink::Reduce => e.ty.clone(),
            _ => Ty::Nil,
        };
        let r = self.alloc(out.max(leaves(&acc_ty)));
        // `mean`'s count, `in`'s needle, `xs[k]`/`get`'s found flag, `reduce`'s started flag.
        let aux = if matches!(sink, Sink::Mean | Sink::In | Sink::At | Sink::Get | Sink::Reduce) { self.alloc(1) } else { u16::MAX };
        // `xs[k]`/`get` over a pipeline: k.
        let target = if matches!(sink, Sink::At | Sink::Get) { self.alloc(1) } else { u16::MAX };
        // `median`'s values, held as floats, sorted as they arrive.
        let mut block = None;
        match sink {
            Sink::Len | Sink::Count | Sink::One | Sink::Sum => {
                self.op(Op::PushInt(0));
                self.op(Op::Store(r));
            }
            Sink::Median => {
                let n = match src.ty.non_nil() {
                    Some(Ty::List(_, n)) => n,
                    _ => 0,
                };
                let base = self.hold_block(e, n, 1, HoldWhy::Median);
                block = Some((base, n.min(u16::MAX as u32) as u16));
                self.op(Op::PushInt(0));
                self.op(Op::Store(r));
            }
            Sink::Mean => {
                self.op(Op::PushInt(0));
                self.op(Op::Store(r));
                self.op(Op::PushInt(0));
                self.op(Op::Store(aux));
            }
            Sink::Any => {
                self.op(Op::PushFalse);
                self.op(Op::Store(r));
            }
            Sink::All | Sink::NoneOf => {
                self.op(Op::PushTrue);
                self.op(Op::Store(r));
            }
            Sink::In => {
                self.op(Op::PushFalse);
                self.op(Op::Store(r));
                self.emit(f.unwrap());
                self.op(Op::Store(aux));
            }
            Sink::Min | Sink::Max | Sink::Find | Sink::FindLast | Sink::FindIndex | Sink::FindLastIndex | Sink::First | Sink::Last => {
                self.push_nils(out);
                self.store_into(r, &e.ty);
            }
            Sink::At | Sink::Get => {
                self.push_nils(out);
                self.store_into(r, &e.ty);
                self.op(Op::PushFalse);
                self.op(Op::Store(aux)); // found
                self.emit(f.unwrap());
                self.op(Op::Store(target));
                // A negative index counts from the end: count the pipeline first.
                self.op(Op::Load(target));
                self.op(Op::PushInt(0));
                self.op_at(Op::Lt, e);
                let pos_ok = self.jump(Op::JumpIfFalse, e);
                // The same pipeline, counted (a replay, planned for a count).
                let as_int = TExpr { kind: TKind::Nil, ty: Ty::Int(crate::types::Range::FULL), span: e.span };
                self.op(Op::Load(target));
                self.replay += 1;
                self.consume(Sink::Len, src, None, None, &as_int, None, if held { None } else { pre });
                self.replay -= 1;
                self.op(Op::Add);
                self.op(Op::Store(target));
                let here = self.pc();
                self.patch(pos_ok, here);
            }
            Sink::Reduce => match init {
                Some(init) => {
                    self.emit_as(init, &acc_ty);
                    self.store_into(r, &acc_ty);
                    self.op(Op::PushTrue);
                    self.op(Op::Store(aux)); // started
                }
                None => {
                    self.op(Op::PushFalse);
                    self.op(Op::Store(aux));
                }
            },
        }

        // A backwards `findLast`/`findLastIndex` (a list or range through
        // `map`s and `filter`s): `findLastIndex`'s position of the first
        // item from the end is the count less one.
        let backward = pipe.backward.then(|| {
            if sink != Sink::FindLastIndex {
                return u16::MAX;
            }
            let last = self.alloc(1);
            let h = pipe.pre.unwrap();
            if matches!(pipe.source, Source::Range(_)) {
                self.op(Op::Load(h + 1));
            } else {
                self.op(Op::Load(h));
                self.op_at(Op::ListLen, e);
            }
            self.op(Op::PushInt(1));
            self.op(Op::Sub);
            self.op(Op::Store(last));
            last
        });

        // Every stage's state (and each window's bounds), before any loop.
        // The table is this sink's: a sink run inside it (a replay, or one
        // in a predicate) sets up its own and gives this one back.
        let outer = std::mem::take(&mut self.stages);
        self.init_stages(&mut pipe, false);
        let top = self.new_level();
        let sl = SinkLocals { sink, r, aux, pos, target, acc_ty: acc_ty.clone(), top, block, backward };
        self.loops(&pipe, top, &[], &sl, f, init, e, collect);
        self.stages = outer;
        let end = self.pc();
        for b in std::mem::take(&mut self.levels[top]) {
            self.patch(b, end);
        }

        // The result.
        if let Some(c) = collect {
            self.op(Op::Load(r));
            // A sorted hold that stopped taking items counts one past its capacity.
            if let HoldKind::Sorted { tail: false, .. } = c.mode {
                self.op(Op::PushInt(c.cap as i64));
                self.op(Op::Call(bytecode::Builtin::Min, 2));
            }
            self.op(Op::ListMake { base: c.base, cap: c.cap, stride: leaves(&c.item) });
            return self.finish(mark, end_jumps);
        }
        match sink {
            Sink::Len | Sink::Count | Sink::Sum | Sink::Any | Sink::All | Sink::NoneOf | Sink::In => self.op(Op::Load(r)),
            Sink::Median => {
                // 0.0 for an empty list; else the middle value, or the mean
                // of the middle two (as expr does).
                let (base, cap) = block.unwrap();
                let (hl, m) = (self.alloc(1), self.alloc(1));
                self.op(Op::Load(r));
                self.op(Op::PushInt(0));
                self.op(Op::Eq);
                let some = self.jump(Op::JumpIfFalse, e);
                self.op(Op::PushFloat(0.0));
                let done = self.jump(Op::Jump, e);
                self.depth -= 1;
                let here = self.pc();
                self.patch(some, here);
                self.op(Op::Load(r));
                self.op(Op::ListMake { base, cap, stride: 1 });
                self.op(Op::Store(hl));
                self.op(Op::Load(r));
                self.op(Op::PushInt(2));
                self.op(Op::Div);
                self.op(Op::Call(bytecode::Builtin::Int, 1));
                self.op(Op::Store(m));
                self.op(Op::Load(r));
                self.op(Op::PushInt(2));
                self.op(Op::Mod);
                self.op(Op::PushInt(0));
                self.op(Op::Eq);
                let odd = self.jump(Op::JumpIfFalse, e);
                self.op(Op::Load(hl));
                self.op(Op::Load(m));
                self.op(Op::PushInt(1));
                self.op(Op::Sub);
                self.op(Op::ListRead(0));
                self.op(Op::Load(hl));
                self.op(Op::Load(m));
                self.op(Op::ListRead(0));
                self.op(Op::Add);
                self.op(Op::PushInt(2));
                self.op(Op::Div);
                let done2 = self.jump(Op::Jump, e);
                self.depth -= 1;
                let here = self.pc();
                self.patch(odd, here);
                self.op(Op::Load(hl));
                self.op(Op::Load(m));
                self.op(Op::ListRead(0));
                let here = self.pc();
                self.patch(done, here);
                self.patch(done2, here);
            }
            Sink::One => {
                self.op(Op::Load(r));
                self.op(Op::PushInt(1));
                self.op(Op::Eq);
            }
            Sink::Mean => {
                // 0.0 for an empty list, as in expr.
                self.op(Op::Load(aux));
                self.op(Op::PushInt(0));
                self.op(Op::Eq);
                let some = self.jump(Op::JumpIfFalse, e);
                self.op(Op::PushFloat(0.0));
                let done = self.jump(Op::Jump, e);
                self.depth -= 1;
                let here = self.pc();
                self.patch(some, here);
                self.op(Op::Load(r));
                self.op(Op::Load(aux));
                self.op_at(Op::Div, e);
                let here = self.pc();
                self.patch(done, here);
            }
            Sink::Min | Sink::Max | Sink::Find | Sink::FindLast | Sink::FindIndex | Sink::FindLastIndex | Sink::First | Sink::Last | Sink::Get => {
                self.load_all(r, &e.ty)
            }
            Sink::At => {
                self.op(Op::Load(aux));
                let found = self.jump(Op::JumpIfFalse, e);
                self.load_all(r, &e.ty);
                let done = self.jump(Op::Jump, e);
                self.depth -= out;
                let here = self.pc();
                self.patch(found, here);
                self.spans.push((self.pc(), e.span.start, e.span.end));
                self.op(Op::Error(2)); // R-INDEX
                // That path ends; the one arriving at `done` carries the result.
                self.depth += out;
                let here = self.pc();
                self.patch(done, here);
            }
            Sink::Reduce => {
                // No initial value and no items: R-ARG, as in expr.
                self.op(Op::Load(aux));
                let ok = self.jump(Op::JumpIfFalse, e);
                self.load_all(r, &acc_ty);
                let done = self.jump(Op::Jump, e);
                self.depth -= leaves(&acc_ty);
                let here = self.pc();
                self.patch(ok, here);
                self.spans.push((self.pc(), e.span.start, e.span.end));
                self.op(Op::Error(3)); // R-ARG
                // That path ends; the one arriving at `done` carries the result.
                self.depth += leaves(&acc_ty);
                let here = self.pc();
                self.patch(done, here);
            }
        }
        self.finish(mark, end_jumps);
    }

    /// The fast paths straight on a list (handle in `h`): `len`, `first`,
    /// `last`, `xs[k]` and `get` read what they need, with no loop. Whether
    /// `sink` is one of them (its result is then on the stack).
    fn fast(&mut self, sink: Sink, f: Option<&TExpr>, e: &TExpr, h: u16, item0: &Ty, end_jumps: &mut Vec<usize>) -> bool {
        let out = leaves(&e.ty);
        match sink {
            Sink::Len => {
                self.op(Op::Load(h));
                self.op_at(Op::ListLen, e);
                true
            }
            Sink::First | Sink::Last | Sink::At | Sink::Get => {
                let k = self.alloc(1);
                match sink {
                    Sink::First => self.op(Op::PushInt(0)),
                    Sink::Last => {
                        self.op(Op::Load(h));
                        self.op_at(Op::ListLen, e);
                        self.op(Op::PushInt(1));
                        self.op(Op::Sub);
                    }
                    _ => {
                        self.emit(f.unwrap());
                        // A negative index counts from the end.
                        self.op(Op::Dup);
                        self.op(Op::PushInt(0));
                        self.op_at(Op::Lt, e);
                        let pos = self.jump(Op::JumpIfFalse, e);
                        self.op(Op::Load(h));
                        self.op_at(Op::ListLen, e);
                        self.op(Op::Add);
                        let here = self.pc();
                        self.patch(pos, here);
                    }
                }
                self.op(Op::Store(k));
                if sink != Sink::At {
                    // In range, or nil: 0 <= k < len.
                    self.op(Op::Load(k));
                    self.op(Op::PushInt(0));
                    self.op(Op::Lt);
                    let neg = self.jump(Op::JumpIfFalse, e);
                    self.push_nils(out);
                    end_jumps.push(self.jump(Op::Jump, e));
                    self.depth -= out;
                    let here = self.pc();
                    self.patch(neg, here);
                    self.op(Op::Load(k));
                    self.op(Op::Load(h));
                    self.op_at(Op::ListLen, e);
                    self.op(Op::Lt);
                    let inside = self.jump(Op::JumpIfFalse, e);
                    let p = Place { root: Root::Item { h, i: k }, off: 0 };
                    self.load_as(p, item0, &e.ty);
                    end_jumps.push(self.jump(Op::Jump, e));
                    self.depth -= out;
                    let here = self.pc();
                    self.patch(inside, here);
                    self.push_nils(out);
                } else {
                    // The VM checks the range (R-INDEX).
                    self.spans.push((self.pc(), e.span.start, e.span.end));
                    let p = Place { root: Root::Item { h, i: k }, off: 0 };
                    self.load_as(p, item0, &e.ty);
                }
                true
            }
            _ => false,
        }
    }

    /// `len`, `first`, `last`, `xs[k]` and `get` over items that are all
    /// still where they lie: a list or a range, or a `concat` whose parts
    /// are each one of those (through their own windows and `reverse`s),
    /// seen only through windows (slices, `take`) and `reverse`s. A window
    /// is worked out as a span `[lo, hi)` of the positions it sees (by the
    /// rules a loop over it applies: a negative bound counts from the end,
    /// and each bound is clamped to the items there are), and the item is
    /// read where it lies. A `concat`'s positions are its parts' items in
    /// turn: its length is the sum of theirs, and its item `k` is in the
    /// first part whose count exceeds what is left of `k`. No loop: the
    /// same few instructions whatever the length (a few more per `concat`
    /// part). Whether `sink` over `pipe` is one of them (its result is then
    /// on the stack).
    fn direct(&mut self, pipe: &mut Pipeline, sink: Sink, f: Option<&TExpr>, e: &TExpr) -> bool {
        // (`maps` whose bodies can't fail are allowed: plan::direct_ok.)
        if !plan::direct_ok(pipe, sink) {
            return false;
        }
        // Where the items are read from, and the span over them.
        let addr;
        let (lo, hi, m);
        let rev;
        match &mut pipe.source {
            Source::List(_) | Source::Range(_) => {
                let Some(pre) = pipe.pre else { return false };
                let range = matches!(pipe.source, Source::Range(_));
                // Every window's bounds, innermost first, before anything
                // else (as the loop evaluates them).
                let cuts = self.cuts(&pipe.stages);
                (lo, hi, m) = (self.alloc(1), self.alloc(1), self.alloc(1));
                self.source_span(pre, range, lo, hi, e);
                rev = self.narrow(cuts, lo, hi, m);
                // A `map` doesn't move items: its body is evaluated for the
                // item read, after it is found.
                let maps = pipe.stages.iter().filter_map(|s| if let Stage::Map(n) = s { Some(*n) } else { None }).collect();
                addr = Addr::Leaf { pre, range, maps };
            }
            Source::Concat(ps) => {
                // Each part's list or range, then its windows' bounds, in
                // turn (as expr evaluates `concat`'s arguments). A part with
                // no items (`[]`) has nothing to read.
                let mut each = Vec::new();
                for p in ps.iter_mut() {
                    self.resolve_hold(p);
                    let src = match (&p.source, &p.item) {
                        (_, Ty::Never) => None,
                        (Source::List(s), _) => Some(match p.pre {
                            Some(h) => (h, false),
                            None => {
                                self.emit(s);
                                (self.store_new(1), false)
                            }
                        }),
                        (Source::Range(r), _) => Some((self.range_bounds(r), true)),
                        _ => unreachable!("a part's hold is resolved"),
                    };
                    let cuts = self.cuts(&p.stages);
                    each.push((src, cuts, p.item.clone()));
                }
                // Each part's span and count (a nil list fails here, R-NIL,
                // as `concat` does in expr). For `len` of the concat itself,
                // the counts are only added up, on the stack.
                let sum = sink == Sink::Len && pipe.stages.is_empty();
                let mut parts = Vec::new();
                for (src, cuts, item) in each {
                    let Some((pre, range)) = src else { continue };
                    // The count: pushed (`sum`), or into `m`.
                    let (span, m) = if cuts.is_empty() {
                        // The whole list or range: its length.
                        if range {
                            self.op(Op::Load(pre + 1));
                        } else {
                            self.op(Op::Load(pre));
                            self.op_at(Op::ListLen, e);
                        }
                        (None, if sum { u16::MAX } else { self.store_new(1) })
                    } else {
                        let (lo, hi, m) = (self.alloc(1), self.alloc(1), self.alloc(1));
                        self.source_span(pre, range, lo, hi, e);
                        let rev = self.narrow(cuts, lo, hi, m);
                        if sum {
                            self.op(Op::Load(hi));
                            self.op(Op::Load(lo));
                            self.op(Op::Sub);
                        } else {
                            self.span_count(lo, hi, m);
                        }
                        (Some((lo, hi, rev)), m)
                    };
                    if sum && !parts.is_empty() {
                        self.op(Op::Add);
                    }
                    parts.push(Part { pre, range, item, span, m });
                }
                if sum {
                    if parts.is_empty() {
                        self.op(Op::PushInt(0));
                    }
                    return true;
                }
                // The concat's own windows, over [0, the parts' total).
                let cuts = self.cuts(&pipe.stages);
                (lo, hi, m) = (self.alloc(1), self.alloc(1), self.alloc(1));
                self.op(Op::PushInt(0));
                self.op(Op::Store(lo));
                if parts.is_empty() {
                    self.op(Op::PushInt(0));
                }
                for (n, p) in parts.iter().enumerate() {
                    self.op(Op::Load(p.m));
                    if n > 0 {
                        self.op(Op::Add);
                    }
                }
                self.op(Op::Store(hi));
                rev = self.narrow(cuts, lo, hi, m);
                addr = Addr::Concat(parts);
            }
            Source::Hold(_) => unreachable!("a hold is resolved before `direct`"),
        }
        if sink == Sink::Len {
            self.op(Op::Load(hi));
            self.op(Op::Load(lo));
            self.op(Op::Sub);
            return true;
        }
        self.direct_read(&addr, &pipe.item, lo, hi, m, rev, sink, f, e);
        true
    }

    /// A window's bounds (each into a local), innermost first; `None` for a
    /// `reverse`. A negative `take` fails (R-ARG), as in expr.
    fn cuts(&mut self, stages: &[Stage]) -> Vec<Option<Cut>> {
        let mut cuts = Vec::new();
        for s in stages {
            let st = s.node();
            match (s, &st.kind) {
                (Stage::Reverse(_), _) => cuts.push(None),
                (Stage::Window(_), TKind::Slice { lo, hi, .. }) => {
                    let bound = |m: &mut Self, b: &Option<Box<TExpr>>| {
                        b.as_ref().map(|b| {
                            m.emit(b);
                            (m.store_new(1), may_be_negative(b))
                        })
                    };
                    let lo = bound(self, lo);
                    let hi = bound(self, hi);
                    cuts.push(Some(Cut::Slice(lo, hi)));
                }
                (Stage::Window(_), TKind::Take { k, .. }) => {
                    // A negative count fails (R-ARG), as in expr.
                    self.emit(k);
                    let n = self.store_new(1);
                    self.op(Op::Load(n));
                    self.op(Op::PushInt(0));
                    self.op_at(Op::Lt, st);
                    let ok = self.jump(Op::JumpIfFalse, st);
                    self.spans.push((self.pc(), st.span.start, st.span.end));
                    self.op(Op::Error(3));
                    let here = self.pc();
                    self.patch(ok, here);
                    cuts.push(Some(Cut::Take(n)));
                }
                // A `map` moves no item (`direct` evaluates it on the one read).
                (Stage::Map(_), _) => {}
                _ => unreachable!("stage {:?}", st.kind),
            }
        }
        cuts
    }

    /// The span `[lo, hi)` of a whole list (handle in `pre`) or range
    /// (start and count from `pre`): `[0, its length)`.
    fn source_span(&mut self, pre: u16, range: bool, lo: u16, hi: u16, e: &TExpr) {
        self.op(Op::PushInt(0));
        self.op(Op::Store(lo));
        if range {
            self.op(Op::Load(pre + 1));
        } else {
            self.op(Op::Load(pre));
            self.op_at(Op::ListLen, e);
        }
        self.op(Op::Store(hi));
    }

    /// `m = hi - lo`: the items in the span.
    fn span_count(&mut self, lo: u16, hi: u16, m: u16) {
        self.op(Op::Load(hi));
        self.op(Op::Load(lo));
        self.op(Op::Sub);
        self.op(Op::Store(m));
    }

    /// A bound on top of the stack, counted from the end of the `m` items
    /// if negative.
    fn from_end(&mut self, m: u16) {
        self.op(Op::Dup);
        self.op(Op::PushInt(0));
        self.op(Op::Lt);
        let pos = self.jump_plain(Op::JumpIfFalse);
        self.op(Op::Load(m));
        self.op(Op::Add);
        let here = self.pc();
        self.patch(pos, here);
    }

    /// Narrows the span `[lo, hi)` by each window of what's left, in turn
    /// (`m` is scratch). Whether the items then reach the end of the span
    /// backwards: its first item is then the one at `hi - 1`.
    fn narrow(&mut self, cuts: Vec<Option<Cut>>, lo: u16, hi: u16, m: u16) -> bool {
        let count = |me: &mut Self| me.span_count(lo, hi, m);
        let from_end = |me: &mut Self| me.from_end(m);
        let mut rev = false;
        for cut in cuts {
            let Some(cut) = cut else {
                rev = !rev;
                continue;
            };
            count(self);
            // The window as offsets `[a, b)` into the span (`a` absent: 0).
            let (a, b) = match cut {
                Cut::Slice(a, b) => {
                    // b: from the end, clamped to [0, m]; absent, m.
                    let b = match b {
                        Some((t, neg)) => {
                            self.op(Op::Load(t));
                            if neg {
                                from_end(self);
                                self.op(Op::PushInt(0));
                                self.op(Op::Call(bytecode::Builtin::Max, 2));
                            }
                            self.op(Op::Load(m));
                            self.op(Op::Call(bytecode::Builtin::Min, 2));
                            Some(self.store_new(1))
                        }
                        None => None,
                    };
                    // a: from the end, at least 0, at most b.
                    let a = a.map(|(t, neg)| {
                        self.op(Op::Load(t));
                        if neg {
                            from_end(self);
                            self.op(Op::PushInt(0));
                            self.op(Op::Call(bytecode::Builtin::Max, 2));
                        }
                        self.op(Op::Load(b.unwrap_or(m)));
                        self.op(Op::Call(bytecode::Builtin::Min, 2));
                        self.store_new(1)
                    });
                    (a, b)
                }
                Cut::Take(k) => {
                    self.op(Op::Load(k));
                    self.op(Op::Load(m));
                    self.op(Op::Call(bytecode::Builtin::Min, 2));
                    (None, Some(self.store_new(1)))
                }
            };
            // Forwards: [lo + a, lo + b). Backwards: [hi - b, hi - a).
            let (near, far, op) = if rev { (hi, lo, Op::Sub) } else { (lo, hi, Op::Add) };
            if let Some(b) = b {
                self.op(Op::Load(near));
                self.op(Op::Load(b));
                self.op(op);
                self.op(Op::Store(far));
            }
            if let Some(a) = a {
                self.op(Op::Load(near));
                self.op(Op::Load(a));
                self.op(op);
                self.op(Op::Store(near));
            }
        }
        rev
    }

    /// `first`, `last`, `xs[k]` or `get` of the items in the span
    /// `[lo, hi)` (backwards when `rev`) of `addr` (`m` is scratch).
    #[allow(clippy::too_many_arguments)]
    fn direct_read(&mut self, addr: &Addr, item: &Ty, lo: u16, hi: u16, m: u16, rev: bool, sink: Sink, f: Option<&TExpr>, e: &TExpr) {
        let count = |me: &mut Self| me.span_count(lo, hi, m);
        let from_end = |me: &mut Self| me.from_end(m);
        // The position `k` within the window (a negative index counts from
        // its end), then whether it is inside: `xs[k]` fails if not (R-INDEX),
        // the others give nil.
        count(self);
        let k = self.alloc(1);
        let may_neg = match sink {
            Sink::First => {
                self.op(Op::PushInt(0));
                false
            }
            Sink::Last => {
                self.op(Op::Load(m));
                self.op(Op::PushInt(1));
                self.op(Op::Sub);
                true
            }
            _ => {
                let f = f.unwrap();
                self.emit(f);
                let neg = may_be_negative(f);
                if neg {
                    from_end(self);
                }
                neg
            }
        };
        self.op(Op::Store(k));
        let out = leaves(&e.ty);
        let mut ends = Vec::new();
        let outside = |me: &mut Self, ends: &mut Vec<usize>| {
            if sink == Sink::At {
                me.spans.push((me.pc(), e.span.start, e.span.end));
                me.op(Op::Error(2)); // R-INDEX
            } else {
                me.push_nils(out);
                ends.push(me.jump_plain(Op::Jump));
                me.depth -= out;
            }
        };
        if may_neg {
            self.op(Op::Load(k));
            self.op(Op::PushInt(0));
            self.op(Op::Lt);
            let inside = self.jump_plain(Op::JumpIfFalse);
            outside(self, &mut ends);
            let here = self.pc();
            self.patch(inside, here);
        }
        // `last`'s position is always below the count.
        let below = if sink == Sink::Last {
            None
        } else {
            self.op(Op::Load(k));
            self.op(Op::Load(m));
            self.op(Op::Lt);
            Some(self.jump_plain(Op::JumpIfFalse))
        };
        // The position in the span's source: `lo + k`, or backwards `hi - 1 - k`.
        self.position(lo, hi, k, rev);
        match addr {
            Addr::Leaf { pre, range, maps } if !maps.is_empty() => {
                // The item, then each `map`'s body over it in turn.
                let (mut at, mut ty) = if *range {
                    self.op(Op::Load(*pre));
                    self.op(Op::Load(k));
                    self.op(Op::Add);
                    self.op(Op::Store(k));
                    (Place { root: Root::Local(k), off: 0 }, item.clone())
                } else {
                    (Place { root: Root::Item { h: *pre, i: k }, off: 0 }, item.clone())
                };
                for m in maps {
                    let TKind::Map { body, .. } = &m.kind else { unreachable!() };
                    self.in_ctx(body, Ctx { item: at, index: None, acc: None }, None);
                    let tmp = self.store_new(leaves(&body.ty));
                    at = Place { root: Root::Local(tmp), off: 0 };
                    ty = body.ty.clone();
                }
                self.load_as(at, &ty, &e.ty);
            }
            Addr::Leaf { pre, range, .. } => self.read_leaf(*pre, *range, k, item, e),
            Addr::Concat(parts) => {
                // The part it is in: the first whose count is more than
                // what is left of `k`, less the counts before it. (`k` is
                // below the total, so it is in the last part if not before.)
                // The position within a part, where it isn't `k` itself.
                let j = if parts.iter().any(|p| p.span.is_some() || p.range) { self.alloc(1) } else { u16::MAX };
                let mut found = Vec::new();
                if parts.is_empty() {
                    // No items: never reached (`k` is never inside).
                    self.push_nils(out);
                }
                for (n, p) in parts.iter().enumerate() {
                    let next = (n + 1 < parts.len()).then(|| {
                        self.op(Op::Load(k));
                        self.op(Op::Load(p.m));
                        self.op(Op::Lt);
                        self.jump_plain(Op::JumpIfFalse)
                    });
                    match p.span {
                        // A whole list: `k` is its position.
                        None if !p.range => self.read_leaf(p.pre, false, k, &p.item, e),
                        _ => {
                            self.op(Op::Load(k));
                            self.op(Op::Store(j));
                            if let Some((lo, hi, rev)) = p.span {
                                self.position(lo, hi, j, rev);
                            }
                            self.read_leaf(p.pre, p.range, j, &p.item, e);
                        }
                    }
                    if let Some(next) = next {
                        found.push(self.jump_plain(Op::Jump));
                        self.depth -= out;
                        let here = self.pc();
                        self.patch(next, here);
                        self.op(Op::Load(k));
                        self.op(Op::Load(p.m));
                        self.op(Op::Sub);
                        self.op(Op::Store(k));
                    }
                }
                let here = self.pc();
                for x in found {
                    self.patch(x, here);
                }
            }
        }
        if let Some(below) = below {
            ends.push(self.jump_plain(Op::Jump));
            self.depth -= out;
            let here = self.pc();
            self.patch(below, here);
            outside(self, &mut ends);
            // That path ended (or jumped to the end with its nils); the
            // others arrive at the end with the result.
            self.depth += out;
        }
        let here = self.pc();
        for j in ends {
            self.patch(j, here);
        }
    }

    /// `k = lo + k`, or backwards `k = hi - 1 - k`: position `k` of the
    /// span `[lo, hi)` as a position in its source.
    fn position(&mut self, lo: u16, hi: u16, k: u16, rev: bool) {
        if rev {
            self.op(Op::Load(hi));
            self.op(Op::PushInt(1));
            self.op(Op::Sub);
            self.op(Op::Load(k));
            self.op(Op::Sub);
        } else {
            self.op(Op::Load(lo));
            self.op(Op::Load(k));
            self.op(Op::Add);
        }
        self.op(Op::Store(k));
    }

    /// Pushes item `k` (in that local) of a list (handle in `pre`) or a
    /// range (start in `pre`), stored as `item`, laid out as `e`'s type.
    fn read_leaf(&mut self, pre: u16, range: bool, k: u16, item: &Ty, e: &TExpr) {
        let p = if range {
            // A range's item: its start plus the position.
            self.op(Op::Load(pre));
            self.op(Op::Load(k));
            self.op(Op::Add);
            self.op(Op::Store(k));
            Place { root: Root::Local(k), off: 0 }
        } else {
            Place { root: Root::Item { h: pre, i: k }, off: 0 }
        };
        self.load_as(p, item, &e.ty);
    }

    fn new_level(&mut self) -> usize {
        self.levels.push(Vec::new());
        self.levels.len() - 1
    }

    /// Allocates and sets up every stage's state in `pipe`, before any of
    /// its loops: a map's value (and its `#index` counter if its body reads
    /// it), a window's counter and bounds, and for a `concat` part laid out
    /// differently from the whole, the converted item. A `concat` part's
    /// hold is lowered here too, before any loop.
    ///
    /// `rev_after`: the items reach the end of `pipe` backwards (an odd
    /// number of `reverse`s follow it). A stage the items reach backwards
    /// counts from the end: a map's `#index` counts down from the number of
    /// items that reach it, less one, and a window's bounds are turned
    /// around, both from that number (the list's length, or a replay that
    /// counts them).
    fn init_stages(&mut self, pipe: &mut Pipeline, rev_after: bool) {
        let depth = self.ctx.len();
        let pre = pipe.pre;
        let all_rev = rev_after ^ (pipe.stages.iter().filter(|s| matches!(s, Stage::Reverse(_))).count() % 2 == 1);
        if let Source::Concat(parts) = &mut pipe.source {
            let whole = pipe.item.clone();
            for part in parts.iter_mut() {
                self.resolve_hold(part);
                // A nil list fails `concat` (R-NIL) before any item is
                // taken, as in expr, even when the sink is done before
                // this part's loop (`first`, `any`, …).
                if let (Source::List(s), None) = (&part.source, part.pre)
                    && s.ty.may_be_nil()
                    && !matches!(part.item, Ty::Never)
                {
                    self.emit(s);
                    let h = self.store_new(1);
                    self.op(Op::Load(h));
                    self.op_at(Op::ListLen, s);
                    self.op(Op::Pop);
                    part.pre = Some(h);
                }
                self.init_stages(part, all_rev);
                let from = part.out_item();
                if !matches!(from, Ty::Never) && !same_layout(&from, &whole) {
                    let tmp = self.alloc(leaves(&whole));
                    part.convert = Some((from, whole.clone(), tmp));
                }
            }
        }
        let random = matches!(pipe.source, Source::List(_) | Source::Range(_));
        let is_range = matches!(pipe.source, Source::Range(_));
        let mut rev = all_rev;
        for (k, stage) in pipe.stages.clone().into_iter().enumerate() {
            // Whether the items reach this stage backwards.
            let back = rev;
            if let Stage::Reverse(_) = stage {
                rev = !rev;
            }
            let st = stage.node();
            let as_int = TExpr { kind: TKind::Nil, ty: Ty::Int(crate::types::Range::FULL), span: st.span };
            // The number of items that reach this stage, into a local.
            let count = |m: &mut Self, inner: &TExpr| -> u16 {
                match (k, pre) {
                    // Straight on a list or range: its length.
                    (0, Some(h)) if random => {
                        if is_range {
                            m.op(Op::Load(h + 1));
                        } else {
                            m.op(Op::Load(h));
                            m.op_at(Op::ListLen, &as_int);
                        }
                    }
                    _ => {
                        m.replay += 1;
                        m.consume(Sink::Len, inner, None, None, &as_int, None, None);
                        m.replay -= 1;
                    }
                }
                m.store_new(1)
            };
            let state = match (&stage, &st.kind) {
                (Stage::Map(_) | Stage::FlatMap(_), TKind::Map { src: inner, body }) => {
                    let tmp = if let Stage::Map(_) = stage { self.alloc(leaves(&body.ty)) } else { u16::MAX };
                    let counter = if uses_index(body, depth) {
                        if back {
                            let n = count(self, inner);
                            self.op(Op::Load(n));
                            self.op(Op::PushInt(1));
                            self.op(Op::Sub);
                            self.op(Op::Store(n));
                            Some(n)
                        } else {
                            let c = self.alloc(1);
                            self.op(Op::PushInt(0));
                            self.op(Op::Store(c));
                            Some(c)
                        }
                    } else {
                        None
                    };
                    StageState::Map { tmp, counter, down: back }
                }
                (Stage::Filter { .. } | Stage::Reverse(_) | Stage::Flatten(_), _) => continue,
                (Stage::Window(_), TKind::Slice { src: inner, lo, hi }) => {
                    // The count before the window, where a bound may be
                    // negative, or where the items arrive backwards.
                    let neg = |b: &Option<Box<TExpr>>| b.as_ref().is_some_and(|b| may_be_negative(b));
                    let total = if neg(lo) || neg(hi) || back { Some(count(self, inner)) } else { None };
                    let start = lo.as_ref().map(|b| self.window_bound(b, total));
                    let end = hi.as_ref().map(|b| self.window_bound(b, total));
                    let folded = k == 0 && random;
                    if back && !folded {
                        let (start, end) = self.turn_around(start, end, total.unwrap());
                        self.window(start, end, false)
                    } else {
                        self.window(start, end, folded)
                    }
                }
                (Stage::Window(_), TKind::Take { src: inner, k: count_e }) => {
                    let total = if back && !(k == 0 && random) { Some(count(self, inner)) } else { None };
                    let count = count_e;
                    // A negative count fails (R-ARG), as in expr.
                    self.emit(count);
                    let end = self.store_new(1);
                    self.op(Op::Load(end));
                    self.op(Op::PushInt(0));
                    self.op_at(Op::Lt, st);
                    let ok = self.jump(Op::JumpIfFalse, st);
                    self.spans.push((self.pc(), st.span.start, st.span.end));
                    self.op(Op::Error(3));
                    let here = self.pc();
                    self.patch(ok, here);
                    match total {
                        Some(n) => {
                            let (start, end) = self.turn_around(None, Some(end), n);
                            self.window(start, end, false)
                        }
                        None => self.window(None, Some(end), k == 0 && random),
                    }
                }
                _ => unreachable!("stage {:?}", st.kind),
            };
            self.stages.insert(st as *const TExpr, state);
        }
    }

    /// Lowers `pipe`'s hold, if its source is one: the held pipeline runs
    /// into the hold's block, and `pipe` reads the block as a list. Whether
    /// there was one.
    fn resolve_hold(&mut self, pipe: &mut Pipeline) -> bool {
        let Source::Hold(_) = pipe.source else { return false };
        let Source::Hold(h) = std::mem::replace(&mut pipe.source, Source::Concat(Vec::new())) else { unreachable!() };
        let node = h.node;
        // A block that holds nothing (`take(sort(xs), 0)`) has no loop.
        if h.cap == 0 {
            pipe.item = Ty::Never;
        }
        self.lower_hold(*h, None);
        pipe.pre = Some(self.store_new(1));
        pipe.source = Source::List(node);
        true
    }

    /// A hold (plan.rs): its pipeline runs into its block (`into`, or a
    /// block of its own, reserved for the whole evaluation), and the block's
    /// handle is pushed. A sorted block keeps each item's key beside it
    /// (after the items), unless the key is one of the item's own leaves.
    fn lower_hold(&mut self, h: plan::Hold, into: Option<&Collect>) {
        let item = h.inner.out_item();
        let c = match into {
            Some(c) => Collect { base: c.base, cap: c.cap, item: c.item.clone(), mode: h.kind },
            None => {
                let stride = leaves(&item);
                let cap = h.cap.min(u16::MAX as u32);
                let base = self.hold_block(h.node, cap, stride + key_slots(&h.kind, &item, &item), HoldWhy::of(&h.kind));
                Collect { base, cap: cap as u16, item, mode: h.kind }
            }
        };
        if c.cap == 0 || matches!(h.inner.item, Ty::Never) {
            self.op(Op::MakeList { base: 0, len: 0, stride: leaves(&c.item) });
            return;
        }
        self.lower(h.inner, h.node, Sink::Len, None, None, h.node, Some(&c));
    }

    /// A hold's block of `cap` items of `per_item` slots, reserved for the
    /// whole evaluation; the same node held again (a replay) reuses it.
    fn hold_block(&mut self, node: &TExpr, cap: u32, per_item: u16, why: HoldWhy) -> u16 {
        let key = (node as *const TExpr, cap);
        if let Some(&b) = self.hold_blocks.get(&key) {
            return b;
        }
        let b = self.perm_block_why(cap, per_item, node.span, why);
        self.hold_blocks.insert(key, b);
        b
    }

    /// A window `[start:end)` over `n` items, as the window over the same
    /// items counted from the end: `[n - end:n - start)`.
    fn turn_around(&mut self, start: Option<u16>, end: Option<u16>, n: u16) -> (Option<u16>, Option<u16>) {
        let from_end = |m: &mut Self, b: u16| {
            m.op(Op::Load(n));
            m.op(Op::Load(b));
            m.op(Op::Sub);
            m.store_new(1)
        };
        let new_start = end.map(|b| from_end(self, b));
        let new_end = start.map(|b| from_end(self, b));
        (new_start, new_end)
    }

    /// A window's state: its counter (unless it is folded into the loop
    /// over the list it slices, which then visits only the window).
    fn window(&mut self, start: Option<u16>, end: Option<u16>, folded: bool) -> StageState {
        let counter = if folded {
            None
        } else {
            let c = self.alloc(1);
            self.op(Op::PushInt(0));
            self.op(Op::Store(c));
            Some(c)
        };
        StageState::Window { counter, start, end }
    }

    /// A slice bound in a local: a negative one counts from the end of the
    /// `total` items (and is clamped at 0), as in expr.
    fn window_bound(&mut self, b: &TExpr, total: Option<u16>) -> u16 {
        self.emit(b);
        let t = self.store_new(1);
        if let (Some(n), true) = (total, may_be_negative(b)) {
            self.op(Op::Load(t));
            self.op(Op::PushInt(0));
            self.op(Op::Lt);
            let done = self.jump(Op::JumpIfFalse, b);
            self.op(Op::Load(t));
            self.op(Op::Load(n));
            self.op(Op::Add);
            self.op(Op::Dup);
            self.op(Op::PushInt(0));
            self.op(Op::Lt);
            let keep = self.jump(Op::JumpIfFalse, b);
            self.op(Op::Pop);
            self.op(Op::PushInt(0));
            let here = self.pc();
            self.patch(keep, here);
            self.op(Op::Store(t));
            let here = self.pc();
            self.patch(done, here);
        }
        t
    }

    /// The loops over `pipe`'s items: one per list they come from (each
    /// part of a `concat` in turn), each running the items through `pipe`'s
    /// own stages, then `down` (the stages of the pipelines it is part of),
    /// then the sink. `level` is `pipe`'s: a window of it that has passed
    /// its end leaves every loop of it.
    #[allow(clippy::too_many_arguments)]
    fn loops(&mut self, pipe: &Pipeline, level: usize, down: &[Down], sl: &SinkLocals, f: Option<&TExpr>, init: Option<&TExpr>, e: &TExpr, collect: Option<&Collect>) {
        let mut mine: Vec<Down> = pipe.stages.iter().map(|s| Down::Node(*s, level)).collect();
        mine.extend(down.iter().cloned());
        match &pipe.source {
            Source::List(_) | Source::Range(_) => {
                // A list with no items (`[]`) has no loop.
                if !matches!(pipe.item, Ty::Never) {
                    self.leaf_loop(pipe, &mine, sl, f, init, e, collect);
                }
            }
            Source::Concat(parts) => {
                // Walked backwards, the last part comes first.
                let order: Vec<&Pipeline> = if reversed(&mine) { parts.iter().rev().collect() } else { parts.iter().collect() };
                for part in order {
                    let sub = self.new_level();
                    let mut part_down: Vec<Down> = Vec::new();
                    if let Some((from, to, tmp)) = &part.convert {
                        part_down.push(Down::Convert(from.clone(), to.clone(), *tmp));
                    }
                    part_down.extend(mine.iter().cloned());
                    self.loops(part, sub, &part_down, sl, f, init, e, collect);
                    let here = self.pc();
                    for b in std::mem::take(&mut self.levels[sub]) {
                        self.patch(b, here);
                    }
                }
            }
            Source::Hold(_) => unreachable!("a hold is lowered before the loops"),
        }
        if level != sl.top {
            let here = self.pc();
            for b in std::mem::take(&mut self.levels[level]) {
                self.patch(b, here);
            }
        }
    }

    /// A range's start and count (`b - a + 1`, or 0), in two locals from
    /// the one given. Its ends are evaluated left to right, once.
    fn range_bounds(&mut self, r: &TExpr) -> u16 {
        let TKind::Range { a, b } = &r.kind else { unreachable!("range {:?}", r.kind) };
        let base = self.alloc(2);
        self.emit(a);
        self.op(Op::Store(base));
        // The count can't overflow: the checker bounds it by 65,535.
        self.emit(b);
        self.op(Op::Load(base));
        self.op(Op::Sub);
        self.op(Op::PushInt(1));
        self.op(Op::Add);
        self.op(Op::Dup);
        self.op(Op::PushInt(0));
        self.op(Op::Lt);
        let keep = self.jump(Op::JumpIfFalse, r);
        self.op(Op::Pop);
        self.op(Op::PushInt(0));
        let here = self.pc();
        self.patch(keep, here);
        self.op(Op::Store(base + 1));
        base
    }

    /// The loop over one list (the source's handle, read here or already in
    /// `pipe.pre`) or one range. Backwards when a `reverse` follows.
    #[allow(clippy::too_many_arguments)]
    fn leaf_loop(&mut self, pipe: &Pipeline, stages: &[Down], sl: &SinkLocals, f: Option<&TExpr>, init: Option<&TExpr>, e: &TExpr, collect: Option<&Collect>) {
        let mark = self.mark();
        // A list's handle, or a range's start; and the count, in `n`.
        let (h, start_of_range) = match &pipe.source {
            Source::List(source) => {
                let h = match pipe.pre {
                    Some(h) => h,
                    None => {
                        self.emit(source);
                        self.store_new(1)
                    }
                };
                (h, None)
            }
            Source::Range(r) => {
                let base = match pipe.pre {
                    Some(b) => b,
                    None => self.range_bounds(r),
                };
                (base + 1, Some(base))
            }
            _ => unreachable!(),
        };
        let n = self.alloc(1);
        self.op(Op::Load(h));
        if start_of_range.is_none() {
            self.op_at(Op::ListLen, e);
        }
        self.op(Op::Store(n));
        let i = self.alloc(1);
        // A window straight on the list: visit only its items.
        match pipe.stages.first().and_then(|s| self.stages.get(&(s.node() as *const TExpr))) {
            Some(&StageState::Window { counter: None, start, end }) => {
                match start {
                    Some(s) => self.op(Op::Load(s)),
                    None => self.op(Op::PushInt(0)),
                }
                self.op(Op::Store(i));
                if let Some(end) = end {
                    self.op(Op::Load(end));
                    self.op(Op::Load(n));
                    self.op(Op::Lt);
                    let keep = self.jump(Op::JumpIfFalse, e);
                    self.op(Op::Load(end));
                    self.op(Op::Store(n));
                    let here = self.pc();
                    self.patch(keep, here);
                }
            }
            _ => {
                self.op(Op::PushInt(0));
                self.op(Op::Store(i));
            }
        }
        // Backwards: `i` still runs up from the first index to the last,
        // and the item read is `first + last - i`.
        let back = if reversed(stages) {
            let s = self.alloc(2);
            self.op(Op::Load(i));
            self.op(Op::Load(n));
            self.op(Op::Add);
            self.op(Op::PushInt(1));
            self.op(Op::Sub);
            self.op(Op::Store(s));
            Some((s, s + 1))
        } else {
            None
        };
        let tmp = start_of_range.map(|_| self.alloc(1));

        // L: while i < n
        let head = self.pc();
        self.op(Op::Load(i));
        self.op(Op::Load(n));
        self.op(Op::Lt);
        let exit = self.jump(Op::JumpIfFalse, e);
        let at = match back {
            Some((s, k)) => {
                self.op(Op::Load(s));
                self.op(Op::Load(i));
                self.op(Op::Sub);
                self.op(Op::Store(k));
                k
            }
            None => i,
        };
        // A range's item: its start plus the index.
        let item = match (start_of_range, tmp) {
            (Some(a), Some(t)) => {
                self.op(Op::Load(a));
                self.op(Op::Load(at));
                self.op(Op::Add);
                self.op(Op::Store(t));
                Place { root: Root::Local(t), off: 0 }
            }
            _ => Place { root: Root::Item { h, i: at }, off: 0 },
        };
        let mut conts = Vec::new();
        let leaves_now = self.through(item, pipe.item.clone(), stages, &mut conts, sl, f, init, e, collect);
        // CONT: the next item in the list. (Nothing reaches it when the
        // first item to arrive always leaves and none is skipped.)
        if !(leaves_now && conts.is_empty()) {
            let cont = self.pc();
            for c in conts {
                self.patch(c, cont);
            }
            self.incr(i);
            self.op(Op::Jump(head as u32));
        }
        let end = self.pc();
        self.patch(exit, end);
        self.release(mark);
    }

    /// An item (at `item`, typed `item_ty`) through `stages`, then the sink.
    /// A filter or window that drops it jumps to `conts` (the next item).
    /// At a `flatten`, the item is a list: a loop over it runs the rest of
    /// the stages and the sink, nested. Whether the sink always leaves.
    #[allow(clippy::too_many_arguments)]
    fn through(&mut self, mut item: Place, mut item_ty: Ty, stages: &[Down], conts: &mut Vec<usize>, sl: &SinkLocals, f: Option<&TExpr>, init: Option<&TExpr>, e: &TExpr, collect: Option<&Collect>) -> bool {
        for (j, d) in stages.iter().enumerate() {
            let (stage, level) = match d {
                Down::Convert(from, to, tmp) => {
                    self.load_as(item, from, to);
                    self.store_into(*tmp, to);
                    item = Place { root: Root::Local(*tmp), off: 0 };
                    item_ty = to.clone();
                    continue;
                }
                Down::Node(s, level) => (*s, *level),
            };
            let st = stage.node();
            match stage {
                Stage::Filter { pred, .. } => {
                    self.in_ctx(pred, Ctx { item, index: None, acc: None }, None);
                    conts.push(self.jump(Op::JumpIfFalse, pred));
                }
                Stage::Map(_) => {
                    let (TKind::Map { body, .. }, StageState::Map { tmp, counter, down }) = (&st.kind, self.stages[&(st as *const TExpr)].clone()) else {
                        unreachable!()
                    };
                    self.in_ctx(body, Ctx { item, index: counter, acc: None }, None);
                    self.store_into(tmp, &body.ty);
                    if let Some(c) = counter {
                        self.next_index(c, down);
                    }
                    item = Place { root: Root::Local(tmp), off: 0 };
                    item_ty = body.ty.clone();
                }
                Stage::Window(_) => match self.stages[&(st as *const TExpr)].clone() {
                    // Folded into the loop's bounds.
                    StageState::Window { counter: None, .. } => {}
                    // Before the window: skip the item. Past it: no item of this
                    // pipeline will pass again.
                    StageState::Window { counter: Some(c), start, end } => {
                        if let Some(s) = start {
                            self.op(Op::Load(c));
                            self.op(Op::Load(s));
                            self.op(Op::Lt);
                            let inside = self.jump(Op::JumpIfFalse, st);
                            self.incr(c);
                            conts.push(self.jump(Op::Jump, st));
                            let here = self.pc();
                            self.patch(inside, here);
                        }
                        if let Some(end) = end {
                            self.op(Op::Load(c));
                            self.op(Op::Load(end));
                            self.op(Op::Lt);
                            let past = self.jump(Op::JumpIfFalse, st);
                            self.levels[level].push(past);
                        }
                        self.incr(c);
                    }
                    other => unreachable!("window {other:?}"),
                },
                // The loop already runs backwards.
                Stage::Reverse(_) => {}
                // The item is a list: its handle, looped over.
                Stage::Flatten(_) => {
                    let mark = self.mark();
                    self.load_leaf(item);
                    let h = self.store_new(1);
                    let mut inner = Pipeline::new(Source::List(st), plan::item_of(&item_ty));
                    inner.pre = Some(h);
                    self.nested(inner, &stages[j + 1..], sl, f, init, e, collect);
                    self.release(mark);
                    return false;
                }
                // The map's body is a pipeline: it runs here, per item, with
                // `#` (and `#index`) the item's.
                Stage::FlatMap(_) => {
                    let StageState::Map { counter, down, .. } = self.stages[&(st as *const TExpr)].clone() else { unreachable!() };
                    let (lets, body) = plan::flat_map_body(st).unwrap();
                    let mark = self.mark();
                    self.ctx.push(Ctx { item, index: counter, acc: None });
                    for l in lets {
                        let TKind::Let { id, value, .. } = &l.kind else { unreachable!() };
                        self.emit(value);
                        let base = self.store_new(leaves(&value.ty));
                        self.bindings.insert(*id, Root::Local(base));
                    }
                    self.nested(plan::for_write(body), &stages[j + 1..], sl, f, init, e, collect);
                    self.ctx.pop();
                    self.release(mark);
                    if let Some(c) = counter {
                        self.next_index(c, down);
                    }
                    return false;
                }
            }
        }
        self.take_item(sl, f, init, e, collect, item, &item_ty)
    }

    /// A pipeline nested in an item (`flatten`): its hold and stages set up
    /// for this item, then its loops, each running `rest` and the sink.
    #[allow(clippy::too_many_arguments)]
    fn nested(&mut self, mut inner: Pipeline, rest: &[Down], sl: &SinkLocals, f: Option<&TExpr>, init: Option<&TExpr>, e: &TExpr, collect: Option<&Collect>) {
        self.resolve_hold(&mut inner);
        if let Source::Range(r) = inner.source
            && inner.pre.is_none()
        {
            inner.pre = Some(self.range_bounds(r));
        }
        self.init_stages(&mut inner, reversed(rest));
        let sub = self.new_level();
        self.loops(&inner, sub, rest, sl, f, init, e, collect);
    }

    /// `#index` for the next item: one more, or one less when the items
    /// arrive backwards.
    fn next_index(&mut self, c: u16, down: bool) {
        if !down {
            return self.incr(c);
        }
        self.op(Op::Load(c));
        self.op(Op::PushInt(1));
        self.op(Op::Sub);
        self.op(Op::Store(c));
    }

    /// What the sink does with an item that reached it (at `item`, typed
    /// `item_ty`). Leaving early (`find`, `any`, …) jumps past every loop.
    /// Whether it always leaves (`first` with no predicate).
    #[allow(clippy::too_many_arguments)]
    fn take_item(&mut self, sl: &SinkLocals, f: Option<&TExpr>, init: Option<&TExpr>, e: &TExpr, collect: Option<&Collect>, item: Place, item_ty: &Ty) -> bool {
        let (sink, r, aux, pos, target, top) = (sl.sink, sl.r, sl.aux, sl.pos, sl.target, sl.top);
        let acc_ty = &sl.acc_ty;
        let pred_ctx = Ctx { item, index: None, acc: None };
        if let Some(c) = collect {
            let stride = leaves(&c.item);
            match c.mode {
                // Into the block as item r, converted to the block's layout.
                HoldKind::Plain => {
                    self.floats = true;
                    self.load_as(item, item_ty, &c.item);
                    self.floats = false;
                    self.op(Op::Load(r));
                    self.op(Op::ListStore { base: c.base, cap: c.cap, stride });
                    self.incr(r);
                }
                // The item, its key unless the key is one of its leaves, and
                // the block's state (its count): the VM inserts it in order.
                HoldKind::Sorted { key, desc, tail } => {
                    let at = key_leaf(key, item_ty, &c.item);
                    self.floats = true;
                    self.load_as(item, item_ty, &c.item);
                    self.floats = false;
                    if at.is_none() {
                        match key {
                            Some(k) => self.in_ctx(k, Ctx { item, index: None, acc: None }, None),
                            None => self.load_as(item, item_ty, item_ty),
                        }
                    }
                    self.op(Op::Load(r));
                    let mode = bytecode::hold_mode(desc, tail, false, at);
                    self.op_at(Op::HoldInsert { base: c.base, cap: c.cap, stride, mode }, e);
                    self.op(Op::Store(r));
                }
                // A value not in the block yet goes in at the end; once the
                // block is full, no other item matters.
                HoldKind::Unique => {
                    let mark = self.mark();
                    let (h, j) = (self.alloc(1), self.alloc(1));
                    self.op(Op::Load(r));
                    self.op(Op::ListMake { base: c.base, cap: c.cap, stride: 1 });
                    self.op(Op::Store(h));
                    self.op(Op::PushInt(0));
                    self.op(Op::Store(j));
                    let head = self.pc();
                    self.op(Op::Load(j));
                    self.op(Op::Load(r));
                    self.op(Op::Lt);
                    let new = self.jump(Op::JumpIfFalse, e);
                    self.op(Op::Load(h));
                    self.op(Op::Load(j));
                    self.op(Op::ListRead(0));
                    self.load_as(item, item_ty, item_ty);
                    self.op(Op::Eq);
                    let differs = self.jump(Op::JumpIfFalse, e);
                    let seen = self.jump(Op::Jump, e);
                    let here = self.pc();
                    self.patch(differs, here);
                    self.incr(j);
                    self.op(Op::Jump(head));
                    let here = self.pc();
                    self.patch(new, here);
                    self.floats = true;
                    self.load_as(item, item_ty, &c.item);
                    self.floats = false;
                    self.op(Op::Load(r));
                    self.op(Op::ListStore { base: c.base, cap: c.cap, stride });
                    self.incr(r);
                    self.op(Op::Load(r));
                    self.op(Op::PushInt(c.cap as i64));
                    self.op(Op::Lt);
                    let room = self.jump(Op::JumpIfFalse, e);
                    self.levels[top].push(room);
                    let here = self.pc();
                    self.patch(seen, here);
                    self.release(mark);
                }
            }
            return false;
        }
        let mut breaks = Vec::new();
        // `first` with no predicate always leaves after the first item to
        // reach it, so nothing follows.
        let always_leaves = sink == Sink::First && f.is_none();
        match sink {
            Sink::Len => self.incr(r),
            // As a float, into the block in order (NaN first, as Go's
            // `sort.Float64s` puts it).
            Sink::Median => {
                let (base, cap) = sl.block.unwrap();
                self.load_as(item, item_ty, item_ty);
                self.op_at(Op::Call(bytecode::Builtin::Float, 1), e);
                self.op(Op::Load(r));
                let mode = bytecode::hold_mode(false, false, true, Some(0));
                self.op_at(Op::HoldInsert { base, cap, stride: 1, mode }, e);
                self.op(Op::Store(r));
            }
            Sink::Count | Sink::One => {
                match f {
                    Some(p) => self.in_ctx(p, pred_ctx.clone(), None),
                    None => self.load_as(item, item_ty, item_ty),
                }
                let skip = self.jump(Op::JumpIfFalse, e);
                self.incr(r);
                let here = self.pc();
                self.patch(skip, here);
            }
            Sink::Sum | Sink::Mean => {
                self.op(Op::Load(r));
                match f {
                    Some(v) => self.in_ctx(v, pred_ctx.clone(), None),
                    None => self.load_as(item, item_ty, item_ty),
                }
                self.op_at(Op::Add, e);
                self.op(Op::Store(r));
                if sink == Sink::Mean {
                    self.incr(aux);
                }
            }
            Sink::Min | Sink::Max => {
                // The first item wins until a later one is strictly better
                // (so a NaN in first place sticks, and later NaNs are skipped).
                self.op(Op::Load(r));
                self.op(Op::PushNil);
                self.op(Op::Eq);
                let cmp = self.jump(Op::JumpIfFalse, e);
                let set = self.jump(Op::Jump, e);
                let here = self.pc();
                self.patch(cmp, here);
                self.load_as(item, item_ty, item_ty);
                self.op(Op::Load(r));
                self.op_at(if sink == Sink::Min { Op::Lt } else { Op::Gt }, e);
                let skip = self.jump(Op::JumpIfFalse, e);
                let here = self.pc();
                self.patch(set, here);
                self.load_as(item, item_ty, item_ty);
                self.op(Op::Store(r));
                let here = self.pc();
                self.patch(skip, here);
            }
            Sink::Any | Sink::All | Sink::NoneOf => {
                self.in_ctx(f.unwrap(), pred_ctx.clone(), None);
                if sink == Sink::All {
                    self.op(Op::Not);
                }
                let skip = self.jump(Op::JumpIfFalse, e);
                self.op(if sink == Sink::Any { Op::PushTrue } else { Op::PushFalse });
                self.op(Op::Store(r));
                breaks.push(self.jump(Op::Jump, e));
                let here = self.pc();
                self.patch(skip, here);
            }
            Sink::In => {
                self.load_as(item, item_ty, item_ty);
                self.op(Op::Load(aux));
                self.op(Op::Eq);
                let skip = self.jump(Op::JumpIfFalse, e);
                self.op(Op::PushTrue);
                self.op(Op::Store(r));
                breaks.push(self.jump(Op::Jump, e));
                let here = self.pc();
                self.patch(skip, here);
            }
            Sink::Find | Sink::FindLast | Sink::First | Sink::Last => {
                let skip = match f {
                    Some(p) => {
                        self.in_ctx(p, pred_ctx.clone(), None);
                        Some(self.jump(Op::JumpIfFalse, e))
                    }
                    None => None,
                };
                self.load_as(item, item_ty, &e.ty);
                self.store_into(r, &e.ty);
                if matches!(sink, Sink::Find | Sink::First) || sl.backward.is_some() {
                    breaks.push(self.jump(Op::Jump, e));
                }
                if let Some(s) = skip {
                    let here = self.pc();
                    self.patch(s, here);
                }
            }
            Sink::FindIndex | Sink::FindLastIndex => {
                self.in_ctx(f.unwrap(), pred_ctx.clone(), None);
                let skip = self.jump(Op::JumpIfFalse, e);
                match sl.backward {
                    // From the end: the count less one, less the items passed.
                    Some(last) => {
                        self.op(Op::Load(last));
                        self.op(Op::Load(pos));
                        self.op(Op::Sub);
                    }
                    None => self.op(Op::Load(pos)),
                }
                self.op(Op::Store(r));
                if sink == Sink::FindIndex || sl.backward.is_some() {
                    breaks.push(self.jump(Op::Jump, e));
                }
                let here = self.pc();
                self.patch(skip, here);
            }
            Sink::At | Sink::Get => {
                self.op(Op::Load(pos));
                self.op(Op::Load(target));
                self.op(Op::Eq);
                let skip = self.jump(Op::JumpIfFalse, e);
                self.load_as(item, item_ty, &e.ty);
                self.store_into(r, &e.ty);
                self.op(Op::PushTrue);
                self.op(Op::Store(aux));
                breaks.push(self.jump(Op::Jump, e));
                let here = self.pc();
                self.patch(skip, here);
            }
            Sink::Reduce => {
                let body = |m: &mut Self| {
                    m.in_ctx(f.unwrap(), Ctx { item, index: Some(pos), acc: Some((r, acc_ty.clone())) }, Some(acc_ty));
                    // The new value is complete on the stack before it replaces the old.
                    m.store_into(r, acc_ty);
                };
                if init.is_some() {
                    body(self);
                } else {
                    // Without an initial value, the first item is the accumulator.
                    self.op(Op::Load(aux));
                    let first = self.jump(Op::JumpIfFalse, e);
                    body(self);
                    let done = self.jump(Op::Jump, e);
                    let here = self.pc();
                    self.patch(first, here);
                    self.load_as(item, item_ty, acc_ty);
                    self.store_into(r, acc_ty);
                    self.op(Op::PushTrue);
                    self.op(Op::Store(aux));
                    let here = self.pc();
                    self.patch(done, here);
                }
            }
        }
        // The item reached the sink: count it. (Filtered-out items jump past this.)
        if !always_leaves && pos != u16::MAX {
            self.incr(pos);
        }
        self.levels[top].extend(breaks);
        always_leaves
    }

    /// Joins the early exits (each left the result on the stack) and gives
    /// back the sink's locals.
    fn finish(&mut self, mark: u16, end_jumps: Vec<usize>) {
        let here = self.pc();
        for j in end_jumps {
            self.patch(j, here);
        }
        self.release(mark);
    }

    // c ? a : b:  c; JumpIfFalse B; a; Jump E; B: b; E:
    fn cond(&mut self, c: &TExpr, a: &TExpr, b: &TExpr, e: &TExpr, dst: &Ty) {
        self.emit(c);
        let else_ = self.jump(Op::JumpIfFalse, c);
        self.emit_as(a, dst);
        let end = self.jump(Op::Jump, e);
        self.depth -= leaves(dst);
        let b_pc = self.pc();
        self.emit_as(b, dst);
        let e_pc = self.pc();
        self.patch(else_, b_pc);
        self.patch(end, e_pc);
    }

    fn let_(&mut self, id: u32, value: &TExpr, body: &TExpr, dst: &Ty) {
        // The body's value starts with the binding, laid out as it is (e.g.
        // `let r = …; {a: r, b: r.x}`): build the binding there, on the
        // stack, rather than in locals and then copied out. Nothing in the
        // body pops below it, so it stays put for the body's other reads.
        if let Some(at) = lead(body, dst, id, &value.ty) {
            let base = self.depth;
            self.emit(value);
            self.bindings.insert(id, Root::Stack(base));
            self.placed.insert(at as *const TExpr);
            self.emit_as(body, dst);
            return;
        }
        self.emit(value);
        let n = leaves(&value.ty);
        let base = self.store_new(n);
        self.bindings.insert(id, Root::Local(base));
        self.emit_as(body, dst);
        self.free(n);
    }

    fn coalesce(&mut self, a: &TExpr, b: &TExpr, e: &TExpr, dst: &Ty) {
        match &a.ty {
            // Always nil: only `b` matters, but Go evaluates `a` first.
            Ty::Nil => {
                self.evaluate_for_failure(a);
                self.emit_as(b, dst)
            }
            t if !t.may_be_nil() => self.emit_as(a, dst),
            // A scalar:  a; Dup; JumpIfNotNil E; Pop; b; E:
            _ if leaves(&a.ty) == 1 => {
                self.emit(a);
                self.op(Op::Dup);
                let end = self.jump(Op::JumpIfNotNil, e);
                self.op(Op::Pop);
                self.emit_as(b, dst);
                let e_pc = self.pc();
                self.patch(end, e_pc);
            }
            // An optional record:  presence; JumpIfNotNil A; b; Jump E; A: a's fields; E:
            Ty::Opt(inner) => {
                let inner = (**inner).clone();
                self.with_place(a, |m, p| {
                    m.load_leaf(p);
                    let have = m.jump(Op::JumpIfNotNil, e);
                    m.emit_as(b, dst);
                    let end = m.jump(Op::Jump, e);
                    m.depth -= leaves(dst);
                    let a_pc = m.pc();
                    m.load_as(p.at(1), &inner, dst);
                    let e_pc = m.pc();
                    m.patch(have, a_pc);
                    m.patch(end, e_pc);
                });
            }
            other => unreachable!("?? on {other:?}"),
        }
    }

    /// Member steps (spec §2.2): `.` on a nil record fails; a `?.` whose
    /// target is nil makes the rest of the chain nil. Where narrowing showed
    /// a record isn't nil (`nonnil`), there is no check: its fields are read
    /// past its presence leaf. With `presence`, the value is an optional
    /// record and only its presence leaf is pushed (for a nil test).
    fn path(&mut self, base: &TExpr, steps: &[Step], nonnil: &[bool], e: &TExpr, presence: bool) {
        self.with_place(base, |m, p| {
            let mut cur = base.ty.clone();
            let mut off = 0u16;
            let mut exits = Vec::new();
            for (i, s) in steps.iter().enumerate() {
                let (fields, rec_off) = match &cur {
                    Ty::Rec(f, _) => (f.clone(), off),
                    Ty::Opt(inner) if nonnil[i] => {
                        let Ty::Rec(f, _) = &**inner else { unreachable!() };
                        (f.clone(), off + 1)
                    }
                    Ty::Opt(inner) => {
                        let Ty::Rec(f, _) = &**inner else { unreachable!() };
                        m.load_leaf(p.at(off));
                        if s.optional {
                            // Present: carry on. Absent: the whole chain is nil.
                            let have = m.jump(Op::JumpIfNotNil, e);
                            exits.push(m.jump(Op::Jump, e));
                            let here = m.pc();
                            m.patch(have, here);
                        } else {
                            m.spans.push((m.pc(), s.span.start, s.span.end));
                            m.op(Op::AssertNotNil);
                        }
                        (f.clone(), off + 1)
                    }
                    other => unreachable!("step on {other:?}"),
                };
                off = rec_off + field_offset(&fields, &s.field);
                cur = fields[&s.field].clone();
            }
            let (cur, off) = strip(&cur, off, *nonnil.last().unwrap());
            let n = if presence {
                match cur {
                    Ty::Opt(_) => m.load_leaf(p.at(off)),
                    // Present unless a `?.` before it short-circuited.
                    _ => m.op(Op::PushTrue),
                }
                1
            } else {
                m.load_as(p.at(off), &cur, &e.ty);
                leaves(&e.ty)
            };
            if !exits.is_empty() {
                let end = m.jump(Op::Jump, e);
                m.depth -= n;
                let nil_pc = m.pc();
                m.push_nils(n);
                let e_pc = m.pc();
                for x in exits {
                    m.patch(x, nil_pc);
                }
                m.patch(end, e_pc);
            }
        });
    }
}

/// Follows a path's field reads from a value stored as `t`: the stored type
/// reached and its leaf offset, or `None` if a read needs a nil check (an
/// optional record that isn't narrowed).
fn walk(t: &Ty, steps: &[Step], nonnil: &[bool]) -> Option<(Ty, u16)> {
    let mut cur = t.clone();
    let mut off = 0u16;
    for (i, s) in steps.iter().enumerate() {
        let (fields, base) = match &cur {
            Ty::Rec(f, _) => (f.clone(), off),
            Ty::Opt(inner) if nonnil[i] => match &**inner {
                Ty::Rec(f, _) => (f.clone(), off + 1),
                _ => return None,
            },
            _ => return None,
        };
        off = base + field_offset(&fields, &s.field);
        cur = fields.get(&s.field)?.clone();
    }
    Some((cur, off))
}

/// The read of binding `id` (whose value is typed `vty`) that `e`, emitted
/// as `dst`, pushes first, before anything else: `e` is that read, laid out
/// as it is, or a record literal whose first field (in `dst`'s order) is,
/// or a `let` whose body is. Then the binding can be built right there.
fn lead<'e>(e: &'e TExpr, dst: &Ty, id: u32, vty: &Ty) -> Option<&'e TExpr> {
    match &e.kind {
        TKind::Local(x) if *x == id && same_layout(vty, dst) && same_layout(&e.ty, dst) => Some(e),
        TKind::Let { body, .. } => lead(body, dst, id, vty),
        // An optional record would push its presence leaf first.
        TKind::Record(fields) => {
            let Ty::Rec(df, _) = dst else { return None };
            let (k, dt) = df.iter().next()?;
            let (_, fe) = fields.iter().find(|(f, _)| f == k)?;
            lead(fe, dt, id, vty)
        }
        _ => None,
    }
}

/// A value stored as `t` at leaf `off`, read as not nil when `nonnil`: an
/// optional record's fields start after its presence leaf; an optional
/// scalar is the same leaf.
fn strip(t: &Ty, off: u16, nonnil: bool) -> (Ty, u16) {
    match t {
        Ty::Opt(inner) if nonnil => match &**inner {
            Ty::Rec(..) => ((**inner).clone(), off + 1),
            _ => ((**inner).clone(), off),
        },
        _ => (t.clone(), off),
    }
}

fn binop(op: BinOp) -> Op {
    match op {
        BinOp::Add => Op::Add,
        BinOp::Sub => Op::Sub,
        BinOp::Mul => Op::Mul,
        BinOp::Div => Op::Div,
        BinOp::Mod => Op::Mod,
        BinOp::Pow => Op::Pow,
        BinOp::Eq => Op::Eq,
        BinOp::Ne => Op::Ne,
        BinOp::Lt => Op::Lt,
        BinOp::Le => Op::Le,
        BinOp::Gt => Op::Gt,
        BinOp::Ge => Op::Ge,
        BinOp::And | BinOp::Or | BinOp::Coalesce | BinOp::In => unreachable!("lowered to jumps or a sink"),
    }
}

/// The `E-SKIPPED` diagnostic for a skip (plan.rs) whose expression can
/// fail (`fail`, its first failure): what isn't evaluated as in Go, why,
/// and a fix that works.
fn skip_diag(s: &plan::Skip, fail: &crate::check::Failure, src: &str) -> Diag {
    use plan::Cause;
    let snip = |e: &TExpr| crate::diag::snippet(src, e.span);
    let what = s.role.replace("{}", &snip(s.node));
    let total = if fail.host {
        "take the host builtin's call out of it (host builtins are treated as able to fail, spec §2.4)"
    } else {
        "make it total (e.g. `a % max(b, 1)`, `(…) ?? 0`, a test that avoids the failing case)"
    };
    let (cause, help) = match s.cause {
        Cause::Stops(sink) => {
            let at = match sink {
                Sink::First => "`first` stops at the first item".to_string(),
                Sink::At => "indexing stops at the item it reads".to_string(),
                Sink::Get => "`get` stops at the item it reads".to_string(),
                Sink::In => "`in` stops at the first item equal to what it looks for".to_string(),
                Sink::FindLast | Sink::FindLastIndex => format!("`{}` runs backwards and stops at the last match", sink.name()),
                _ => format!("`{}` stops at the first item that decides it", sink.name()),
            };
            let fix = match sink {
                Sink::First | Sink::At | Sink::Get => format!("{total}, or read the item first and apply the body to it alone (`let x = xs[i]; …` rather than `map(xs, …)[i]`)"),
                Sink::In => format!("{total}, or test the items themselves: `any(xs, f(#) == x)` rather than `x in map(xs, f(#))` (Go stops that early too)"),
                Sink::FindLast | Sink::FindLastIndex => total.to_string(),
                _ => format!(
                    "{total}, or put the body in `{n}`'s own predicate: `{n}(xs, f(#) > 0)` rather than `{n}(map(xs, f(#)), # > 0)` (Go stops that early too)",
                    n = sink.name()
                ),
            };
            (at, fix)
        }
        Cause::UniqFull(u) => (format!("`{}` stops once it has the items it needs", snip(u)), total.to_string()),
        Cause::Window(w) => (
            format!("`{}` leaves the loop at its end", snip(w)),
            format!("{total}, or slice the list before the body (`map(take(xs, k), …)` rather than `take(map(xs, …), k)`)"),
        ),
        Cause::Dropped(sort, sink) => {
            let by = sink.map_or(String::new(), |k| format!(" (`{}` doesn't depend on the order)", k.name()));
            (format!("the sort `{}` is left out{by}, so its keys are never evaluated", snip(sort)), format!("{total}, or leave the sort out yourself: it doesn't change the result"))
        }
        Cause::Filtered(sort) => (
            format!("the filter goes under the sort `{}`, which then evaluates the key only for the items that pass it", snip(sort)),
            format!("{total}, or filter before sorting (`first(sortBy(filter(xs, p), k))` rather than `find(sortBy(xs, k), p)` or `first(filter(sortBy(xs, k), p))`): the same item"),
        ),
        Cause::EveryItem(sink) => {
            let (how, fix) = match sink {
                Sink::Find => (
                    "goes under the sort (to keep one item) and runs for every item, where Go stops at the first match in sorted order (the last, for `findLast`)",
                    format!("{total}, or filter before sorting (`first(sortBy(filter(xs, p), k))`): the same item"),
                ),
                _ => (
                    "runs forwards over every item here (the list's stages can fail, or can't be walked from the end), where Go runs it backwards and stops at the last match",
                    format!("{total}, or make the `map` and `filter` bodies before it total, so it can run backwards"),
                ),
            };
            let d = Diag::error("E-SKIPPED", s.node.span, format!("{what} {how}, and it can fail: {}", fail.describe(src)));
            return d.help(fix);
        }
    };
    // A list that isn't built by a list builtin, bound with `let`, is
    // evaluated where the `let` is (a list built by one is re-run at each use).
    let help = if s.role == plan::CONCAT_LIST {
        format!("{total}, or bind it with `let` first (`let ys = …; concat(xs, ys)`): it is then evaluated where the `let` is")
    } else {
        help
    };
    Diag::error(
        "E-SKIPPED",
        s.node.span,
        format!("{what} isn't evaluated for every item here: {cause}; Go evaluates it for every item, and it can fail: {}", fail.describe(src)),
    )
    .help(help)
}

/// Whether `e` reads `#index` of the predicate at `depth` (not a nested one's).
fn uses_index(e: &TExpr, depth: usize) -> bool {
    match &e.kind {
        TKind::Pointer { which: Ptr::Index, depth: d } => *d == depth,
        _ => children(e).into_iter().any(|c| uses_index(c, depth)),
    }
}

/// Whether a loop running these stages runs backwards: an odd number of
/// `reverse`s among them.
fn reversed(stages: &[Down]) -> bool {
    stages.iter().filter(|d| matches!(d, Down::Node(Stage::Reverse(_), _))).count() % 2 == 1
}

/// A block of locals reserved for the whole evaluation, for the diagnostics.
struct Held {
    span: Span,
    bound: u32,
    /// Slots per item (a sorted block's separate key included).
    stride: u16,
    slots: u32,
    why: HoldWhy,
}

/// Why a block is held.
#[derive(Clone, Copy)]
enum HoldWhy {
    /// A list literal, or a list kept as a value.
    Value,
    Sort,
    Unique,
    Median,
    /// A list reversed again.
    Reverse,
}

impl HoldWhy {
    fn of(k: &HoldKind) -> HoldWhy {
        match k {
            HoldKind::Sorted { .. } => HoldWhy::Sort,
            HoldKind::Unique => HoldWhy::Unique,
            HoldKind::Plain => HoldWhy::Reverse,
        }
    }

    /// For `E-HOLD`: what is held, and the fix.
    fn words(self) -> (&'static str, &'static str) {
        match self {
            HoldWhy::Value => (
                "this list is held",
                "slice it first (`(…)[-N:]`, `take(…, N)`), or keep only what you need (`first`, `last`, `sum`, `count`, …)",
            ),
            HoldWhy::Sort => (
                "this sort holds its whole input",
                "take only what you need of it (`first(sortBy(…))`, `last(…)`, `take(sortBy(…), k)`, `sortBy(…)[-k:]`: k items), aggregate it (`sum`, `count`, `max`, …: nothing held), or slice its input first (`xs[-N:]`)",
            ),
            HoldWhy::Unique => (
                "`uniq` holds the values it has kept",
                "take only what you need of it (`take(uniq(…), k)`: k items), or slice its input first (`xs[-N:]`)",
            ),
            HoldWhy::Median => ("`median` holds every value", "slice its input first (`xs[-N:]`)"),
            HoldWhy::Reverse => ("this list is held to be reversed again", "reverse it once: `reverse(reverse(xs))` is `xs`"),
        }
    }
}

/// The slots a sorted block keeps per item beside the item's own: 1 for
/// its key, unless the key is one of the item's leaves.
fn key_slots(kind: &HoldKind, item: &Ty, block: &Ty) -> u16 {
    match kind {
        HoldKind::Sorted { key, .. } if key_leaf(*key, item, block).is_none() => 1,
        _ => 0,
    }
}

/// The leaf of a block item that is its sort key, when the key is the item
/// itself (`sort`) or a field path into it (`sortBy(xs, .t)`) to a number
/// that can't be nil, and the block keeps the item's own layout.
fn key_leaf(key: Option<&TExpr>, item: &Ty, block: &Ty) -> Option<u16> {
    if !same_layout(item, block) || int_to_float(item, block) {
        return None;
    }
    let leaf = match key {
        None => matches!(item, Ty::Int(_) | Ty::Float).then_some(0)?,
        Some(k) => match &k.kind {
            TKind::Path { base, steps, nonnil } if matches!(base.kind, TKind::Pointer { which: Ptr::Item, .. }) => {
                let (t, off) = walk(&base.ty, steps, nonnil)?;
                let (t, off) = strip(&t, off, *nonnil.last().unwrap());
                matches!(t, Ty::Int(_) | Ty::Float).then_some(off)?
            }
            _ => return None,
        },
    };
    (leaf < bytecode::HOLD_KEY_MAX).then_some(leaf)
}
