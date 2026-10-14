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
//!
//! A plan also says what its lowering evaluates for fewer items than Go expr
//! (which builds every list in full), or a predicate for more ([`skips`]):
//! the emitter rejects each one that can fail (`E-SKIPPED`, spec §2.7).

use crate::check::{Ptr, Sink as Agg, TExpr, TKind, can_fail, children};
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
    /// The `sortBy`/`sort`/`uniq` nodes left out because the consumer
    /// doesn't need them (the order, or only which values appear): their
    /// keys are never evaluated, and their list's nil is still `R-NIL`.
    pub dropped: Vec<&'e TExpr>,
    /// `findLast`/`findLastIndex` run backwards (the last stage is a
    /// `Reverse` added for them), stopping at the first match, as Go does.
    pub backward: bool,
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
        // Go runs `findLast`'s predicate backwards from the end, stopping at
        // the first match. The loop runs backwards too (a `Reverse` at the
        // end), where the stages before the sink can be walked from the end
        // for free and can't fail (they then run only for the items the
        // predicate reaches): a list or range through `map`s and (for
        // `findLast`) `filter`s. Otherwise it runs forwards over every item.
        Agg::FindLast | Agg::FindLastIndex => {
            let mut p = plan(src, Need::Ordered);
            let total = |s: &Stage| match *s {
                Stage::Map(m) => matches!(&m.kind, TKind::Map { body, .. } if !mentions_index(body) && !can_fail(body)),
                Stage::Filter { pred, .. } => agg == Agg::FindLast && !can_fail(pred),
                _ => false,
            };
            if matches!(p.source, Source::List(_) | Source::Range(_)) && p.stages.iter().all(total) {
                p.stages.push(Stage::Reverse(src));
                p.backward = true;
            }
            return p;
        }
        Agg::Find | Agg::FindIndex | Agg::Reduce => Need::Ordered,
    };
    plan(src, need)
}

/// Whether `len`, `first`, `last`, `xs[i]` or `get` over `p` reads what it
/// needs where it lies, with no loop (emit.rs `direct`; spec §7.6): over a
/// list or range (or a hold, read as one) through windows, `reverse`s and
/// `map`s whose bodies can't fail and don't read `#index` (a body is then
/// evaluated for the item read only, or not at all for `len`: unobservable,
/// as it can't fail), or a `concat` of lists, ranges and holds through
/// windows and `reverse`s, with only windows and `reverse`s after it.
pub fn direct_ok(p: &Pipeline, sink: Agg) -> bool {
    let windows = |stages: &[Stage]| stages.iter().all(|s| matches!(s, Stage::Window(_) | Stage::Reverse(_)));
    let total_map = |s: &Stage| matches!(*s, Stage::Map(m) if matches!(&m.kind, TKind::Map { body, .. } if !mentions_index(body) && !can_fail(body)));
    if !matches!(sink, Agg::Len | Agg::First | Agg::Last | Agg::At | Agg::Get) || matches!(p.item, Ty::Never) {
        return false;
    }
    match &p.source {
        Source::Hold(h) if h.cap == 0 => false,
        Source::List(_) | Source::Range(_) | Source::Hold(_) => p.stages.iter().all(|s| windows(std::slice::from_ref(s)) || total_map(s)),
        Source::Concat(parts) => windows(&p.stages) && parts.iter().all(|q| matches!(q.source, Source::List(_) | Source::Range(_) | Source::Hold(_)) && windows(&q.stages)),
    }
}

/// An expression a lowering evaluates for fewer items than Go expr does (or,
/// for a predicate, for more), and why (spec §2.7). Go builds every list in
/// full before the builtin that takes it runs, so it evaluates every `map`
/// body, `filter` predicate and sort key for every item, and every `concat`
/// list; only `any`, `all`, `none`, `find`, `findIndex` (forwards) and
/// `findLast`, `findLastIndex` (backwards) stop their own predicate early.
pub struct Skip<'e> {
    /// The body, predicate, key or list.
    pub node: &'e TExpr,
    /// What it is, with `{}` for its text: "the `map` body `{}`", …
    pub role: &'static str,
    pub cause: Cause<'e>,
}

/// The role of a `concat`'s list (not built by a list builtin) that its
/// walk reaches only item by item.
pub const CONCAT_LIST: &str = "the list `{}` in `concat`";

#[derive(Clone, Copy)]
pub enum Cause<'e> {
    /// The sink leaves the loop early (`first`, `xs[i]`, `get`, `find`,
    /// `findIndex`, `any`, `all`, `none`, `in`, a backwards `findLast`).
    Stops(Agg),
    /// A held `uniq` stops once it has its `k` items.
    UniqFull(&'e TExpr),
    /// A window (a slice with an end, or `take`) leaves the loop at its end.
    Window(&'e TExpr),
    /// The sort is dropped: the consumer doesn't depend on the order.
    Dropped(&'e TExpr, Option<Agg>),
    /// A `filter` (or `find`'s predicate) goes under the sort, which then
    /// evaluates its key only for the items that pass.
    Filtered(&'e TExpr),
    /// More, not fewer: the predicate runs for every item, where Go stops
    /// at the first match (`find`/`findLast` over a sort, which keeps one
    /// item; a forwards `findLast`).
    EveryItem(Agg),
}

/// What the lowering of `p` for `sink` (`None`: written into a block, every
/// item in order) evaluates for fewer items than Go does, or (a predicate)
/// for more. `f` is the sink's predicate.
pub fn skips<'e>(p: &Pipeline<'e>, sink: Option<Agg>, f: Option<&'e TExpr>) -> Vec<Skip<'e>> {
    let mut out = Vec::new();
    let looped = !sink.is_some_and(|s| direct_ok(p, s));
    // (`xs[i]` and `get` with an index that is always negative count the
    // whole pipeline first, by a replay that evaluates all of it.)
    let negative = f.is_some_and(|i| matches!(i.ty, Ty::Int(r) if r.hi < 0));
    let stops = sink.filter(|s| {
        matches!(s, Agg::First | Agg::Find | Agg::FindIndex | Agg::Any | Agg::All | Agg::NoneOf | Agg::In)
            || (matches!(s, Agg::At | Agg::Get) && !negative)
            || (matches!(s, Agg::FindLast | Agg::FindLastIndex) && p.backward)
    });
    level(p, sink, looped, stops.map(Cause::Stops), false, &mut out);
    // A forwards `findLast` runs its predicate for every item (unless it
    // reads a one-item hold, whose own predicate is accounted for).
    if let (Some(s @ (Agg::FindLast | Agg::FindLastIndex)), Some(f), false) = (sink, f, p.backward)
        && !pushed_find(p)
    {
        out.push(Skip { node: f, role: role_of_pred(s), cause: Cause::EveryItem(s) });
    }
    out
}

fn role_of_pred(s: Agg) -> &'static str {
    match s {
        Agg::Find => "`find`'s predicate `{}`",
        Agg::FindLast => "`findLast`'s predicate `{}`",
        Agg::FindLastIndex => "`findLastIndex`'s predicate `{}`",
        _ => "the predicate `{}`",
    }
}

/// `find`/`findLast` over a sort: the predicate went under the sort.
fn pushed_find(p: &Pipeline) -> bool {
    matches!(&p.source, Source::Hold(h) if h.inner.stages.iter().any(|s| matches!(s, Stage::Filter { node, .. } if std::ptr::eq(*node, h.node))))
}

/// The skips at one pipeline level (`p`, its holds, its parts, the
/// pipelines nested in its items). `exit`: the loop over `p` may be left
/// early, for this cause, skipping everything evaluated per item.
/// `rev_after`: the items leave `p` backwards (an odd number of `reverse`s
/// follow it, in the pipelines it is part of).
fn level<'e>(p: &Pipeline<'e>, sink: Option<Agg>, looped: bool, exit: Option<Cause<'e>>, rev_after: bool, out: &mut Vec<Skip<'e>>) {
    let odd = |stages: &[Stage]| stages.iter().filter(|s| matches!(s, Stage::Reverse(_))).count() % 2 == 1;
    let all_rev = rev_after ^ odd(&p.stages);
    for d in &p.dropped {
        if let TKind::SortBy { key: Some(k), .. } = &d.kind {
            out.push(Skip { node: k, role: "the sort key `{}`", cause: Cause::Dropped(d, sink) });
        }
    }
    match &p.source {
        Source::Hold(h) => hold(h, out),
        Source::Concat(parts) => {
            for q in parts {
                // Each part's own windows leave only the part's loop.
                level(q, sink, looped, None, all_rev, out);
            }
        }
        _ => {}
    }
    // An empty hold, or a list with no items: no loop.
    if !looped || matches!(p.item, Ty::Never) || matches!(&p.source, Source::Hold(h) if h.cap == 0) {
        return;
    }
    // A window with an end leaves this level's loops there: what the items
    // went through before it is skipped for the items after. Not where its
    // bounds are counted first by a replay of what comes before it (a bound
    // that may be negative, or items that reach it backwards, where its
    // start is turned into the end): the replay evaluates all of that for
    // every item. (A window straight on a list or range has nothing before
    // it.)
    let mut rev = all_rev;
    for (j, s) in p.stages.iter().enumerate() {
        let back = rev;
        let w = match *s {
            Stage::Reverse(_) => {
                rev = !rev;
                continue;
            }
            Stage::Window(w) => w,
            _ => continue,
        };
        let ends = !back
            && match &w.kind {
                TKind::Slice { lo, hi, .. } => hi.is_some() && !lo.as_deref().is_some_and(may_be_negative) && !hi.as_deref().is_some_and(may_be_negative),
                TKind::Take { .. } => true,
                _ => false,
            };
        if ends {
            per_item(p, j, &mut |node, role| out.push(Skip { node, role, cause: Cause::Window(w) }));
        }
    }
    if let Some(c) = exit {
        per_item(p, p.stages.len(), &mut |node, role| out.push(Skip { node, role, cause: c }));
    }
    // The pipelines nested in each item (`flatten(map(xs, <pipeline>))`),
    // their items leaving backwards when the stages after them reverse.
    for (j, s) in p.stages.iter().enumerate() {
        if let Stage::FlatMap(m) = s
            && let Some((_, body)) = flat_map_body(m)
        {
            level(&for_write(body), sink, true, None, rev_after ^ odd(&p.stages[j + 1..]), out);
        }
    }
}

/// Whether a slice bound or index may be negative (counting from the end).
pub fn may_be_negative(e: &TExpr) -> bool {
    match &e.ty {
        Ty::Int(r) => r.lo < 0,
        _ => true,
    }
}

/// A hold is lowered in full, before the loop that reads it: its own loop
/// stops early only for a `uniq` that has its `k` items. A `filter` (or
/// `find`'s predicate) moved under a sort means the key is evaluated only
/// for the items that pass, and `find`'s predicate for every item.
fn hold<'e>(h: &Hold<'e>, out: &mut Vec<Skip<'e>>) {
    let bound = match h.node.ty.non_nil() {
        Some(Ty::List(_, n)) => n,
        _ => 0,
    };
    let exit = (matches!(h.kind, HoldKind::Unique) && h.cap < bound).then_some(Cause::UniqFull(h.node));
    level(&h.inner, None, true, exit, false, out);
    if let HoldKind::Sorted { key, .. } = h.kind {
        for s in &h.inner.stages {
            let Stage::Filter { node, pred } = *s else { continue };
            let find = std::ptr::eq(node, h.node);
            let under = find || matches!(&node.kind, TKind::Filter { src, .. } if std::ptr::eq(&**src, h.node));
            if !under {
                continue;
            }
            if let Some(k) = key {
                out.push(Skip { node: k, role: "the sort key `{}`", cause: Cause::Filtered(h.node) });
            }
            if find {
                out.push(Skip { node: pred, role: "the predicate `{}`", cause: Cause::EveryItem(Agg::Find) });
            }
        }
    }
}

/// Everything evaluated once per item in `p`'s loops, by the stages before
/// stage `upto` (and every `concat` part's list and stages, which are
/// reached item by item): `map` bodies (with what a flat-map's body
/// evaluates), `filter` predicates, and the lists of a `concat`'s parts that
/// aren't evaluated before the loops (those that can't be nil).
fn per_item<'e>(p: &Pipeline<'e>, upto: usize, f: &mut dyn FnMut(&'e TExpr, &'static str)) {
    if let Source::Concat(parts) = &p.source {
        for q in parts {
            match &q.source {
                Source::List(s) if !s.ty.may_be_nil() && !matches!(q.item, Ty::Never) => f(s, CONCAT_LIST),
                Source::Range(r) => f(r, "the range `{}` in `concat`"),
                _ => {}
            }
            per_item(q, q.stages.len(), f);
        }
    }
    for s in &p.stages[..upto] {
        match *s {
            Stage::Map(m) | Stage::FlatMap(m) => {
                if let TKind::Map { body, .. } = &m.kind {
                    f(body, "the `map` body `{}`");
                }
            }
            Stage::Filter { node, pred } => f(pred, if matches!(node.kind, TKind::Filter { .. }) { "the `filter` predicate `{}`" } else { "the predicate `{}`" }),
            _ => {}
        }
    }
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
                Need::Unordered | Need::Set => dropped(plan(src, need), e),
                Need::Head(k) => with_cap(held(e, plan(src, Need::Ordered), sorted(false)), k),
                Need::Tail(k) => with_cap(held(e, plan(src, Need::Ordered), sorted(true)), k),
                Need::Ordered => held(e, plan(src, Need::Ordered), sorted(false)),
            }
        }
        TKind::Uniq { src } => match need {
            // `first(uniq(xs))` is `first(xs)`; `min`, `max` and `in` see the same values.
            Need::Set | Need::Head(1) => dropped(plan(src, need), e),
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

fn dropped<'e>(mut p: Pipeline<'e>, node: &'e TExpr) -> Pipeline<'e> {
    p.dropped.push(node);
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
        Pipeline { source, stages: Vec::new(), item, pre: None, convert: None, dropped: Vec::new(), backward: false }
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
