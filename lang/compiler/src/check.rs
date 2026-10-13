//! Type checking (spec §3), integer ranges (§4.1) and the output check (§3.4).
//!
//! Produces a typed tree ready to emit: names are resolved to inputs or to
//! `let` bindings (the emitter assigns their slots). Keeps going after an
//! error where it can, giving the failed part the type `Unknown` so one
//! mistake isn't reported many times.

use crate::diag::{Diag, Span};
use crate::schema::Interface;
use crate::syntax::{BinOp, Expr, ExprKind, Step, UnOp};
use crate::types::{Range, StrTy, Ty, list, one_of_by_type, unify};
use bytecode::Builtin;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Debug)]
pub struct TExpr {
    pub kind: TKind,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TKind {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// Input `index` (into the interface's inputs).
    Input(usize),
    /// A `let` binding, by id.
    Local(u32),
    Unary(UnOp, Box<TExpr>),
    /// Arithmetic, `==`/`!=` and a single relational comparison (scalars only).
    Binary(BinOp, Box<TExpr>, Box<TExpr>),
    And(Box<TExpr>, Box<TExpr>),
    Or(Box<TExpr>, Box<TExpr>),
    Coalesce(Box<TExpr>, Box<TExpr>),
    /// `a < b <= c`: each operand evaluated once.
    Chain { first: Box<TExpr>, rest: Vec<(BinOp, TExpr)> },
    Cond(Box<TExpr>, Box<TExpr>, Box<TExpr>),
    Let { id: u32, value: Box<TExpr>, body: Box<TExpr> },
    Call(Builtin, Vec<TExpr>),
    /// A record literal, fields in name order.
    Record(Vec<(String, TExpr)>),
    /// Member steps applied to a record. `nonnil[k]` says the value after
    /// the first `k` field reads (0: `base` itself) is known not to be nil,
    /// by narrowing (spec §3.3): no nil check is needed there. `base.ty` is
    /// the stored type; the path's own type is narrowed. A narrowed name
    /// alone is a path with no field reads.
    Path { base: Box<TExpr>, steps: Vec<Step>, nonnil: Vec<bool> },
    /// `rec == nil` (or `!=` when `negate`), for a record-typed operand.
    IsNil { e: Box<TExpr>, negate: bool },
    /// A call to host builtin `decl` (an index into the interface's
    /// builtins). `args` may be shorter than the parameters: the rest are
    /// optional and passed as nil.
    HostCall { decl: usize, args: Vec<TExpr> },
    /// `#` (the item), `#index` or `#acc` of the predicate at `depth` (0 is
    /// the outermost predicate in scope).
    Pointer { which: Ptr, depth: usize },
    /// `[a, b, …]`: a list handle to its items, laid out in locals.
    ListLit(Vec<TExpr>),
    /// `map(src, body)`: a pipeline stage, never a stored list.
    Map { src: Box<TExpr>, body: Box<TExpr> },
    /// `filter(src, pred)`: a pipeline stage.
    Filter { src: Box<TExpr>, pred: Box<TExpr> },
    /// A builtin that consumes a list (or a pipeline) and gives one value.
    /// `f` is the predicate, the mapped expression, the index or the needle,
    /// as the sink needs; `init` is `reduce`'s initial value.
    Sink { sink: Sink, src: Box<TExpr>, f: Option<Box<TExpr>>, init: Option<Box<TExpr>> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ptr {
    Item,
    Index,
    Acc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sink {
    Len,
    Count,
    Sum,
    Mean,
    Min,
    Max,
    Any,
    All,
    NoneOf,
    One,
    Find,
    FindLast,
    FindIndex,
    FindLastIndex,
    First,
    Last,
    /// `xs[i]`: fails out of range.
    At,
    /// `get(xs, i)`: nil out of range.
    Get,
    /// `x in xs`.
    In,
    Reduce,
}

impl Sink {
    pub fn name(self) -> &'static str {
        match self {
            Sink::Len => "len",
            Sink::Count => "count",
            Sink::Sum => "sum",
            Sink::Mean => "mean",
            Sink::Min => "min",
            Sink::Max => "max",
            Sink::Any => "any",
            Sink::All => "all",
            Sink::NoneOf => "none",
            Sink::One => "one",
            Sink::Find => "find",
            Sink::FindLast => "findLast",
            Sink::FindIndex => "findIndex",
            Sink::FindLastIndex => "findLastIndex",
            Sink::First => "first",
            Sink::Last => "last",
            Sink::At => "[]",
            Sink::Get => "get",
            Sink::In => "in",
            Sink::Reduce => "reduce",
        }
    }
}

/// A predicate in scope: what `#`, `#index` and `#acc` are inside it.
#[derive(Clone, Debug)]
struct Pred {
    item: Ty,
    index: Option<Range>,
    acc: Option<Ty>,
}

/// Whether `e` is a `map`/`filter` pipeline, which only another list
/// builtin can consume: it's never stored.
pub fn is_pipeline(e: &TExpr) -> bool {
    matches!(e.kind, TKind::Map { .. } | TKind::Filter { .. })
}

/// What a narrowing fact is about: an input, a `let` name, or a predicate's
/// `#` or `#acc` (at its depth), followed by field names.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Root {
    Input(usize),
    Local(u32),
    Ptr(Ptr, usize),
}

type Key = (Root, Vec<String>);

/// The paths a condition shows aren't nil: when it's true, and when it's false.
#[derive(Clone, Debug, Default)]
struct Facts {
    when_true: Vec<Key>,
    when_false: Vec<Key>,
}

/// The narrowing key of `e`, if it is a name, `#`/`#acc`, or one of those
/// followed by field reads (`.` or `?.`).
fn key(e: &TExpr) -> Option<Key> {
    match &e.kind {
        TKind::Input(i) => Some((Root::Input(*i), Vec::new())),
        TKind::Local(id) => Some((Root::Local(*id), Vec::new())),
        TKind::Pointer { which: which @ (Ptr::Item | Ptr::Acc), depth } => Some((Root::Ptr(*which, *depth), Vec::new())),
        TKind::Path { base, steps, .. } => {
            let (root, mut fields) = key(base)?;
            fields.extend(steps.iter().map(|s| s.field.clone()));
            Some((root, fields))
        }
        _ => None,
    }
}

/// `p` and every prefix of it: if `a?.b.c` isn't nil, neither is `a` or
/// `a.b` (a `.` on nil fails, and a `?.` on nil makes the whole chain nil).
fn with_prefixes((root, fields): Key) -> Vec<Key> {
    (0..=fields.len()).map(|n| (root.clone(), fields[..n].to_vec())).collect()
}

fn union(mut a: Vec<Key>, b: Vec<Key>) -> Vec<Key> {
    for k in b {
        if !a.contains(&k) {
            a.push(k);
        }
    }
    a
}

fn intersection(a: Vec<Key>, b: &[Key]) -> Vec<Key> {
    a.into_iter().filter(|k| b.contains(k)).collect()
}

pub fn check(ast: &Expr, iface: &Interface) -> Result<TExpr, Vec<Diag>> {
    let mut c = Checker {
        iface,
        scopes: Vec::new(),
        next_id: 0,
        diags: Vec::new(),
        preds: Vec::new(),
        narrow: Vec::new(),
        let_facts: HashMap::new(),
        let_names: HashMap::new(),
    };
    let e = c.expr(ast);
    c.stored(&e);
    c.output(Some(&e), &e.ty, &iface.output, e.span, "");
    let mut lets = HashMap::new();
    collect_lets(&e, &mut lets);
    c.variants(&lets, Proj::E(&e), &iface.output, "");
    if c.diags.is_empty() { Ok(e) } else { Err(c.diags) }
}

/// Each `let` binding's value, by id.
fn collect_lets<'e>(e: &'e TExpr, out: &mut HashMap<u32, &'e TExpr>) {
    if let TKind::Let { id, value, .. } = &e.kind {
        out.insert(*id, value);
    }
    for c in children(e) {
        collect_lets(c, out);
    }
}

/// A value at some position of a bigger one, for the variant check: the
/// expression that gives it, where it can be traced, or else just its type.
#[derive(Clone)]
enum Proj<'e> {
    E(&'e TExpr),
    T(Ty, Span),
}

pub fn children(e: &TExpr) -> Vec<&TExpr> {
    match &e.kind {
        TKind::Nil | TKind::Bool(_) | TKind::Int(_) | TKind::Float(_) | TKind::Str(_) | TKind::Input(_) | TKind::Local(_) | TKind::Pointer { .. } => vec![],
        TKind::Unary(_, a) | TKind::IsNil { e: a, .. } => vec![a],
        TKind::Binary(_, a, b) | TKind::And(a, b) | TKind::Or(a, b) | TKind::Coalesce(a, b) => vec![a, b],
        TKind::Chain { first, rest } => std::iter::once(&**first).chain(rest.iter().map(|(_, e)| e)).collect(),
        TKind::Cond(c, a, b) => vec![c, a, b],
        TKind::Let { value, body, .. } => vec![value, body],
        TKind::Call(_, args) | TKind::HostCall { args, .. } | TKind::ListLit(args) => args.iter().collect(),
        TKind::Record(fields) => fields.iter().map(|(_, e)| e).collect(),
        TKind::Path { base, .. } => vec![base],
        TKind::Map { src, body: f } | TKind::Filter { src, pred: f } => vec![src, f],
        TKind::Sink { src, f, init, .. } => std::iter::once(&**src).chain(f.as_deref()).chain(init.as_deref()).collect(),
    }
}

use crate::schema::EXPR_BUILTINS;

/// Builtins the language will have, but this build doesn't yet.
const LATER: &[&str] = &["sortBy", "median", "take", "sort", "concat", "flatten", "uniq", "reverse"];

struct Checker<'a> {
    iface: &'a Interface,
    /// `let` bindings in scope, innermost last: (name, id, type, the value
    /// itself when it's a pipeline, which each use re-runs).
    scopes: Vec<(String, u32, Ty, Option<TExpr>)>,
    next_id: u32,
    diags: Vec<Diag>,
    /// Predicates in scope, innermost last.
    preds: Vec<Pred>,
    /// The paths known not to be nil here (narrowing, spec §3.3).
    narrow: Vec<Key>,
    /// What each boolean `let` shows about nil, by id.
    let_facts: HashMap<u32, Facts>,
    /// Each `let`'s name, by id (for messages).
    let_names: HashMap<u32, String>,
}

fn node(kind: TKind, ty: Ty, span: Span) -> TExpr {
    TExpr { kind, ty, span }
}

impl Checker<'_> {
    fn err(&mut self, d: Diag) {
        self.diags.push(d);
    }

    fn expr(&mut self, e: &Expr) -> TExpr {
        let span = e.span;
        match &e.kind {
            ExprKind::Nil => node(TKind::Nil, Ty::Nil, span),
            ExprKind::Bool(b) => node(TKind::Bool(*b), Ty::Bool, span),
            ExprKind::Int(v) => node(TKind::Int(*v), Ty::Int(Range::exact(*v)), span),
            ExprKind::Float(v) => node(TKind::Float(*v), Ty::Float, span),
            ExprKind::Str(s) => {
                let ty = Ty::Str(StrTy { max: s.len() as u32, chars: s.chars().count() as u32, values: Some(BTreeSet::from([s.clone()])) });
                node(TKind::Str(s.clone()), ty, span)
            }
            ExprKind::Name(n) => self.name(n, span),
            ExprKind::Unary(op, a) => self.unary(*op, a, span),
            ExprKind::Binary(op, a, b) => self.binary(*op, a, b, span),
            ExprKind::Chain(first, rest) => self.chain(first, rest, span),
            ExprKind::Cond(c, a, b) => {
                let c = self.condition(c);
                let f = self.facts(&c);
                let a = self.narrowed(&f.when_true, |m| m.expr(a));
                let b = self.narrowed(&f.when_false, |m| m.expr(b));
                self.stored(&a);
                self.stored(&b);
                let ty = match unify(&a.ty, &b.ty) {
                    Some(t) => t,
                    None => {
                        self.err(
                            Diag::error("E-TYPE", a.span.to(b.span), format!("the branches don't agree: one is {}, the other {}", a.ty.describe(), b.ty.describe()))
                                .help("both branches must give the same kind of value (ints and floats mix; records need the same fields)"),
                        );
                        Ty::Unknown
                    }
                };
                node(TKind::Cond(Box::new(c), Box::new(a), Box::new(b)), ty, span)
            }
            ExprKind::Let { name, name_span, value, body } => {
                if EXPR_BUILTINS.contains(&name.as_str()) || self.iface.builtins.iter().any(|b| b.name == *name) {
                    self.err(
                        Diag::error("E-REDECLARE", *name_span, format!("`{name}` is a builtin and can't be a name"))
                            .help(format!("rename it, e.g. `{name}_v`")),
                    );
                } else if self.scopes.iter().any(|(n, _, _, _)| n == name) {
                    self.err(
                        Diag::error("E-REDECLARE", *name_span, format!("`{name}` is already named"))
                            .help("a name is set once; pick a new name for the new value"),
                    );
                } else if self.iface.inputs.iter().any(|i| i.name == *name) {
                    // expr: `cannot redeclare i` when `i` is in the environment.
                    self.err(
                        Diag::error("E-REDECLARE", *name_span, format!("`{name}` is an input and can't be renamed"))
                            .help(format!("pick a new name, e.g. `{name}2`")),
                    );
                }
                let value = self.expr(value);
                let id = self.next_id;
                self.next_id += 1;
                self.let_names.insert(id, name.clone());
                // A boolean name carries what its value shows about nil:
                // `let ok = p != nil; ok ? p.f : …` narrows `p`.
                if value.ty == Ty::Bool {
                    let f = self.facts(&value);
                    if !f.when_true.is_empty() || !f.when_false.is_empty() {
                        self.let_facts.insert(id, f);
                    }
                }
                // A pipeline isn't stored: each use of the name re-runs it.
                let inline = is_pipeline(&value).then(|| value.clone());
                let inlined = inline.is_some();
                self.scopes.push((name.clone(), id, value.ty.clone(), inline));
                let body = self.expr(body);
                self.scopes.pop();
                if inlined {
                    return body;
                }
                let ty = body.ty.clone();
                node(TKind::Let { id, value: Box::new(value), body: Box::new(body) }, ty, span)
            }
            ExprKind::Call { name, name_span, args } => self.call(name, *name_span, args, span),
            ExprKind::Record(fields) => {
                let mut out: BTreeMap<String, TExpr> = BTreeMap::new();
                for (k, kspan, v) in fields {
                    let t = self.expr(v);
                    self.stored(&t);
                    if out.contains_key(k) {
                        self.err(Diag::error("E-DUP-KEY", *kspan, format!("the field `{k}` appears twice")).help("keep one"));
                        continue;
                    }
                    out.insert(k.clone(), t);
                }
                let ty = Ty::Rec(out.iter().map(|(k, t)| (k.clone(), t.ty.clone())).collect(), None);
                node(TKind::Record(out.into_iter().collect()), ty, span)
            }
            ExprKind::Path { base, steps } => self.path(base, steps, span),
            ExprKind::Pointer(p) => {
                let p = self.pointer(p, span);
                self.narrow_read(p)
            }
            ExprKind::List(items) => {
                let mut titems = Vec::new();
                let mut item = Ty::Never;
                for it in items {
                    let t = self.expr(it);
                    self.stored(&t);
                    if crate::schema::contains_list(&t.ty) {
                        self.err(Diag::error("E-UNSUPPORTED", t.span, "lists inside lists aren't supported yet"));
                    }
                    item = match unify(&item, &t.ty) {
                        Some(u) if matches!(item, Ty::Never) || crate::types::same_layout(&item, &t.ty) => u,
                        _ => {
                            self.err(
                                Diag::error("E-TYPE", t.span, format!("the list's items don't agree: {} and {}", item.describe(), t.ty.describe()))
                                    .help("every item must be the same kind of value, with the same fields"),
                            );
                            Ty::Unknown
                        }
                    };
                    titems.push(t);
                }
                let n = titems.len() as u32;
                node(TKind::ListLit(titems), Ty::List(Box::new(item), n), span)
            }
            ExprKind::Index(base, i) => {
                let base = self.expr(base);
                let i = self.expr(i);
                self.list_sink(Sink::At, base, Some(i), None, span)
            }
        }
    }

    /// `#`, `#index`, `#acc`: from the innermost predicate in scope.
    fn pointer(&mut self, p: &str, span: Span) -> TExpr {
        let Some(pred) = self.preds.last().cloned() else {
            self.err(
                Diag::error("E-POINTER", span, format!("`#{p}` is only meaningful inside a predicate"))
                    .help("use it in the expression argument of a list builtin, e.g. `filter(xs, # > 0)`"),
            );
            return node(TKind::Nil, Ty::Unknown, span);
        };
        let depth = self.preds.len() - 1;
        match p {
            "" => node(TKind::Pointer { which: Ptr::Item, depth }, pred.item, span),
            "index" => match pred.index {
                Some(r) => node(TKind::Pointer { which: Ptr::Index, depth }, Ty::Int(r), span),
                None => {
                    self.err(
                        Diag::error("E-POINTER", span, "`#index` exists only in `map` and `reduce`")
                            .help("to skip items use a slice; to count positions, `map` first: `map(xs, #index)`"),
                    );
                    node(TKind::Nil, Ty::Unknown, span)
                }
            },
            _ => match pred.acc {
                Some(t) => node(TKind::Pointer { which: Ptr::Acc, depth }, t, span),
                None => {
                    self.err(Diag::error("E-POINTER", span, "`#acc` exists only in `reduce`"));
                    node(TKind::Nil, Ty::Unknown, span)
                }
            },
        }
    }

    /// A `map`/`filter` result can only be consumed by another list builtin
    /// for now: holding one (in a branch, a record, the output) comes later.
    fn stored(&mut self, e: &TExpr) {
        if is_pipeline(e) {
            self.err(
                Diag::error("E-UNSUPPORTED", e.span, "a `map` or `filter` result can only be passed to another list builtin here")
                    .help("reduce it to a value (`sum`, `count`, `first`, `find`, …); keeping a computed list isn't implemented yet"),
            );
        }
    }

    /// The item type and bound of a list operand (nil is allowed: most
    /// list builtins fail on it at run time, as in expr).
    fn list_of(&mut self, e: &TExpr, what: &str) -> Option<(Ty, u32)> {
        match e.ty.non_nil() {
            Some(Ty::List(item, n)) => Some((*item, n)),
            Some(Ty::Unknown) => None,
            None => {
                self.err(Diag::error("E-TYPE", e.span, format!("`{what}` needs a list, but this is always nil")));
                None
            }
            Some(other) => {
                self.err(Diag::error("E-TYPE", e.span, format!("`{what}` needs a list, but this is {}", other.describe())));
                None
            }
        }
    }

    /// Checks `e` as a predicate over items of `item`.
    fn in_pred(&mut self, e: &Expr, item: &Ty, index: Option<Range>, acc: Option<Ty>) -> TExpr {
        self.preds.push(Pred { item: item.clone(), index, acc });
        let t = self.expr(e);
        self.preds.pop();
        t
    }

    /// A list builtin: `name(args…)` where `args[0]` is the list.
    fn list_call(&mut self, name: &str, args: &[Expr], span: Span) -> TExpr {
        let (want, sink) = match name {
            "len" => (1..=1, Some(Sink::Len)),
            "count" => (1..=2, Some(Sink::Count)),
            "sum" => (1..=2, Some(Sink::Sum)),
            "mean" => (1..=1, Some(Sink::Mean)),
            "min" => (1..=1, Some(Sink::Min)),
            "max" => (1..=1, Some(Sink::Max)),
            "any" => (2..=2, Some(Sink::Any)),
            "all" => (2..=2, Some(Sink::All)),
            "none" => (2..=2, Some(Sink::NoneOf)),
            "one" => (2..=2, Some(Sink::One)),
            "find" => (2..=2, Some(Sink::Find)),
            "findLast" => (2..=2, Some(Sink::FindLast)),
            "findIndex" => (2..=2, Some(Sink::FindIndex)),
            "findLastIndex" => (2..=2, Some(Sink::FindLastIndex)),
            "first" => (1..=1, Some(Sink::First)),
            "last" => (1..=1, Some(Sink::Last)),
            "get" => (2..=2, Some(Sink::Get)),
            "reduce" => (2..=3, Some(Sink::Reduce)),
            "map" | "filter" => (2..=2, None),
            _ => unreachable!(),
        };
        if !want.contains(&args.len()) {
            let n = if want.start() == want.end() { format!("{}", want.start()) } else { format!("{} or {}", want.start(), want.end()) };
            self.err(Diag::error("E-ARG", span, format!("`{name}` takes {n} arguments, but this gives {}", args.len())));
            return node(TKind::Nil, Ty::Unknown, span);
        }
        let src = self.expr(&args[0]);
        let Some((item, n)) = self.list_of(&src, name) else {
            return node(TKind::Nil, Ty::Unknown, span);
        };
        let index = Some(if n == 0 { Range::exact(0) } else { Range { lo: 0, hi: n as i128 - 1 } });
        match (name, sink) {
            ("map", _) => {
                let body = self.in_pred(&args[1], &item, index, None);
                self.stored(&body);
                if crate::schema::contains_list(&body.ty) {
                    self.err(Diag::error("E-UNSUPPORTED", body.span, "a `map` that gives lists isn't supported yet"));
                }
                let ty = Ty::List(Box::new(body.ty.clone()), n);
                node(TKind::Map { src: Box::new(src), body: Box::new(body) }, ty, span)
            }
            ("filter", _) => {
                let pred = self.in_pred(&args[1], &item, None, None);
                self.want_pred_bool(&pred, "filter");
                node(TKind::Filter { src: Box::new(src), pred: Box::new(pred) }, Ty::List(Box::new(item), n), span)
            }
            (_, Some(Sink::Reduce)) => {
                let init = args.get(2).map(|a| self.expr(a));
                // The accumulator's type: iterate to a fixed point, widening an
                // int range that keeps changing to the full range.
                let mut acc = init.as_ref().map(|i| i.ty.clone()).unwrap_or_else(|| item.clone());
                let mut body;
                let mut round = 0;
                loop {
                    let before = self.diags.len();
                    body = self.in_pred(&args[1], &item, index, Some(acc.clone()));
                    let next = unify(&acc, &body.ty);
                    match next {
                        Some(t) if t == acc => break,
                        Some(t) if round < 3 => {
                            self.diags.truncate(before);
                            acc = if round >= 1 { widen(&t) } else { t };
                            round += 1;
                        }
                        Some(t) => {
                            acc = widen(&t);
                            break;
                        }
                        None => {
                            self.err(Diag::error("E-TYPE", body.span, format!("this `reduce` step gives {}, but the accumulator is {}", body.ty.describe(), acc.describe()))
                                .help("each step must give the same kind of value as the initial one"));
                            break;
                        }
                    }
                }
                if crate::schema::contains_list(&acc) {
                    self.err(Diag::error("E-ACC-GROWS", body.span, "a `reduce` accumulator can't be a list").help("keep a fixed set of values, e.g. a record of totals"));
                }
                let ty = acc.clone();
                node(TKind::Sink { sink: Sink::Reduce, src: Box::new(src), f: Some(Box::new(body)), init: init.map(Box::new) }, ty, span)
            }
            (_, Some(sink)) => {
                let f = match sink {
                    Sink::Count | Sink::Any | Sink::All | Sink::NoneOf | Sink::One | Sink::Find | Sink::FindLast | Sink::FindIndex | Sink::FindLastIndex => {
                        match args.get(1) {
                            Some(a) => {
                                let p = self.in_pred(a, &item, None, None);
                                self.want_pred_bool(&p, sink.name());
                                Some(p)
                            }
                            None => {
                                // `count(xs)` counts the trues.
                                if !matches!(item, Ty::Bool | Ty::Unknown) {
                                    self.err(Diag::error("E-ARG", span, "`count(xs)` with no predicate counts the trues in a list of bools").help("give a predicate: `count(xs, …)`"));
                                }
                                None
                            }
                        }
                    }
                    Sink::Sum => args.get(1).map(|a| self.in_pred(a, &item, None, None)),
                    Sink::Get => Some(self.expr(&args[1])),
                    _ => None,
                };
                self.list_sink(sink, src, f, None, span)
            }
            _ => unreachable!(),
        }
    }

    /// Types a sink over `src`: `f` is already checked.
    fn list_sink(&mut self, sink: Sink, src: TExpr, f: Option<TExpr>, init: Option<TExpr>, span: Span) -> TExpr {
        let Some((item, n)) = self.list_of(&src, sink.name()) else {
            return node(TKind::Nil, Ty::Unknown, span);
        };
        let pos = if n == 0 { Range::exact(0) } else { Range { lo: 0, hi: n as i128 - 1 } };
        let numeric = |c: &mut Self, t: &Ty, at: Span| -> Option<Ty> {
            match t.non_nil() {
                Some(t @ (Ty::Int(_) | Ty::Float)) if !matches!(t, Ty::Unknown) => Some(t),
                Some(Ty::Unknown) => None,
                _ => {
                    c.err(Diag::error("E-TYPE", at, format!("`{}` needs numbers, but these are {}", sink.name(), t.describe())));
                    None
                }
            }
        };
        let ty = match sink {
            Sink::Len | Sink::Count => Ty::Int(Range { lo: 0, hi: n as i128 }),
            Sink::Sum => {
                let (t, at) = match &f {
                    Some(f) => (f.ty.clone(), f.span),
                    None => (item.clone(), src.span),
                };
                match numeric(self, &t, at) {
                    // Each of up to n items in lo..hi: the total is in min(0, n·lo)..max(0, n·hi).
                    Some(Ty::Int(r)) => Ty::Int(Range::checked((n as i128 * r.lo).min(0), (n as i128 * r.hi).max(0))),
                    Some(_) => Ty::Float,
                    None => Ty::Unknown,
                }
            }
            Sink::Mean => {
                numeric(self, &item, src.span);
                Ty::Float
            }
            Sink::Min | Sink::Max => match numeric(self, &item, src.span) {
                Some(t) => t.nullable(),
                None => Ty::Unknown,
            },
            Sink::Any | Sink::All | Sink::NoneOf | Sink::One | Sink::In => Ty::Bool,
            Sink::Find | Sink::FindLast | Sink::First | Sink::Last | Sink::Get => item.clone().nullable(),
            Sink::FindIndex | Sink::FindLastIndex => Ty::Int(pos).nullable(),
            Sink::At => item.clone(),
            Sink::Reduce => unreachable!(),
        };
        if let (Sink::At | Sink::Get, Some(i)) = (sink, &f) {
            match i.ty.non_nil() {
                Some(Ty::Int(_) | Ty::Unknown) => {}
                _ => self.err(Diag::error("E-TYPE", i.span, format!("an index must be an integer, but this is {}", i.ty.describe())).help("e.g. `xs[int(n / 2)]`")),
            }
        }
        if sink == Sink::In {
            if let Some(x) = &f {
                if is_record(&item) || is_record(&x.ty) {
                    self.err(Diag::error("E-UNSUPPORTED", span, "`in` doesn't compare records").help("compare a field: `any(xs, .id == x.id)`"));
                } else if let (Some(a), Some(b)) = (x.ty.non_nil(), item.non_nil()) {
                    if unify(&a, &b).is_none() {
                        self.err(Diag::error("E-TYPE", span, format!("`in` looks for {} in a list of {}, which can never match", a.describe(), b.describe())));
                    }
                }
            }
        }
        node(TKind::Sink { sink, src: Box::new(src), f: f.map(Box::new), init: init.map(Box::new) }, ty, span)
    }

    /// Checks `f` with the paths `keys` known not to be nil.
    fn narrowed<T>(&mut self, keys: &[Key], f: impl FnOnce(&mut Self) -> T) -> T {
        let mark = self.narrow.len();
        for k in keys {
            if !self.narrow.contains(k) {
                self.narrow.push(k.clone());
            }
        }
        let r = f(self);
        self.narrow.truncate(mark);
        r
    }

    /// What a condition shows about nil (spec §3.3): `p != nil` (or
    /// `nil != p`) shows `p` and its prefixes aren't nil when it's true,
    /// `p == nil` when it's false; `!`, `&&`, `||` and boolean names combine
    /// them.
    fn facts(&self, c: &TExpr) -> Facts {
        let nil_test = |x: &TExpr, is_nil: bool| -> Facts {
            match key(x) {
                Some(k) if is_nil => Facts { when_true: Vec::new(), when_false: with_prefixes(k) },
                Some(k) => Facts { when_true: with_prefixes(k), when_false: Vec::new() },
                None => Facts::default(),
            }
        };
        match &c.kind {
            TKind::IsNil { e, negate } => nil_test(e, !*negate),
            TKind::Binary(op @ (BinOp::Eq | BinOp::Ne), a, b) => match (&a.kind, &b.kind) {
                (_, TKind::Nil) => nil_test(a, *op == BinOp::Eq),
                (TKind::Nil, _) => nil_test(b, *op == BinOp::Eq),
                _ => Facts::default(),
            },
            TKind::Unary(UnOp::Not, a) => {
                let f = self.facts(a);
                Facts { when_true: f.when_false, when_false: f.when_true }
            }
            TKind::And(a, b) => {
                let (fa, fb) = (self.facts(a), self.facts(b));
                Facts { when_true: union(fa.when_true, fb.when_true), when_false: intersection(fa.when_false, &fb.when_false) }
            }
            TKind::Or(a, b) => {
                let (fa, fb) = (self.facts(a), self.facts(b));
                Facts { when_true: intersection(fa.when_true, &fb.when_true), when_false: union(fa.when_false, fb.when_false) }
            }
            TKind::Local(id) => self.let_facts.get(id).cloned().unwrap_or_default(),
            TKind::Let { body, .. } => self.facts(body),
            _ => Facts::default(),
        }
    }

    /// A name, `#` or `#acc` read where it's known not to be nil: a path of
    /// no field reads, with the nil taken out of its type.
    fn narrow_read(&self, e: TExpr) -> TExpr {
        let Some(k) = key(&e) else { return e };
        match e.ty.non_nil() {
            Some(ty) if e.ty.may_be_nil() && self.narrow.contains(&k) => {
                let span = e.span;
                TExpr { kind: TKind::Path { base: Box::new(e), steps: Vec::new(), nonnil: vec![true] }, ty, span }
            }
            _ => e,
        }
    }

    /// How `e` is written, if it's a name or `#` followed by fields (for messages).
    fn render(&self, e: &TExpr) -> Option<String> {
        match &e.kind {
            TKind::Input(i) => Some(self.iface.inputs[*i].name.clone()),
            TKind::Local(id) => self.let_names.get(id).cloned(),
            TKind::Pointer { which: Ptr::Item, .. } => Some("#".into()),
            TKind::Pointer { which: Ptr::Acc, .. } => Some("#acc".into()),
            TKind::Path { base, steps, .. } => {
                let mut s = self.render(base)?;
                for st in steps {
                    s.push_str(if st.optional { "?." } else { "." });
                    s.push_str(&st.field);
                }
                Some(s)
            }
            _ => None,
        }
    }

    /// The help for a value that may be nil where nil isn't allowed: test
    /// it, when it's a path narrowing applies to, or give a default.
    fn nil_help(&self, e: Option<&TExpr>, default: &str) -> String {
        match e.and_then(|e| self.render(e)) {
            Some(p) => format!("test it first: `{p} != nil ? … : …` (in the first branch `{p}` isn't nil), or give a default: `{p} ?? {default}`"),
            None => format!("give a default with `??`, e.g. `(…) ?? {default}`"),
        }
    }

    fn name(&mut self, n: &str, span: Span) -> TExpr {
        let e = self.raw_name(n, span);
        self.narrow_read(e)
    }

    fn raw_name(&mut self, n: &str, span: Span) -> TExpr {
        if let Some((_, id, ty, inline)) = self.scopes.iter().rev().find(|(m, _, _, _)| m == n) {
            if let Some(v) = inline {
                return TExpr { span, ..v.clone() };
            }
            return node(TKind::Local(*id), ty.clone(), span);
        }
        if let Some(i) = self.iface.inputs.iter().position(|inp| inp.name == n) {
            return node(TKind::Input(i), self.iface.inputs[i].ty.clone(), span);
        }
        let d = if EXPR_BUILTINS.contains(&n) || self.iface.builtins.iter().any(|b| b.name == n) {
            Diag::error("E-NAME", span, format!("`{n}` is a builtin")).help(format!("call it: `{n}(…)`"))
        } else {
            let names: Vec<&str> =
                self.scopes.iter().map(|(m, _, _, _)| m.as_str()).chain(self.iface.inputs.iter().map(|i| i.name.as_str())).collect();
            let d = Diag::error("E-NAME", span, format!("there's no input or name `{n}`"));
            match nearest(n, &names) {
                Some(s) => d.help(format!("did you mean `{s}`?")),
                None if names.is_empty() => d.help("this program has no inputs"),
                None => d.help(format!("the names in scope are: {}", names.join(", "))),
            }
        };
        self.err(d);
        node(TKind::Nil, Ty::Unknown, span)
    }

    /// `base.f?.g…` (spec §2.2): `.` on nil fails at run time; a `?.` whose
    /// target is nil makes the whole rest of the chain nil.
    fn path(&mut self, base: &Expr, steps: &[Step], span: Span) -> TExpr {
        let mut what = match &base.kind {
            ExprKind::Name(n) => format!("`{n}`"),
            _ => "this".to_string(),
        };
        let mut base = self.expr(base);
        // A narrowed name: start from the name itself; the prefixes are
        // narrowed below, the name among them.
        if let TKind::Path { base: inner, steps, .. } = &base.kind
            && steps.is_empty()
        {
            base = (**inner).clone();
        }
        // Narrowing (spec §3.3): a prefix known not to be nil loses its nil.
        let mut k = key(&base);
        let mut nonnil = Vec::with_capacity(steps.len() + 1);
        let mut cur = base.ty.clone();
        let narrowed = |k: &Option<Key>, narrow: &[Key]| k.as_ref().is_some_and(|k| narrow.contains(k));
        if narrowed(&k, &self.narrow) && cur.may_be_nil() {
            cur = cur.non_nil().unwrap_or(Ty::Nil);
        }
        nonnil.push(narrowed(&k, &self.narrow));
        let mut short_circuits = false;
        for s in steps {
            let rec = match &cur {
                Ty::Unknown => return node(TKind::Nil, Ty::Unknown, span),
                Ty::Rec(f, _) => f,
                // (`Opt` only when this prefix isn't narrowed)
                Ty::Opt(inner) if matches!(**inner, Ty::Rec(..)) => {
                    if s.optional {
                        short_circuits = true;
                    }
                    let Ty::Rec(f, _) = &**inner else { unreachable!() };
                    f
                }
                Ty::Nil => {
                    self.err(Diag::error("E-TYPE", s.span, format!("{what} is always nil, so it has no field `{}`", s.field)));
                    return node(TKind::Nil, Ty::Unknown, span);
                }
                other => {
                    self.err(Diag::error("E-TYPE", s.span, format!("{what} is {}, not a record, so it has no field `{}`", other.describe(), s.field)));
                    return node(TKind::Nil, Ty::Unknown, span);
                }
            };
            let Some(ft) = rec.get(&s.field) else {
                let names: Vec<&str> = rec.keys().map(String::as_str).collect();
                let d = Diag::error("E-NAME", s.span, format!("{what} has no field `{}`", s.field));
                let d = match nearest(&s.field, &names) {
                    Some(n) => d.help(format!("did you mean `{n}`?")),
                    None => d.help(format!("its fields are: {}", names.join(", "))),
                };
                self.err(d);
                return node(TKind::Nil, Ty::Unknown, span);
            };
            cur = ft.clone();
            what = format!("`{}`", s.field);
            if let Some((_, fields)) = &mut k {
                fields.push(s.field.clone());
            }
            let known = narrowed(&k, &self.narrow);
            if known {
                // Not nil here, so neither is any prefix, and no `?.` before
                // it short-circuited.
                short_circuits = false;
                nonnil.iter_mut().for_each(|n| *n = true);
                if cur.may_be_nil() {
                    cur = cur.non_nil().unwrap_or(Ty::Nil);
                }
            }
            nonnil.push(known);
        }
        let ty = if short_circuits { cur.nullable() } else { cur };
        node(TKind::Path { base: Box::new(base), steps: steps.to_vec(), nonnil }, ty, span)
    }

    /// A condition must be a bool; a `bool?` is an error with a fix.
    fn condition(&mut self, e: &Expr) -> TExpr {
        let t = self.expr(e);
        self.want_bool(&t);
        t
    }

    fn want_bool(&mut self, t: &TExpr) {
        match &t.ty {
            Ty::Bool | Ty::Unknown => {}
            Ty::Opt(inner) if **inner == Ty::Bool => self.err(
                Diag::error("E-TYPE", t.span, "this may be nil, so it can't be used as a condition").help("give it a default: `(…) ?? false`"),
            ),
            other => self.err(Diag::error("E-TYPE", t.span, format!("a condition must be a bool, but this is {}", other.describe()))),
        }
    }

    /// A list builtin's predicate must be a bool; the message names the builtin.
    fn want_pred_bool(&mut self, t: &TExpr, name: &str) {
        match &t.ty {
            Ty::Bool | Ty::Unknown => {}
            Ty::Opt(inner) if **inner == Ty::Bool => self.err(
                Diag::error("E-TYPE", t.span, format!("`{name}`'s predicate may be nil, but it must be a bool"))
                    .help("give it a default: `(…) ?? false`"),
            ),
            other => self.err(
                Diag::error("E-TYPE", t.span, format!("`{name}`'s predicate must be a bool, but this is {}", other.describe()))
                    .help(format!("compare it, e.g. `{name}(xs, # > 0)`")),
            ),
        }
    }

    /// The numeric part of an operand's type (nil fails at run time, as in
    /// expr), or an error.
    fn number(&mut self, t: &TExpr, what: &str) -> Option<Ty> {
        match t.ty.non_nil() {
            Some(Ty::Unknown) => None,
            Some(n @ (Ty::Int(_) | Ty::Float)) => Some(n),
            None => {
                self.err(Diag::error("E-TYPE", t.span, format!("`{what}` needs a number, but this is always nil")));
                None
            }
            Some(other) => {
                self.err(Diag::error("E-TYPE", t.span, format!("`{what}` needs a number, but this is {}", other.describe())));
                None
            }
        }
    }

    fn unary(&mut self, op: UnOp, a: &Expr, span: Span) -> TExpr {
        let a = self.expr(a);
        let ty = match op {
            UnOp::Neg => match self.number(&a, "-") {
                Some(Ty::Int(r)) => Ty::Int(r.neg()),
                Some(t) => t,
                None => Ty::Unknown,
            },
            UnOp::Not => {
                self.want_bool(&a);
                Ty::Bool
            }
        };
        node(TKind::Unary(op, Box::new(a)), ty, span)
    }

    fn binary(&mut self, op: BinOp, a: &Expr, b: &Expr, span: Span) -> TExpr {
        let a = self.expr(a);
        // `l && r` checks `r` knowing `l` is true; `l || r`, knowing it's false.
        let b = match op {
            BinOp::And => {
                let f = self.facts(&a);
                self.narrowed(&f.when_true, |m| m.expr(b))
            }
            BinOp::Or => {
                let f = self.facts(&a);
                self.narrowed(&f.when_false, |m| m.expr(b))
            }
            _ => self.expr(b),
        };
        let ty = match op {
            BinOp::And | BinOp::Or => {
                self.want_bool(&a);
                self.want_bool(&b);
                let k = if op == BinOp::And { TKind::And(Box::new(a), Box::new(b)) } else { TKind::Or(Box::new(a), Box::new(b)) };
                return node(k, Ty::Bool, span);
            }
            BinOp::In => {
                self.stored(&a);
                return self.list_sink(Sink::In, b, Some(a), None, span);
            }
            BinOp::Eq | BinOp::Ne if [&a, &b].iter().any(|t| matches!(t.ty.non_nil(), Some(Ty::List(..)))) => {
                if [&a, &b].iter().all(|t| matches!(t.ty.non_nil(), Some(Ty::List(..)))) {
                    self.err(Diag::error("E-UNSUPPORTED", span, "comparing lists isn't implemented yet").help("compare their lengths or items"));
                    return node(TKind::Nil, Ty::Unknown, span);
                }
                // A list compared with nil: the handle is nil or not.
                self.comparable(&a, &b, op);
                Ty::Bool
            }
            BinOp::Coalesce => {
                self.stored(&a);
                self.stored(&b);
                let ty = match a.ty.non_nil() {
                    None => Some(b.ty.clone()),
                    Some(left) => unify(&left, &b.ty),
                };
                let ty = ty.unwrap_or_else(|| {
                    self.err(Diag::error(
                        "E-TYPE",
                        span,
                        format!("the two sides of `??` don't agree: {} and {}", a.ty.describe(), b.ty.describe()),
                    ).help("the default must be the same kind of value as the left side"));
                    Ty::Unknown
                });
                return node(TKind::Coalesce(Box::new(a), Box::new(b)), ty, span);
            }
            BinOp::Eq | BinOp::Ne if is_record(&a.ty) || is_record(&b.ty) => {
                // A record compares only with nil.
                let (rec, other) = if is_record(&a.ty) { (a, b) } else { (b, a) };
                if matches!(other.ty, Ty::Nil) {
                    return node(TKind::IsNil { e: Box::new(rec), negate: op == BinOp::Ne }, Ty::Bool, span);
                }
                self.err(
                    Diag::error("E-UNSUPPORTED", span, format!("records can't be compared with `{}`", op.symbol()))
                        .help("compare their fields, e.g. `a.x == b.x`, or compare with `nil`"),
                );
                return node(TKind::Nil, Ty::Unknown, span);
            }
            BinOp::Add if [&a, &b].iter().any(|t| matches!(t.ty.non_nil(), Some(Ty::Str(_)))) => {
                self.err(
                    Diag::error("E-UNSUPPORTED", span, "strings can't be built with `+`")
                        .help("output numbers; format them where they're displayed"),
                );
                Ty::Unknown
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul => {
                let (x, y) = (self.number(&a, op.symbol()), self.number(&b, op.symbol()));
                match (x, y) {
                    (Some(Ty::Int(x)), Some(Ty::Int(y))) => Ty::Int(match op {
                        BinOp::Add => x.add(y),
                        BinOp::Sub => x.sub(y),
                        _ => x.mul(y),
                    }),
                    (Some(_), Some(_)) => Ty::Float,
                    _ => Ty::Unknown,
                }
            }
            BinOp::Div | BinOp::Pow => {
                self.number(&a, op.symbol());
                self.number(&b, op.symbol());
                Ty::Float
            }
            BinOp::Mod => {
                for t in [&a, &b] {
                    match t.ty.non_nil() {
                        Some(Ty::Int(_) | Ty::Unknown) => {}
                        Some(Ty::Float) => self.err(Diag::error("E-TYPE", t.span, "`%` needs integers, but this is a float").help("convert it first: `int(x) % n`")),
                        _ => {
                            self.number(t, "%");
                        }
                    }
                }
                Ty::Int(Range::FULL)
            }
            BinOp::Eq | BinOp::Ne => {
                self.comparable(&a, &b, op);
                Ty::Bool
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                self.ordered(&a, op);
                self.ordered(&b, op);
                Ty::Bool
            }
        };
        node(TKind::Binary(op, Box::new(a), Box::new(b)), ty, span)
    }

    fn comparable(&mut self, a: &TExpr, b: &TExpr, op: BinOp) {
        let (Some(x), Some(y)) = (a.ty.non_nil(), b.ty.non_nil()) else { return }; // comparing with nil is fine
        if unify(&x, &y).is_none() {
            self.err(
                Diag::error("E-TYPE", a.span.to(b.span), format!("`{}` compares {} with {}, which can never be equal", op.symbol(), x.describe(), y.describe()))
                    .help("compare values of the same kind (ints and floats mix)"),
            );
        }
    }

    fn ordered(&mut self, t: &TExpr, op: BinOp) {
        match t.ty.non_nil() {
            Some(Ty::Str(_)) => self.err(Diag::error("E-TYPE", t.span, format!("strings can't be compared with `{}`", op.symbol())).help("strings support only `==` and `!=`")),
            _ => {
                self.number(t, op.symbol());
            }
        }
    }

    fn chain(&mut self, first: &Expr, rest: &[(BinOp, Expr)], span: Span) -> TExpr {
        let first = self.expr(first);
        self.ordered(&first, rest[0].0);
        let mut out = Vec::new();
        for (op, e) in rest {
            let t = self.expr(e);
            self.ordered(&t, *op);
            out.push((*op, t));
        }
        node(TKind::Chain { first: Box::new(first), rest: out }, Ty::Bool, span)
    }

    /// A call to a host builtin: arity, then each argument against its
    /// parameter. A record argument may have more fields than the parameter
    /// asks for (a fix with `alt` is a position).
    fn host_call(&mut self, decl: usize, name_span: Span, args: &[Expr], span: Span) -> TExpr {
        let d = self.iface.builtins[decl].clone();
        let targs: Vec<TExpr> = args.iter().map(|a| self.expr(a)).collect();
        let required = d.params.iter().filter(|(_, _, opt)| !opt).count();
        let usage = format!(
            "{}({})",
            d.name,
            d.params.iter().map(|(n, _, opt)| if *opt { format!("{n}?") } else { n.clone() }).collect::<Vec<_>>().join(", ")
        );
        if targs.len() < required || targs.len() > d.params.len() {
            let want = if required == d.params.len() { format!("{required}") } else { format!("{required} to {}", d.params.len()) };
            self.err(Diag::error("E-ARG", span, format!("`{}` takes {want} arguments, but this gives {}", d.name, targs.len())).help(format!("call it as `{usage}`")));
            return node(TKind::Nil, Ty::Unknown, span);
        }
        let _ = name_span;
        for a in &targs {
            self.stored(a);
        }
        for (a, (pname, pty, _)) in targs.iter().zip(&d.params) {
            if let Err(why) = conform(&a.ty, pty) {
                // A nullable argument that narrowing could make non-nil: say how.
                let help = match self.render(a) {
                    Some(p) if a.ty.may_be_nil() && !pty.may_be_nil() => format!(
                        "test it first: `{p} != nil ? {}(…) : …` (in the first branch `{p}` isn't nil), or give a default: `{p} ?? …`",
                        d.name
                    ),
                    _ => format!("`{pname}` is {}; call it as `{usage}`", pty.describe()),
                };
                self.err(Diag::error("E-ARG", a.span, format!("the argument `{pname}` of `{}`: {why}", d.name)).help(help));
            }
        }
        node(TKind::HostCall { decl, args: targs }, d.result.clone(), span)
    }

    fn call(&mut self, name: &str, name_span: Span, args: &[Expr], span: Span) -> TExpr {
        if let Some(decl) = self.iface.builtins.iter().position(|b| b.name == name) {
            return self.host_call(decl, name_span, args, span);
        }
        let list_builtin = matches!(
            name,
            "len" | "count" | "sum" | "mean" | "any" | "all" | "none" | "one" | "find" | "findLast" | "findIndex" | "findLastIndex"
                | "first" | "last" | "get" | "reduce" | "map" | "filter"
        ) || (matches!(name, "min" | "max") && args.len() == 1);
        if list_builtin {
            return self.list_call(name, args, span);
        }
        let builtin = match name {
            "abs" => Builtin::Abs,
            "int" => Builtin::Int,
            "float" => Builtin::Float,
            "round" => Builtin::Round,
            "floor" => Builtin::Floor,
            "ceil" => Builtin::Ceil,
            "min" => Builtin::Min,
            "max" => Builtin::Max,
            _ => {
                let d = if LATER.contains(&name) {
                    Diag::error("E-UNSUPPORTED", name_span, format!("`{name}` isn't implemented yet"))
                        .help("lists that are sorted, deduplicated or built from parts come in the next step")
                } else if matches!(name, "now" | "date" | "duration" | "timezone") {
                    Diag::error("E-UNSUPPORTED", name_span, format!("`{name}` isn't available: a program is pure"))
                        .help("pass time in as an input")
                } else if EXPR_BUILTINS.contains(&name) {
                    Diag::error("E-UNSUPPORTED", name_span, format!("`{name}` isn't available"))
                        .help("strings are compared, not built: output numbers and format them where they're displayed")
                } else {
                    let mut names: Vec<&str> = vec!["abs", "int", "float", "round", "floor", "ceil", "min", "max"];
                    names.extend(self.iface.builtins.iter().map(|b| b.name.as_str()));
                    let help = match name {
                        "pow" => "use `x ** y`".to_string(),
                        "sqrt" => "use `x ** 0.5`".to_string(),
                        "trunc" => "use `int(x)` (it truncates toward zero)".to_string(),
                        _ => match nearest(name, &names) {
                            Some(n) => format!("did you mean `{n}`?"),
                            None => format!("the builtins are {}", names.join(", ")),
                        },
                    };
                    Diag::error("E-NAME", name_span, format!("`{name}` isn't a builtin")).help(help)
                };
                self.err(d);
                for a in args {
                    self.expr(a);
                }
                return node(TKind::Nil, Ty::Unknown, span);
            }
        };
        let targs: Vec<TExpr> = args.iter().map(|a| self.expr(a)).collect();
        let arity_ok = match builtin {
            Builtin::Min | Builtin::Max => targs.len() >= 2,
            _ => targs.len() == 1,
        };
        if !arity_ok {
            let d = match builtin {
                Builtin::Min | Builtin::Max if targs.len() == 1 => Diag::error("E-UNSUPPORTED", span, format!("`{name}` of a list isn't implemented yet"))
                    .help(format!("give two or more numbers: `{name}(a, b)`")),
                Builtin::Min | Builtin::Max => Diag::error("E-ARG", span, format!("`{name}` takes two or more numbers")),
                Builtin::Round => Diag::error("E-ARG", span, "`round` takes one argument").help("to round to 2 places: `round(x * 100) / 100`"),
                _ => Diag::error("E-ARG", span, format!("`{name}` takes one argument")),
            };
            self.err(d);
            return node(TKind::Call(builtin, targs), Ty::Unknown, span);
        }
        let nums: Vec<Option<Ty>> = targs.iter().map(|a| self.number(a, name)).collect();
        if nums.iter().any(Option::is_none) {
            return node(TKind::Call(builtin, targs), Ty::Unknown, span);
        }
        let nums: Vec<Ty> = nums.into_iter().flatten().collect();
        let ty = match builtin {
            // abs(MinInt) wraps to MinInt, so its range is the full one (spec §4.1).
            Builtin::Abs => match &nums[0] {
                Ty::Int(_) => Ty::Int(Range::FULL),
                t => t.clone(),
            },
            Builtin::Int => match &nums[0] {
                Ty::Int(r) => Ty::Int(*r),
                _ => Ty::Int(Range::FULL),
            },
            Builtin::Float | Builtin::Round | Builtin::Floor | Builtin::Ceil => Ty::Float,
            Builtin::Min | Builtin::Max => {
                let mut acc = nums[0].clone();
                for t in &nums[1..] {
                    acc = match (&acc, t) {
                        (Ty::Int(x), Ty::Int(y)) => Ty::Int(if builtin == Builtin::Min { x.min(*y) } else { x.max(*y) }),
                        _ => Ty::Float,
                    };
                }
                acc
            }
        };
        node(TKind::Call(builtin, targs), ty, span)
    }

    /// The program's value must fit the output type at every position
    /// (spec §3.4). `at` names the position for messages ("" is the output itself).
    fn output(&mut self, e: Option<&TExpr>, got: &Ty, want: &Ty, span: Span, at: &str) {
        let place = if at.is_empty() { "the output".to_string() } else { format!("the output field `{at}`") };
        let field = |k: &str| if at.is_empty() { k.to_string() } else { format!("{at}.{k}") };
        match (got, want) {
            (Ty::Unknown, _) => {}
            (_, Ty::List(..)) => self.err(
                Diag::error("E-UNSUPPORTED", span, format!("{place} is a list; writing lists to the output isn't implemented yet"))
                    .help("it comes in the next step (list outputs)"),
            ),
            (Ty::Nil, Ty::Opt(_)) => {}
            (g, Ty::Opt(w)) => {
                let g = g.non_nil().unwrap_or(Ty::Nil);
                self.output(e, &g, w, span, at);
            }
            (g, _) if g.may_be_nil() => {
                let example = match g.non_nil() {
                    Some(Ty::Rec(..)) => "{…}",
                    Some(Ty::Str(_)) => "\"\"",
                    Some(Ty::Bool) => "false",
                    _ => "0",
                };
                let help = self.nil_help(e, example);
                self.err(Diag::error("E-TYPE", span, format!("{place} can't be nil, but this is {}", g.describe())).help(help))
            }
            (Ty::Rec(g, _), Ty::Rec(w, _)) => {
                // The field's own expression, when this is a record literal.
                let sub = |k: &str| match e.map(|e| &e.kind) {
                    Some(TKind::Record(fields)) => fields.iter().find(|(f, _)| f == k).map(|(_, x)| x),
                    _ => None,
                };
                for (k, wt) in w {
                    match g.get(k) {
                        Some(gt) => self.output(sub(k), gt, wt, span, &field(k)),
                        None if matches!(wt, Ty::Opt(_)) => {}
                        None => self.err(
                            Diag::error("E-FIELD", span, format!("{place} is missing the field `{k}`"))
                                .help(format!("every required field must be written, e.g. `{k}: …`")),
                        ),
                    }
                }
                for k in g.keys().filter(|k| !w.contains_key(*k)) {
                    let names: Vec<&str> = w.keys().map(String::as_str).collect();
                    let d = Diag::error("E-FIELD", span, format!("{place} has no field `{k}`"));
                    self.err(match nearest(k, &names) {
                        Some(n) => d.help(format!("did you mean `{n}`?")),
                        None => d.help(format!("its fields are: {}", names.join(", "))),
                    });
                }
            }
            (Ty::Int(g), Ty::Int(w)) => {
                if !g.within(*w) {
                    self.err(
                        Diag::error("E-RANGE", span, format!("{place} is {} but this can be {}", Ty::Int(*w).describe(), g.describe()))
                            .help(format!("keep it in range, e.g. `min(max(…, {}), {})`", w.lo, w.hi)),
                    );
                }
            }
            (Ty::Float, Ty::Int(_)) => self.err(
                Diag::error("E-TYPE", span, format!("{place} is an integer but this is a float"))
                    .help("convert it: `int(…)` truncates, `int(round(…))` rounds (`/` always gives a float)"),
            ),
            // An int is converted to float when it's written (the VM does it).
            (Ty::Int(_) | Ty::Float, Ty::Float) | (Ty::Bool, Ty::Bool) => {}
            (Ty::Str(g), Ty::Str(w)) => match (&w.values, &g.values) {
                (Some(allowed), Some(got)) => {
                    let extra: BTreeSet<String> = got.difference(allowed).cloned().collect();
                    if !extra.is_empty() {
                        self.err(
                            Diag::error("E-RANGE", span, format!("{place} is one of {}, but this can be {}", list(allowed), list(&extra)))
                                .help("use only its values, or add the new one to its `enum`"),
                        );
                    }
                }
                (Some(allowed), None) => self.err(
                    Diag::error("E-RANGE", span, format!("{place} is one of {}, but this can be any string", list(allowed)))
                        .help("choose between the values with a conditional, e.g. `c ? \"a\" : \"b\"`"),
                ),
                // Compare bytes (what a slot holds); report characters (what `maxLength` counts).
                (None, _) if g.max > w.max => self.err(
                    Diag::error("E-OUTPUT-BOUND", span, format!("{place}'s `maxLength` is {}, but this string can be {} characters", w.chars, g.chars))
                        .help(format!("raise its `maxLength` to {}", g.chars)),
                ),
                _ => {}
            },
            (g, w) => self.err(Diag::error("E-TYPE", span, format!("{place} is {} but this is {}", kind_name(w), g.describe()))),
        }
    }

    /// Every variant group in the output must have exactly one variant
    /// present (spec §3.4). `p` is the value at `want`'s position.
    fn variants<'e>(&mut self, lets: &HashMap<u32, &'e TExpr>, p: Proj<'e>, want: &Ty, at: &str) {
        let want = match want {
            Ty::Opt(w) => w,
            w => w,
        };
        let Ty::Rec(wf, wv) = want else { return };
        if let Some(v) = wv {
            self.one_of(lets, p.clone(), v, at);
        }
        for (k, wt) in wf {
            if !crate::schema::contains_group(wt) {
                continue;
            }
            let mut subs = Vec::new();
            project(lets, &p, k, &mut subs);
            let at = if at.is_empty() { k.clone() } else { format!("{at}.{k}") };
            for s in subs {
                self.variants(lets, s, wt, &at);
            }
        }
    }

    /// The value `p` must have exactly one of the variants `v` present: a
    /// record literal with one of them, which can't be nil; a value whose
    /// type has them (passed on); or a conditional, `??` or name whose every
    /// value is one of those.
    fn one_of<'e>(&mut self, lets: &HashMap<u32, &'e TExpr>, p: Proj<'e>, v: &BTreeSet<String>, at: &str) {
        let place = if at.is_empty() { "the output".to_string() } else { format!("the output field `{at}`") };
        let names = v.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ");
        let first = v.iter().next().cloned().unwrap_or_default();
        let by_type = |c: &mut Self, t: &Ty, span: Span| match t.non_nil() {
            None | Some(Ty::Unknown) => {}
            Some(Ty::Rec(f, own)) if one_of_by_type(&f, &own, v) => {}
            Some(t) => c.err(
                Diag::error("E-VARIANT", span, format!("{place} must have exactly one of {names}, but this is {} and isn't known to have exactly one", t.describe()))
                    .help(format!("build it with one variant, e.g. `{{{first}: {{…}}}}`, or pass on a value that has these variants")),
            ),
        };
        let e = match p {
            Proj::T(t, span) => return by_type(self, &t, span),
            Proj::E(e) => e,
        };
        match &e.kind {
            TKind::Record(fields) => {
                let set: Vec<&(String, TExpr)> = fields.iter().filter(|(k, x)| v.contains(k) && x.ty != Ty::Nil).collect();
                match set.as_slice() {
                    [] => self.err(
                        Diag::error("E-VARIANT", e.span, format!("{place} must have exactly one of {names}, but this sets none of them"))
                            .help(format!("set one, e.g. `{first}: {{…}}`")),
                    ),
                    [(k, x)] if x.ty.may_be_nil() => {
                        let other = v.iter().find(|o| *o != k).unwrap_or(k);
                        let test = match self.render(x) {
                            Some(r) => format!("`{r} != nil ? {{{k}: {r}}} : {{{other}: …}}`"),
                            None => format!("`c ? {{{k}: …}} : {{{other}: …}}`"),
                        };
                        self.err(
                            Diag::error("E-VARIANT", x.span, format!("{place} must have exactly one of {names}, but the variant `{k}` here may be nil"))
                                .help(format!("choose the variant with a condition, e.g. {test}")),
                        )
                    }
                    [_] => {}
                    several => {
                        let set = several.iter().map(|(k, _)| format!("`{k}`")).collect::<Vec<_>>().join(" and ");
                        self.err(
                            Diag::error("E-VARIANT", e.span, format!("{place} must have exactly one of {names}, but this sets {} of them: {set}", several.len()))
                                .help("set only one; to choose between them, use a condition: `c ? {a: …} : {b: …}`"),
                        )
                    }
                }
            }
            TKind::Cond(_, a, b) | TKind::Coalesce(a, b) => {
                self.one_of(lets, Proj::E(a), v, at);
                self.one_of(lets, Proj::E(b), v, at);
            }
            TKind::Let { body, .. } => self.one_of(lets, Proj::E(body), v, at),
            TKind::Local(id) if lets.contains_key(id) => self.one_of(lets, Proj::E(lets[id]), v, at),
            TKind::Nil => {}
            TKind::Path { base, steps, .. } if traceable(lets, base) => {
                let mut ps = vec![Proj::E(base)];
                for s in steps {
                    let mut next = Vec::new();
                    for p in &ps {
                        project(lets, p, &s.field, &mut next);
                    }
                    ps = next;
                }
                for p in ps {
                    self.one_of(lets, p, v, at);
                }
            }
            _ => by_type(self, &e.ty, e.span),
        }
    }
}

/// Whether the variant check can follow `e` to the expressions that make
/// it (rather than go by its type).
fn traceable(lets: &HashMap<u32, &TExpr>, e: &TExpr) -> bool {
    match &e.kind {
        TKind::Record(_) | TKind::Cond(..) | TKind::Coalesce(..) | TKind::Let { .. } => true,
        TKind::Local(id) => lets.contains_key(id),
        TKind::Path { base, .. } => traceable(lets, base),
        _ => false,
    }
}

/// The values at field `k` of the value `p`: a literal's field, each
/// branch's, a name's value's; or, where the expression can't be followed,
/// the field's type. A literal without the field contributes nothing (the
/// field is absent there).
fn project<'e>(lets: &HashMap<u32, &'e TExpr>, p: &Proj<'e>, k: &str, out: &mut Vec<Proj<'e>>) {
    let field_ty = |t: &Ty, span: Span, out: &mut Vec<Proj<'e>>| {
        if let Some(Ty::Rec(f, _)) = t.non_nil()
            && let Some(ft) = f.get(k)
        {
            out.push(Proj::T(ft.clone(), span));
        }
    };
    let e = match p {
        Proj::T(t, span) => return field_ty(t, *span, out),
        Proj::E(e) => *e,
    };
    match &e.kind {
        TKind::Record(fields) => {
            if let Some((_, x)) = fields.iter().find(|(f, _)| f == k) {
                out.push(Proj::E(x));
            }
        }
        TKind::Cond(_, a, b) | TKind::Coalesce(a, b) => {
            project(lets, &Proj::E(a), k, out);
            project(lets, &Proj::E(b), k, out);
        }
        TKind::Let { body, .. } => project(lets, &Proj::E(body), k, out),
        TKind::Local(id) if lets.contains_key(id) => project(lets, &Proj::E(lets[id]), k, out),
        TKind::Nil => {}
        TKind::Path { base, steps, .. } if traceable(lets, base) => {
            let mut ps = vec![Proj::E(base)];
            for s in steps {
                let mut next = Vec::new();
                for p in &ps {
                    project(lets, p, &s.field, &mut next);
                }
                ps = next;
            }
            for p in &ps {
                project(lets, p, k, out);
            }
        }
        _ => field_ty(&e.ty, e.span, out),
    }
}

/// Whether a value of type `got` can be passed where `want` is expected:
/// ints pass as floats, nil only where `want` is nullable, and a record may
/// have fields `want` doesn't mention.
fn conform(got: &Ty, want: &Ty) -> Result<(), String> {
    match (got, want) {
        (Ty::Unknown, _) => Ok(()),
        (Ty::Nil, Ty::Opt(_)) => Ok(()),
        (g, Ty::Opt(w)) => conform(&g.non_nil().unwrap_or(Ty::Nil), w),
        (g, _) if g.may_be_nil() => Err(format!("this may be nil (it's {}), but nil isn't allowed here", g.describe())),
        (Ty::Rec(g, _), Ty::Rec(w, _)) => {
            for (k, wt) in w {
                match g.get(k) {
                    Some(gt) => conform(gt, wt).map_err(|e| format!("its field `{k}`: {e}"))?,
                    None if matches!(wt, Ty::Opt(_)) => {}
                    None => return Err(format!("it has no field `{k}`")),
                }
            }
            Ok(())
        }
        (Ty::Int(_) | Ty::Float, Ty::Float) | (Ty::Int(_), Ty::Int(_)) | (Ty::Bool, Ty::Bool) | (Ty::Str(_), Ty::Str(_)) => Ok(()),
        (Ty::Float, Ty::Int(_)) => Err("an integer is needed, but this is a float".into()),
        (g, w) => Err(format!("{} is needed, but this is {}", kind_name(w), g.describe())),
    }
}

/// An accumulator type that has stopped settling: int ranges widen to full.
fn widen(t: &Ty) -> Ty {
    match t {
        Ty::Int(_) => Ty::Int(Range::FULL),
        Ty::Rec(f, g) => Ty::Rec(f.iter().map(|(k, v)| (k.clone(), widen(v))).collect(), g.clone()),
        Ty::Opt(inner) => widen(inner).nullable(),
        t => t.clone(),
    }
}

fn is_record(t: &Ty) -> bool {
    matches!(t.non_nil(), Some(Ty::Rec(..)))
}

fn kind_name(t: &Ty) -> &'static str {
    match t {
        Ty::Bool => "a bool",
        Ty::Int(_) => "an integer",
        Ty::Float => "a float",
        Ty::Str(_) => "a string",
        Ty::Rec(..) => "a record",
        _ => "a value",
    }
}

/// The closest name by edit distance, if it's close enough to be a likely typo.
fn nearest<'a>(n: &str, names: &[&'a str]) -> Option<&'a str> {
    names
        .iter()
        .map(|m| (edit_distance(n, m), *m))
        .filter(|(d, m)| *d <= 2.max(m.len() / 3))
        .min()
        .map(|(_, m)| m)
}

/// Edit distance where swapping two adjacent letters counts as one edit
/// (optimal string alignment), so `lta` is one edit from `lat`.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=b.len() {
        d[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = (a[i - 1] != b[j - 1]) as usize;
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}
