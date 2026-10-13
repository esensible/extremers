//! The pipeline IR (spec §6.1, §7.6): fusion by construction.
//!
//! Every list a list builtin consumes, and every list kept as a value, is
//! lowered to one [`Pipeline`]: a **source**, then **stages**, then one
//! **sink**. The planner here builds it from the typed tree,
//! deterministically, and decides every hold; the emitter (`emit.rs`)
//! lowers it by one routine into loops. Memory is read off the plan the
//! emitter actually lowers, never off a rewrite that might fire.
//!
//! - **Sources** ([`Source`]): a list value (an input list, a literal, a
//!   held list, or any expression giving a handle), a range `a..b` (a
//!   generator: `a + i`), a `concat` of pipelines, or a [`Hold`].
//! - **Stages** ([`Stage`]): `map`, `filter`, a window (a slice or `take`),
//!   `reverse` (the loops run backwards: no memory), `flatten` (a nested
//!   loop over each item's list), and flat-map (`flatten(map(xs, <a
//!   pipeline>))`: the inner pipeline runs per item, never held).
//! - **Sinks** ([`Sink`]): an aggregate (`sum`, `count`, `first`, `find`,
//!   `reduce`, `median`, …), or a block write (a list kept as a value, or a
//!   hold's block).
//! - **Holds** ([`Hold`]): a breaker's input materialised into a block of
//!   locals, then read from it as a random-access list. A hold is inserted
//!   only where a rule requires it, and its size is the least the consumer
//!   allows ([`Need`]): `sortBy`/`sort` hold their input's bound, unless the
//!   consumer needs only the first or last `k` items (`first`, `last`,
//!   `take(…, k)`, `[:k]`, `[-k:]`, `[c]`: a k-selection, `k` items), or
//!   ignores order (`sum`, `count`, `min`, `max`, `any`, …: the sort is
//!   dropped, 0 items). `uniq` holds its input's bound, `k` items under a
//!   head window, and nothing under `first`, `min`, `max` or `in`. A second
//!   `reverse` in one loop holds what the first one reversed.
//!
//! A `let` of a list isn't stored (the checker inlines it): each use plans
//! its own pipeline, so each use is fused, or holds, on its own.

use crate::check::{Ptr, Sink as Agg, TExpr, TKind, children};
use crate::types::{Range, Ty};

/// A source, its stages (innermost first), and what consumes the items.
pub struct Pipeline<'e> {
    pub source: Source<'e>,
    pub stages: Vec<Stage<'e>>,
    /// The source's items' type.
    pub item: Ty,
    /// Set by the emitter: the source list's handle, in this local.
    pub pre: Option<u16>,
    /// Set by the emitter, for a `concat` part laid out differently from the
    /// whole: (the part's items' type, the whole's, the local the converted
    /// item goes in).
    pub convert: Option<(Ty, Ty, u16)>,
}

pub enum Source<'e> {
    /// A list value: its expression gives a handle.
    List(&'e TExpr),
    /// `a..b` (the `Range` node): the items `a`, `a + 1`, …, `b`.
    Range(&'e TExpr),
    /// Each part's items in turn, each through its own stages.
    Concat(Vec<Pipeline<'e>>),
    /// A breaker's input, held in a block first.
    Hold(Box<Hold<'e>>),
}

/// A hold: `inner`'s items written into a block of `cap` items (kept in the
/// order `kind` says), which the pipeline then reads as a list.
pub struct Hold<'e> {
    /// The node held (`sortBy`, `uniq`, …), for its span and messages.
    pub node: &'e TExpr,
    pub inner: Pipeline<'e>,
    pub kind: HoldKind<'e>,
    pub cap: u32,
}

#[derive(Clone, Copy)]
pub enum HoldKind<'e> {
    /// In arrival order.
    Plain,
    /// Kept sorted by `key` (the item itself when `None`) as items arrive,
    /// stably. `tail`: keep the last `cap` of the sorted order, else the
    /// first `cap` (all of them when `cap` is the input's bound).
    Sorted { key: Option<&'e TExpr>, desc: bool, tail: bool },
    /// First occurrences only (`==`), in order; at most `cap` of them.
    Unique,
}

#[derive(Clone, Copy)]
pub enum Stage<'e> {
    /// A `map` node.
    Map(&'e TExpr),
    /// A predicate, and the node it came from (a `filter`, or a `find`
    /// pushed under a sort).
    Filter { node: &'e TExpr, pred: &'e TExpr },
    /// A slice or `take` node.
    Window(&'e TExpr),
    /// A `reverse` node: the stages before it, and the source, run backwards.
    Reverse(&'e TExpr),
    /// A `flatten` node over items that are lists (handles).
    Flatten(&'e TExpr),
    /// `flatten(map(xs, body))` where `body` is a pipeline (under `let`s):
    /// the `map` node. Per item, `body`'s own pipeline runs, nested.
    FlatMap(&'e TExpr),
}

impl Stage<'_> {
    pub fn node(&self) -> &TExpr {
        match *self {
            Stage::Map(n) | Stage::Filter { node: n, .. } | Stage::Window(n) | Stage::Reverse(n) | Stage::Flatten(n) | Stage::FlatMap(n) => n,
        }
    }
}

/// What the consumer of a list needs of it: this decides each hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// Every item, in order.
    Ordered,
    /// Every item, in any order (duplicates count).
    Unordered,
    /// Only which values appear (`min`, `max`, `in`).
    Set,
    /// Only the first `k` items, in order.
    Head(u32),
    /// Only the last `k` items, in order.
    Tail(u32),
}

/// The pipeline for a sink `agg` (with its predicate, mapped expression or
/// index `f`) over `src`.
pub fn for_sink<'e>(agg: Agg, f: Option<&'e TExpr>, src: &'e TExpr) -> Pipeline<'e> {
    let need = match agg {
        Agg::Len | Agg::Count | Agg::Sum | Agg::Mean | Agg::Median | Agg::Any | Agg::All | Agg::NoneOf | Agg::One => Need::Unordered,
        Agg::Min | Agg::Max | Agg::In => Need::Set,
        Agg::First => Need::Head(1),
        Agg::Last => Need::Tail(1),
        Agg::At | Agg::Get => match f.map(|i| &i.ty) {
            Some(Ty::Int(r)) if r.lo == r.hi && r.lo >= 0 && r.lo < u32::MAX as i128 => Need::Head(r.lo as u32 + 1),
            Some(Ty::Int(r)) if r.lo == r.hi && r.lo < 0 && -r.lo < u32::MAX as i128 => Need::Tail((-r.lo) as u32),
            _ => Need::Ordered,
        },
        // `find(sortBy(xs, k), p)` is the first of the sorted items that pass
        // `p`: the predicate goes under the sort, which keeps one item.
        Agg::Find | Agg::FindLast if matches!(src.kind, TKind::SortBy { .. }) => {
            let need = if agg == Agg::Find { Need::Head(1) } else { Need::Tail(1) };
            let mut p = plan(src, need);
            if let Source::Hold(h) = &mut p.source {
                h.inner.stages.push(Stage::Filter { node: src, pred: f.unwrap() });
            }
            return p;
        }
        Agg::Find | Agg::FindLast | Agg::FindIndex | Agg::FindLastIndex | Agg::Reduce => Need::Ordered,
    };
    plan(src, need)
}

/// The pipeline for a list kept as a value (written into a block).
pub fn for_write(src: &TExpr) -> Pipeline<'_> {
    plan(src, Need::Ordered)
}

/// Plans `e` for a consumer that needs `need` of it.
pub fn plan(e: &TExpr, need: Need) -> Pipeline<'_> {
    match &e.kind {
        TKind::Map { src, body } => {
            let index = mentions_index(body);
            let below = match need {
                Need::Head(k) => Need::Head(k),
                Need::Tail(k) if !index => Need::Tail(k),
                Need::Unordered | Need::Set if !index => Need::Unordered,
                _ => Need::Ordered,
            };
            stage(plan(src, below), Stage::Map(e))
        }
        TKind::Filter { src, pred } => {
            // A filter straight on a sort commutes with it: it goes under the
            // sort, so a k-selection still keeps only k items.
            if matches!(src.kind, TKind::SortBy { .. }) && matches!(need, Need::Head(_) | Need::Tail(_)) {
                let mut p = plan(src, need);
                if let Source::Hold(h) = &mut p.source {
                    h.inner.stages.push(Stage::Filter { node: e, pred });
                    return p;
                }
                return stage(p, Stage::Filter { node: e, pred });
            }
            let below = match need {
                Need::Unordered | Need::Set => Need::Unordered,
                _ => Need::Ordered,
            };
            stage(plan(src, below), Stage::Filter { node: e, pred })
        }
        TKind::Slice { src, lo, hi } => {
            let r = |b: &Option<Box<TExpr>>| b.as_ref().map(|b| int_range(b));
            let below = match (r(lo), r(hi)) {
                // [a:b] with neither end counted from the back: the first hi(b).
                (None | Some(Some(Range { lo: 0.., .. })), Some(Some(h))) if h.lo >= 0 => Need::Head(cap(h.hi)),
                // [-k:]: the last k.
                (Some(Some(l)), None) if l.hi < 0 => Need::Tail(cap(-l.lo)),
                _ => Need::Ordered,
            };
            stage(plan(src, below), Stage::Window(e))
        }
        TKind::Take { src, k } => {
            let below = match int_range(k) {
                Some(r) => Need::Head(cap(r.hi.max(0))),
                None => Need::Ordered,
            };
            stage(plan(src, below), Stage::Window(e))
        }
        TKind::Reverse { src } => {
            let below = match need {
                Need::Head(k) => Need::Tail(k),
                Need::Tail(k) => Need::Head(k),
                n => n,
            };
            let p = plan(src, below);
            // One reversal per loop: a second one reverses a held block.
            let p = if p.stages.iter().any(|s| matches!(s, Stage::Reverse(_))) { held(src, p, HoldKind::Plain) } else { p };
            stage(p, Stage::Reverse(e))
        }
        TKind::Flatten { src } => {
            let below = match need {
                Need::Unordered | Need::Set => Need::Unordered,
                _ => Need::Ordered,
            };
            let mut p = plan(src, below);
            if !matches!(item_of(&src.ty), Ty::List(..)) {
                // Items that aren't lists: `flatten` gives them as they are.
                return p;
            }
            match p.stages.last() {
                Some(Stage::Map(m)) if flat_map_body(m).is_some() => {
                    let m = *m;
                    p.stages.pop();
                    stage(p, Stage::FlatMap(m))
                }
                _ => stage(p, Stage::Flatten(e)),
            }
        }
        TKind::SortBy { src, key, desc } => {
            let sorted = |tail| HoldKind::Sorted { key: key.as_deref(), desc: *desc, tail };
            match need {
                // Order doesn't matter: the sort is dropped.
                Need::Unordered | Need::Set => plan(src, need),
                Need::Head(k) => with_cap(held(e, plan(src, Need::Ordered), sorted(false)), k),
                Need::Tail(k) => with_cap(held(e, plan(src, Need::Ordered), sorted(true)), k),
                Need::Ordered => held(e, plan(src, Need::Ordered), sorted(false)),
            }
        }
        TKind::Uniq { src } => match need {
            // `first(uniq(xs))` is `first(xs)`; `min`, `max` and `in` see the same values.
            Need::Set | Need::Head(1) => plan(src, need),
            Need::Head(k) => with_cap(held(e, plan(src, Need::Ordered), HoldKind::Unique), k),
            _ => held(e, plan(src, Need::Ordered), HoldKind::Unique),
        },
        TKind::Concat(parts) => {
            let below = match need {
                Need::Unordered | Need::Set => Need::Unordered,
                _ => Need::Ordered,
            };
            Pipeline::new(Source::Concat(parts.iter().map(|p| plan(p, below)).collect()), item_of(&e.ty))
        }
        TKind::Range { .. } => Pipeline::new(Source::Range(e), item_of(&e.ty)),
        _ => Pipeline::new(Source::List(e), item_of(&e.ty)),
    }
}

fn stage<'e>(mut p: Pipeline<'e>, s: Stage<'e>) -> Pipeline<'e> {
    p.stages.push(s);
    p
}

/// `inner`'s items held for `node`, at `node`'s bound.
fn held<'e>(node: &'e TExpr, inner: Pipeline<'e>, kind: HoldKind<'e>) -> Pipeline<'e> {
    let item = inner.out_item();
    let cap = match node.ty.non_nil() {
        Some(Ty::List(_, n)) => n,
        _ => 0,
    };
    Pipeline::new(Source::Hold(Box::new(Hold { node, inner, kind, cap })), item)
}

/// A hold that keeps at most `k` items.
fn with_cap(mut p: Pipeline<'_>, k: u32) -> Pipeline<'_> {
    if let Source::Hold(h) = &mut p.source {
        h.cap = h.cap.min(k);
    }
    p
}

impl<'e> Pipeline<'e> {
    pub fn new(source: Source<'e>, item: Ty) -> Pipeline<'e> {
        Pipeline { source, stages: Vec::new(), item, pre: None, convert: None }
    }

    /// The type of the items that come out of the stages.
    pub fn out_item(&self) -> Ty {
        self.stages.iter().fold(self.item.clone(), |t, s| match s {
            Stage::Map(m) => match &m.kind {
                TKind::Map { body, .. } => body.ty.clone(),
                _ => t,
            },
            Stage::Flatten(_) => item_of(&t),
            Stage::FlatMap(m) => match &m.kind {
                TKind::Map { body, .. } => item_of(&body.ty),
                _ => t,
            },
            _ => t,
        })
    }
}

/// A `map` body that `flatten` can run as a nested pipeline: a pipeline,
/// possibly under `let`s. Gives the `let`s (outermost first) and the pipeline.
pub fn flat_map_body(m: &TExpr) -> Option<(Vec<&TExpr>, &TExpr)> {
    let TKind::Map { body, .. } = &m.kind else { return None };
    let mut lets = Vec::new();
    let mut b: &TExpr = body;
    loop {
        match &b.kind {
            TKind::Let { body, .. } => {
                lets.push(b);
                b = body;
            }
            _ if crate::check::is_pipeline(b) => return Some((lets, b)),
            _ => return None,
        }
    }
}

/// A list type's items' type (`Never` for nothing known).
pub fn item_of(t: &Ty) -> Ty {
    match t.non_nil() {
        Some(Ty::List(item, _)) => *item,
        _ => Ty::Never,
    }
}

fn int_range(e: &TExpr) -> Option<Range> {
    match &e.ty {
        Ty::Int(r) => Some(*r),
        _ => None,
    }
}

fn cap(v: i128) -> u32 {
    v.clamp(0, u32::MAX as i128) as u32
}

/// Whether `e` reads `#index` anywhere (its own, or a nested predicate's).
pub fn mentions_index(e: &TExpr) -> bool {
    match &e.kind {
        TKind::Pointer { which: Ptr::Index, .. } => true,
        _ => children(e).into_iter().any(mentions_index),
    }
}
