//! Static types (spec §3.1) and integer ranges (§4.1), and how a value of
//! each type is laid out as leaves (slots).

use std::collections::{BTreeMap, BTreeSet};

/// An integer range, held in i128 so arithmetic on two i64 ranges is exact.
/// A range never extends past i64: a result that could leave it (and so wrap
/// at run time) becomes the full range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub lo: i128,
    pub hi: i128,
}

impl Range {
    pub const FULL: Range = Range { lo: i64::MIN as i128, hi: i64::MAX as i128 };

    pub fn exact(v: i64) -> Range {
        Range { lo: v as i128, hi: v as i128 }
    }

    /// `lo..=hi`, or the full range if any of it lies outside i64.
    pub fn checked(lo: i128, hi: i128) -> Range {
        if lo < i64::MIN as i128 || hi > i64::MAX as i128 { Range::FULL } else { Range { lo, hi } }
    }

    pub fn union(self, o: Range) -> Range {
        Range { lo: self.lo.min(o.lo), hi: self.hi.max(o.hi) }
    }

    pub fn within(self, o: Range) -> bool {
        o.lo <= self.lo && self.hi <= o.hi
    }

    pub fn add(self, o: Range) -> Range {
        Range::checked(self.lo + o.lo, self.hi + o.hi)
    }
    pub fn sub(self, o: Range) -> Range {
        Range::checked(self.lo - o.hi, self.hi - o.lo)
    }
    pub fn mul(self, o: Range) -> Range {
        let c = [self.lo * o.lo, self.lo * o.hi, self.hi * o.lo, self.hi * o.hi];
        Range::checked(*c.iter().min().unwrap(), *c.iter().max().unwrap())
    }
    pub fn neg(self) -> Range {
        Range::checked(-self.hi, -self.lo)
    }
    pub fn min(self, o: Range) -> Range {
        Range { lo: self.lo.min(o.lo), hi: self.hi.min(o.hi) }
    }
    pub fn max(self, o: Range) -> Range {
        Range { lo: self.lo.max(o.lo), hi: self.hi.max(o.hi) }
    }

    pub fn describe(self) -> String {
        let end = |v: i128| match v {
            v if v == i64::MIN as i128 => "MinInt".to_string(),
            v if v == i64::MAX as i128 => "MaxInt".to_string(),
            v => v.to_string(),
        };
        if self.lo == self.hi { end(self.lo) } else { format!("{}..{}", end(self.lo), end(self.hi)) }
    }
}

/// A string type: at most `max` bytes (`chars` characters, as `maxLength`
/// counts them), and when `values` is known, one of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrTy {
    pub max: u32,
    pub chars: u32,
    pub values: Option<BTreeSet<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    /// The type of something that already failed to check; never reported again.
    Unknown,
    Nil,
    Bool,
    Int(Range),
    Float,
    Str(StrTy),
    /// A record. Fields are kept in name order, which is also their leaf order.
    ///
    /// The second part, when present, makes the record a **variant group**
    /// (a schema `oneOf`, spec §3.2): those fields are each an optional
    /// record, and exactly one of them is present. A type carries the set
    /// only when every value of it is known to keep that rule.
    Rec(BTreeMap<String, Ty>, Option<BTreeSet<String>>),
    /// A list of at most `bound` items. As a value it is one leaf: a handle.
    List(Box<Ty>, u32),
    /// The item type of `[]`: unifies with anything.
    Never,
    /// `T` or nil. Never wraps `Nil`, `Unknown` or another `Opt`.
    Opt(Box<Ty>),
}

/// How many leaves (slots) a value of this type occupies.
///
/// A scalar, `nil` or an optional scalar is one leaf. A record is its
/// fields' leaves in name order. An optional record is a presence leaf
/// (true, or nil when absent) followed by its fields' leaves.
pub fn leaves(t: &Ty) -> u16 {
    match t {
        Ty::Rec(f, _) => f.values().map(leaves).sum(),
        Ty::Opt(inner) => match &**inner {
            Ty::Rec(..) => 1 + leaves(inner),
            _ => 1,
        },
        _ => 1,
    }
}

/// Whether values of the two types are laid out the same way (the same
/// leaves in the same places), so one can stand in for the other without
/// rearranging. Scalars of different kinds still share a layout: a leaf is a
/// leaf.
pub fn same_layout(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        // A list is a handle: one leaf, whatever its items.
        (Ty::List(..), Ty::List(..)) => true,
        (Ty::Rec(x, _), Ty::Rec(y, _)) => x.len() == y.len() && x.iter().zip(y).all(|((k, s), (l, t))| k == l && same_layout(s, t)),
        (Ty::Opt(x), Ty::Opt(y)) if matches!(**x, Ty::Rec(..)) || matches!(**y, Ty::Rec(..)) => same_layout(x, y),
        (Ty::Rec(..), _) | (_, Ty::Rec(..)) => false,
        (Ty::Opt(x), _) | (_, Ty::Opt(x)) if matches!(**x, Ty::Rec(..)) => false,
        _ => true,
    }
}

/// Whether a record with fields `f` is, by its type alone, sure to have
/// exactly one of the variants `v` present: either it is itself a variant
/// group whose variants are among `v` and it has none of the others, or it
/// has exactly one of `v` as a field that can't be nil and none of the
/// others (a literal such as `{idle: {}}`). A field typed `nil` counts as
/// absent.
pub fn one_of_by_type(f: &BTreeMap<String, Ty>, own: &Option<BTreeSet<String>>, v: &BTreeSet<String>) -> bool {
    let absent = |k: &String| matches!(f.get(k), None | Some(Ty::Nil));
    match own {
        Some(m) if m.is_subset(v) => v.iter().filter(|k| !m.contains(*k)).all(absent),
        _ => {
            let present: Vec<&String> = v.iter().filter(|k| !absent(k)).collect();
            present.len() == 1 && matches!(f.get(present[0]), Some(Ty::Rec(..)))
        }
    }
}

/// The leaf offset of `field` within a record whose fields are `f`.
pub fn field_offset(f: &BTreeMap<String, Ty>, field: &str) -> u16 {
    f.iter().take_while(|(k, _)| k.as_str() != field).map(|(_, t)| leaves(t)).sum()
}

impl Ty {
    pub fn nullable(self) -> Ty {
        match self {
            Ty::Nil | Ty::Unknown | Ty::Opt(_) => self,
            t => Ty::Opt(Box::new(t)),
        }
    }

    /// The type without nil (`a` in `a ?? b`). `None` for plain `nil`.
    pub fn non_nil(&self) -> Option<Ty> {
        match self {
            Ty::Nil => None,
            Ty::Opt(t) => Some((**t).clone()),
            t => Some(t.clone()),
        }
    }

    pub fn may_be_nil(&self) -> bool {
        matches!(self, Ty::Nil | Ty::Opt(_))
    }

    pub fn is_number(&self) -> bool {
        matches!(self, Ty::Int(_) | Ty::Float)
    }

    /// For messages: "an integer (0..10)", "a string", "an integer or nil".
    pub fn describe(&self) -> String {
        match self {
            Ty::Unknown => "unknown".into(),
            Ty::Nil => "nil".into(),
            Ty::Bool => "a bool".into(),
            Ty::Int(r) if *r == Range::FULL => "an integer".into(),
            Ty::Int(r) => format!("an integer ({})", r.describe()),
            Ty::Float => "a float".into(),
            Ty::Str(s) => match &s.values {
                Some(v) if v.len() == 1 => format!("the string {}", list(v)),
                Some(v) => format!("one of {}", list(v)),
                None => "a string".into(),
            },
            Ty::Rec(f, None) => format!("a record {{{}}}", f.keys().cloned().collect::<Vec<_>>().join(", ")),
            Ty::Rec(f, Some(v)) => {
                let plain: Vec<&str> = f.keys().filter(|k| !v.contains(*k)).map(String::as_str).collect();
                let one = format!("one of {}", v.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", "));
                if plain.is_empty() { format!("a record with {one}") } else { format!("a record {{{}}} with {one}", plain.join(", ")) }
            }
            Ty::List(t, n) => format!("a list of up to {n} {}", match &**t {
                Ty::Never => "items".to_string(),
                t => format!("({})", t.describe()),
            }),
            Ty::Never => "nothing".into(),
            Ty::Opt(t) => format!("{} or nil", t.describe()),
        }
    }
}

pub fn list(v: &BTreeSet<String>) -> String {
    v.iter().map(|s| format!("{s:?}")).collect::<Vec<_>>().join(", ")
}

/// The type of a value that is one of `a` or `b` (branches, `??`), or
/// `None` if they don't unify (spec §3.3).
pub fn unify(a: &Ty, b: &Ty) -> Option<Ty> {
    Some(match (a, b) {
        (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
        (Ty::Never, t) | (t, Ty::Never) => t.clone(),
        // Lists unify only when their items share a layout: a list is read by
        // leaf number, so both must number their items' leaves the same way.
        (Ty::List(x, n), Ty::List(y, m)) => {
            let t = unify(x, y)?;
            if !(matches!(**x, Ty::Never) || matches!(**y, Ty::Never) || same_layout(x, y)) {
                return None;
            }
            Ty::List(Box::new(t), (*n).max(*m))
        }
        (Ty::Nil, Ty::Nil) => Ty::Nil,
        (Ty::Nil, t) | (t, Ty::Nil) => t.clone().nullable(),
        (Ty::Opt(x), y) | (y, Ty::Opt(x)) => unify(x, &y.non_nil().unwrap_or(Ty::Nil))?.nullable(),
        (Ty::Bool, Ty::Bool) => Ty::Bool,
        (Ty::Int(x), Ty::Int(y)) => Ty::Int(x.union(*y)),
        (Ty::Int(_) | Ty::Float, Ty::Int(_) | Ty::Float) => Ty::Float,
        (Ty::Str(x), Ty::Str(y)) => Ty::Str(StrTy {
            max: x.max.max(y.max),
            chars: x.chars.max(y.chars),
            values: match (&x.values, &y.values) {
                (Some(a), Some(b)) => Some(a.union(b).cloned().collect()),
                _ => None,
            },
        }),
        // Records unify field by field. A field only one side has is nil on
        // the other (an absent field reads as nil, as in expr), so it
        // becomes optional.
        // A variant group stays one when the other side is sure to keep its
        // rule too (the same group, or a record with exactly one of its
        // variants, such as `{idle: {}}`).
        (Ty::Rec(x, mx), Ty::Rec(y, my)) => {
            let keys: BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            let mut out = BTreeMap::new();
            for k in keys {
                let t = unify(x.get(k).unwrap_or(&Ty::Nil), y.get(k).unwrap_or(&Ty::Nil))?;
                out.insert(k.clone(), t);
            }
            let group = match (mx, my) {
                (Some(a), Some(b)) if a == b => Some(a.clone()),
                (Some(a), _) if one_of_by_type(y, my, a) => Some(a.clone()),
                (_, Some(b)) if one_of_by_type(x, mx, b) => Some(b.clone()),
                _ => None,
            };
            Ty::Rec(out, group)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_widen_when_they_could_wrap() {
        let big = Range { lo: 0, hi: i64::MAX as i128 };
        assert_eq!(big.add(Range::exact(1)), Range::FULL);
        assert_eq!(Range::exact(i64::MIN).neg(), Range::FULL);
        assert_eq!(Range { lo: -3, hi: 2 }.mul(Range { lo: -5, hi: 4 }), Range { lo: -12, hi: 15 });
        assert_eq!(Range::exact(5).sub(Range { lo: 1, hi: 2 }), Range { lo: 3, hi: 4 });
    }

    #[test]
    fn unification() {
        let i = Ty::Int(Range::exact(1));
        assert_eq!(unify(&i, &Ty::Float), Some(Ty::Float));
        assert_eq!(unify(&Ty::Nil, &i), Some(Ty::Opt(Box::new(i.clone()))));
        assert_eq!(unify(&Ty::Opt(Box::new(i.clone())), &Ty::Int(Range::exact(3))), Some(Ty::Opt(Box::new(Ty::Int(Range { lo: 1, hi: 3 })))));
        assert_eq!(unify(&Ty::Bool, &i), None);
    }
}
