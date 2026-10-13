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

use crate::check::{Ptr, Sink, TExpr, TKind, children};
use crate::schema::{GroupCheck, Interface, LeafCheck, group_checks, leaf_checks};
use crate::syntax::{BinOp, Step, UnOp};
use crate::types::{Ty, field_offset, leaves, same_layout};
use bytecode::{Check, Import, Kind, MAX_OP_LEN, Op};
use std::collections::{BTreeMap, HashMap, HashSet};

pub struct Emitted {
    pub bytes: Vec<u8>,
    pub max_stack: u16,
    pub locals: u16,
    pub output_leaves: u16,
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
    /// Each list literal's own block of locals, reserved for the whole
    /// evaluation (so a handle to it can be copied freely).
    perm: HashMap<*const TExpr, u16>,
    perm_next: u16,
}

pub fn emit(e: &TExpr, iface: &Interface) -> Emitted {
    let mut m = Emitter {
        iface,
        code: Vec::new(),
        consts: Vec::new(),
        strings: HashMap::new(),
        spans: Vec::new(),
        depth: 0,
        max_depth: 0,
        next_local: 0,
        max_locals: 0,
        bindings: HashMap::new(),
        placed: HashSet::new(),
        imports: Vec::new(),
        ctx: Vec::new(),
        list_leaf: iface.lists.iter().enumerate().map(|(id, l)| (l.leaf, id as u16)).collect(),
        perm: HashMap::new(),
        perm_next: 0,
    };
    // List literals' blocks come first; everything else is allocated above them.
    let perm = literal_slots(e);
    m.next_local = perm;
    m.max_locals = perm;
    let mut ins = Vec::new();
    for i in &iface.inputs {
        leaf_checks(&i.ty, false, &mut ins);
    }
    let mut outs = Vec::new();
    leaf_checks(&iface.output, false, &mut outs);
    let inputs: Vec<Check> = ins.iter().map(|c| m.check(c)).collect();
    let outputs: Vec<Check> = outs.iter().map(|c| m.check(c)).collect();
    m.emit_as(e, &iface.output);
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
    Emitted { bytes, max_stack: m.max_depth, locals: m.max_locals, output_leaves: n }
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
        if same_layout(src, dst) {
            for i in 0..leaves(src) {
                self.load_leaf(p.at(i));
            }
            return;
        }
        match (src, dst) {
            (Ty::Nil, _) => self.push_nils(leaves(dst)),
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
        if same_layout(&e.ty, dst) {
            return self.emit(e);
        }
        match &e.kind {
            TKind::Nil => self.push_nils(leaves(dst)),
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
                let key = e as *const TExpr;
                let base = match self.perm.get(&key) {
                    Some(&b) => b,
                    None => {
                        let b = self.perm_next;
                        self.perm_next += stride * items.len() as u16;
                        self.perm.insert(key, b);
                        b
                    }
                };
                for (k, it) in items.iter().enumerate() {
                    self.emit_as(it, item);
                    self.store_into(base + k as u16 * stride, item);
                }
                self.op(Op::MakeList { base, len: items.len() as u16, stride });
            }
            TKind::Map { .. } | TKind::Filter { .. } => unreachable!("a pipeline is consumed by a sink, never emitted alone"),
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
                    // A record that can't be nil.
                    self.op(if *negate { Op::PushTrue } else { Op::PushFalse });
                }
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
    /// anything over `map`/`filter`) is one loop: the source's handle,
    /// length and position in locals, the stages inlined, and the sink's
    /// state in locals. Nothing is held: a stage's value lives only for the
    /// item being processed.
    fn sink(&mut self, sink: Sink, src: &TExpr, f: Option<&TExpr>, init: Option<&TExpr>, e: &TExpr) {
        let mark = self.mark();
        let (source, stages) = pipeline(src);
        let Ty::List(item0, _) = source.ty.non_nil().unwrap_or(Ty::Unknown) else { unreachable!("sink over {:?}", source.ty) };
        let item0 = *item0;
        let out = leaves(&e.ty);

        // The handle.
        self.emit(source);
        let h = self.store_new(1);
        let mut end_jumps = Vec::new();
        // These give nil (or false) for a nil list; the rest fail, as in expr.
        if matches!(sink, Sink::First | Sink::Last | Sink::Get | Sink::In) {
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

        // Fast paths straight on a list.
        if stages.is_empty() {
            match sink {
                Sink::Len => {
                    self.op(Op::Load(h));
                    self.op_at(Op::ListLen, e);
                    return self.finish(mark, end_jumps);
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
                        self.load_as(p, &item0, &e.ty);
                        end_jumps.push(self.jump(Op::Jump, e));
                        self.depth -= out;
                        let here = self.pc();
                        self.patch(inside, here);
                        self.push_nils(out);
                    } else {
                        // The VM checks the range (R-INDEX).
                        self.spans.push((self.pc(), e.span.start, e.span.end));
                        let p = Place { root: Root::Item { h, i: k }, off: 0 };
                        self.load_as(p, &item0, &e.ty);
                    }
                    return self.finish(mark, end_jumps);
                }
                _ => {}
            }
        }

        // The loop's locals: length, position in the source, and for the
        // sink, how many items have reached it.
        let n = self.alloc(1);
        self.op(Op::Load(h));
        self.op_at(Op::ListLen, e);
        self.op(Op::Store(n));
        let i = self.alloc(1);
        self.op(Op::PushInt(0));
        self.op(Op::Store(i));
        // How many items have reached the sink: only where a position is read.
        let pos = if matches!(sink, Sink::FindIndex | Sink::FindLastIndex | Sink::At | Sink::Get | Sink::Reduce) {
            let p = self.alloc(1);
            self.op(Op::PushInt(0));
            self.op(Op::Store(p));
            p
        } else {
            u16::MAX
        };
        // Each map stage's value, and its #index counter if its body reads it.
        let depth = self.ctx.len();
        let mut stage_slots = Vec::new();
        for st in &stages {
            if let TKind::Map { body, .. } = &st.kind {
                let tmp = self.alloc(leaves(&body.ty));
                let counter = if uses_index(body, depth) {
                    let c = self.alloc(1);
                    self.op(Op::PushInt(0));
                    self.op(Op::Store(c));
                    Some(c)
                } else {
                    None
                };
                stage_slots.push(Some((tmp, counter)));
            } else {
                stage_slots.push(None);
            }
        }

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
        match sink {
            Sink::Len | Sink::Count | Sink::One => {
                self.op(Op::PushInt(0));
                self.op(Op::Store(r));
            }
            Sink::Sum => {
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
                // The same pipeline, counted (by reference: its literals keep their blocks).
                let as_int = TExpr { kind: TKind::Nil, ty: Ty::Int(crate::types::Range::FULL), span: e.span };
                self.op(Op::Load(target));
                self.sink(Sink::Len, src, None, None, &as_int);
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

        // L: while i < n
        let head = self.pc();
        self.op(Op::Load(i));
        self.op(Op::Load(n));
        self.op(Op::Lt);
        let exit = self.jump(Op::JumpIfFalse, e);
        let mut conts = Vec::new();
        let mut breaks = Vec::new();
        let mut item = Place { root: Root::Item { h, i }, off: 0 };
        let mut item_ty = item0.clone();
        for (st, slots) in stages.iter().zip(&stage_slots) {
            match &st.kind {
                TKind::Filter { pred, .. } => {
                    self.in_ctx(pred, Ctx { item, index: None, acc: None }, None);
                    conts.push(self.jump(Op::JumpIfFalse, pred));
                }
                TKind::Map { body, .. } => {
                    let (tmp, counter) = slots.unwrap();
                    self.in_ctx(body, Ctx { item, index: counter, acc: None }, None);
                    self.store_into(tmp, &body.ty);
                    if let Some(c) = counter {
                        self.incr(c);
                    }
                    item = Place { root: Root::Local(tmp), off: 0 };
                    item_ty = body.ty.clone();
                }
                _ => unreachable!(),
            }
        }
        let pred_ctx = Ctx { item, index: None, acc: None };
        // The step. `first` with no predicate always leaves after the first
        // item to reach it, so nothing follows its step.
        let always_leaves = sink == Sink::First && f.is_none();
        match sink {
            Sink::Len => self.incr(r),
            Sink::Count | Sink::One => {
                match f {
                    Some(p) => self.in_ctx(p, pred_ctx.clone(), None),
                    None => self.load_as(item, &item_ty, &item_ty),
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
                    None => self.load_as(item, &item_ty, &item_ty),
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
                self.load_as(item, &item_ty, &item_ty);
                self.op(Op::Load(r));
                self.op_at(if sink == Sink::Min { Op::Lt } else { Op::Gt }, e);
                let skip = self.jump(Op::JumpIfFalse, e);
                let here = self.pc();
                self.patch(set, here);
                self.load_as(item, &item_ty, &item_ty);
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
                self.load_as(item, &item_ty, &item_ty);
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
                self.load_as(item, &item_ty, &e.ty);
                self.store_into(r, &e.ty);
                if matches!(sink, Sink::Find | Sink::First) {
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
                self.op(Op::Load(pos));
                self.op(Op::Store(r));
                if sink == Sink::FindIndex {
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
                self.load_as(item, &item_ty, &e.ty);
                self.store_into(r, &e.ty);
                self.op(Op::PushTrue);
                self.op(Op::Store(aux));
                breaks.push(self.jump(Op::Jump, e));
                let here = self.pc();
                self.patch(skip, here);
            }
            Sink::Reduce => {
                // Without an initial value, the first item is the accumulator.
                self.op(Op::Load(aux));
                let first = self.jump(Op::JumpIfFalse, e);
                self.in_ctx(
                    f.unwrap(),
                    Ctx { item, index: Some(pos), acc: Some((r, acc_ty.clone())) },
                    Some(&acc_ty),
                );
                // The new value is complete on the stack before it replaces the old.
                self.store_into(r, &acc_ty);
                let done = self.jump(Op::Jump, e);
                let here = self.pc();
                self.patch(first, here);
                self.load_as(item, &item_ty, &acc_ty);
                self.store_into(r, &acc_ty);
                self.op(Op::PushTrue);
                self.op(Op::Store(aux));
                let here = self.pc();
                self.patch(done, here);
            }
        }
        // The item reached the sink: count it. (Filtered-out items jump past this.)
        if !always_leaves && pos != u16::MAX {
            self.incr(pos);
        }
        // CONT: the next item in the source.
        let cont = self.pc();
        for c in conts {
            self.patch(c, cont);
        }
        self.incr(i);
        self.op(Op::Jump(head as u32));
        let end = self.pc();
        self.patch(exit, end);
        for b in breaks {
            self.patch(b, end);
        }

        // The result.
        match sink {
            Sink::Len | Sink::Count | Sink::Sum | Sink::Any | Sink::All | Sink::NoneOf | Sink::In => self.op(Op::Load(r)),
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
            Ty::Nil => self.emit_as(b, dst),
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

/// Whether `e` reads `#index` of the predicate at `depth` (not a nested one's).
fn uses_index(e: &TExpr, depth: usize) -> bool {
    match &e.kind {
        TKind::Pointer { which: Ptr::Index, depth: d } => *d == depth,
        _ => children(e).into_iter().any(|c| uses_index(c, depth)),
    }
}

/// The locals every list literal in `e` needs (each its own block).
fn literal_slots(e: &TExpr) -> u16 {
    let own = match (&e.kind, &e.ty) {
        (TKind::ListLit(items), Ty::List(item, _)) if !items.is_empty() && !matches!(**item, Ty::Never) => leaves(item) * items.len() as u16,
        _ => 0,
    };
    own + children(e).into_iter().map(literal_slots).sum::<u16>()
}

/// A sink's source as its underlying list and the `map`/`filter` stages
/// applied to it, innermost first.
fn pipeline(e: &TExpr) -> (&TExpr, Vec<&TExpr>) {
    match &e.kind {
        TKind::Map { src, .. } | TKind::Filter { src, .. } => {
            let (s, mut stages) = pipeline(src);
            stages.push(e);
            (s, stages)
        }
        _ => (e, vec![]),
    }
}
