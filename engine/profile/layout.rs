//! The device tables of a bundle (the plan's A3, A4): what a host needs to
//! run an engine without the compiler, worked out here from its interface,
//! its compiled program and the bitwire descriptors of its view and events.
//!
//! - `layout.bin`: where the host's inputs and outputs are among the
//!   program's leaves ([`LAYOUT_BYTES`] bytes, u16 little-endian fields,
//!   absolute leaf numbers, [`NONE`] for none). With lists in the state
//!   (version 2) the list table follows: `count u16, gps_list u16`, then
//!   per state list `{list u16, leaf u16, max_items u16, item_leaves u16}`
//!   ([`LIST_ENTRY_BYTES`]): the program's list id, its handle's leaf in
//!   the `state` input (absolute), and the input list's bound and item
//!   leaves; `gps_list` is `gps`'s list id. Without state lists the layout
//!   is version 1, the fixed part alone, `gps` list 0.
//! - `view.map`, `event.map`: one entry per descriptor node, pre-order, that
//!   ties the node to its leaf, so a bitwire `Source`/`Sink` can follow the
//!   walk with a cursor: `node_count u32`, then per node `{leaf u16, span
//!   u16, aux u16}` (leaf relative to the published value's first leaf, or
//!   the event's; item-relative inside an array; `span` the node's subtree
//!   in entries, itself included; `aux` an enum's first string in
//!   `enums.bin`).
//! - `enums.bin`: the enum nodes' strings, in node order then index order:
//!   `count u16, 0 u16`, `count × {off u32, len u16, 0 u16}` (`off` from the
//!   start of the section), then the bytes.
//!
//! The descriptor is walked in its declared order beside the compiler's
//! type (fields in name order), matching fields by name. A node's leaf: a
//! scalar's value leaf; an object's or a record's presence leaf when it is
//! an optional record, else [`NONE`]; a variant's presence leaf; an array's
//! list-handle leaf. Unions, byte strings, and lists and free (non-enum)
//! strings in events aren't supported (v1).

use bitwire::{Descriptor, Names, Node, Op};
use compiler::schema::parse_interface;
use compiler::types::{StrTy, Ty, field_offset, leaf_offset, leaves, same_layout};
use serde_json::Value as Json;

/// No leaf.
pub const NONE: u16 = 0xFFFF;
/// `layout.bin`'s version with a list table (state lists); one without
/// state lists is [`LAYOUT_V1`].
pub const LAYOUT_VERSION: u16 = 2;
/// The version of a layout without state lists: the fixed part alone.
pub const LAYOUT_V1: u16 = 1;
/// `layout.bin`'s fixed part: 35 fields, padding to 72, the interface hash.
pub const LAYOUT_BYTES: usize = 80;
/// The list table's header (`count`, `gps_list`).
pub const LISTS_HEADER_BYTES: usize = 4;
/// Bytes per list table entry.
pub const LIST_ENTRY_BYTES: usize = 8;
/// Bytes per map entry.
pub const MAP_ENTRY_BYTES: usize = 6;

/// `layout.bin`'s fields, in order (each a u16 at twice its index), then
/// the interface hash at 72.
pub const FIELDS: [&str; 35] = [
    "version", "input_count", "output_count", "list_count", "in_state", "state_leaves", "in_event", "event_leaves",
    "in_timestamp", "in_gps", "out_state", "out_view", "view_leaves", "out_wake", "pub_base", "pub_leaves", "ev_gps",
    "ev_gps_fix", "ev_gps_fix_t", "ev_gps_fix_lat", "ev_gps_fix_lon", "ev_gps_fix_alt", "ev_gps_velocity",
    "ev_gps_velocity_speed", "ev_gps_velocity_heading", "ev_wake", "fix_leaves", "fix_t", "fix_lat", "fix_lon",
    "fix_alt", "gps_max_items", "pub_max_bytes", "event_max_bytes", "state_free_strings",
];

/// Where the interface hash sits in `layout.bin`.
pub const HASH_AT: usize = 72;

/// A bundle's device tables.
#[derive(Clone, Debug)]
pub struct Tables {
    /// `layout.bin`: the fixed part ([`LAYOUT_BYTES`]), then the list table
    /// when there are state lists.
    pub layout: Vec<u8>,
    pub view_map: Vec<u8>,
    /// With author events.
    pub event_map: Option<Vec<u8>>,
    /// When a descriptor has an enum.
    pub enums: Option<Vec<u8>>,
}

/// A list in the state, as the list table has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateList {
    /// The program's list id.
    pub list: u16,
    /// Its handle's leaf in the `state` input (absolute).
    pub leaf: u16,
    pub max_items: u16,
    pub item_leaves: u16,
}

/// `layout.bin`'s fixed fields by name, in [`FIELDS`] order (`layout` at
/// least [`LAYOUT_BYTES`] long).
pub fn layout_fields(layout: &[u8]) -> Vec<(&'static str, u16)> {
    FIELDS.iter().enumerate().map(|(i, n)| (*n, u16::from_le_bytes([layout[2 * i], layout[2 * i + 1]]))).collect()
}

/// `layout.bin`'s list table: `gps`'s list id and the state lists (list 0
/// and none for a version 1 layout). `None` if the table is cut short.
pub fn layout_lists(layout: &[u8]) -> Option<(u16, Vec<StateList>)> {
    let table = layout.get(LAYOUT_BYTES..)?;
    if table.is_empty() {
        return Some((0, Vec::new()));
    }
    let at = |i: usize| table.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let (count, gps) = (at(0)? as usize, at(2)?);
    let lists = (0..count)
        .map(|k| {
            let e = LISTS_HEADER_BYTES + k * LIST_ENTRY_BYTES;
            Some(StateList { list: at(e)?, leaf: at(e + 2)?, max_items: at(e + 4)?, item_leaves: at(e + 6)? })
        })
        .collect::<Option<Vec<_>>>()?;
    Some((gps, lists))
}

/// The type at the field path `path` inside `t` (through optional records).
fn at_path<'t>(mut t: &'t Ty, path: &[String]) -> Option<&'t Ty> {
    for k in path {
        let Ty::Rec(fields, _) = compiler::types::unopt(t) else { return None };
        t = fields.get(k)?;
    }
    Some(t)
}

/// The tables for an engine: `interface` as the compiler took it, `program`
/// the compiled bytecode (its interface hash and counts must be the
/// interface's), `view_desc` the descriptor of the published value (the
/// view, or the state without one), `event_desc` that of the author's events.
pub fn tables(interface: &Json, program: &[u8], view_desc: &[u8], event_desc: Option<&[u8]>) -> Result<Tables, String> {
    let iface = parse_interface(interface).map_err(|d| format!("the interface: {}", d.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")))?;
    let p = bytecode::Program::parse(program).map_err(|e| format!("the program: {e:?}"))?;
    if p.interface_hash != iface.hash {
        return Err("the program was compiled against another interface (its interface hash differs)".into());
    }
    let input_count: u32 = iface.inputs.iter().map(|i| leaves(&i.ty) as u32).sum();
    if p.input_count as u32 != input_count || p.output_count != leaves(&iface.output) || p.list_count as usize != iface.lists.len() {
        return Err("the program's leaf counts aren't the interface's".into());
    }

    let input = |name: &str| iface.inputs.iter().find(|i| i.name == name).ok_or_else(|| format!("the interface has no `{name}` input"));
    let (state, event, timestamp, gps) = (input("state")?, input("event")?, input("timestamp")?, input("gps")?);
    let Ty::Opt(state_ty) = &state.ty else { return Err("the `state` input is nullable".into()) };
    let Ty::Rec(out_fields, _) = &iface.output else { return Err("the output is a record".into()) };
    let out_state = out_fields.get("state").ok_or("the output has no `state`")?;
    if !same_layout(state_ty, out_state) {
        return Err("the `state` input and the output's `state` are laid out differently".into());
    }
    let view = out_fields.get("view");
    let published = view.unwrap_or(out_state);
    let at = |t: &Ty, path: &[&str]| leaf_offset(t, path).ok_or_else(|| format!("no `{}`", path.join(".")));
    let out = |path: &str| at(&iface.output, &[path]);
    let ev = |path: &[&str]| at(&event.ty, path).map(|l| event.base + l);
    let as_u16 = |what: &str, v: u64| u16::try_from(v).map_err(|_| format!("{what} is {v}, more than a u16"));

    // The input lists: `gps`, and the lists in the state.
    let position = |name: &str| iface.inputs.iter().position(|i| i.name == name).unwrap();
    let (gps_input, state_input) = (position("gps"), position("state"));
    let mut gps_list = None;
    let mut state_lists = Vec::new();
    for (id, l) in iface.lists.iter().enumerate() {
        let id = id as u16;
        let name = if l.path.is_empty() { iface.inputs[l.input].name.clone() } else { format!("{}.{}", iface.inputs[l.input].name, l.path.join(".")) };
        if l.input == gps_input && l.path.is_empty() {
            gps_list = Some(id);
        } else if l.input == state_input {
            // The output's list must fit the input's: the host keeps it for
            // the next evaluation's input.
            let Some(Ty::List(_, out_max)) = at_path(out_state, &l.path).map(compiler::types::unopt) else {
                return Err(format!("`{name}` isn't a list in the output's `state`"));
            };
            if *out_max > l.max {
                return Err(format!("the output's `{name}` holds up to {out_max} items, the input's up to {}: the next evaluation couldn't take it", l.max));
            }
            if matches!(compiler::types::unopt(&l.item), Ty::List(..)) {
                return Err(format!("`{name}`: lists in list items aren't supported"));
            }
            state_lists.push(StateList { list: id, leaf: l.leaf, max_items: as_u16(&format!("`{name}` maxItems"), l.max as u64)?, item_leaves: leaves(&l.item) });
        } else {
            return Err(format!("`{name}`: the host gives only `gps` and lists in the state"));
        }
    }
    let gps_list = gps_list.ok_or("the interface has no `gps` list")?;
    let fix = &iface.lists[gps_list as usize].item;
    let fix_at = |k: &str| at(fix, &[k]);

    let mut w = Walker { names: None, entries: Vec::new(), enums: Vec::new(), path: Vec::new() };
    let view_d = Descriptor::load(view_desc).map_err(|e| format!("the view descriptor: {e}"))?;
    let (view_map, pub_max) = w.map(&view_d, published, false)?;
    let (event_map, event_max) = match event_desc {
        None => (None, 0),
        Some(b) => {
            let d = Descriptor::load(b).map_err(|e| format!("the event descriptor: {e}"))?;
            let (m, max) = w.map(&d, &event.ty, true)?;
            (Some(m), max)
        }
    };
    let out_view = view.map(|_| out("view")).transpose()?;
    let pub_base = out_view.map_or_else(|| out("state"), Ok)?;
    let fields: [u16; 35] = [
        if state_lists.is_empty() { LAYOUT_V1 } else { LAYOUT_VERSION },
        p.input_count,
        p.output_count,
        p.list_count,
        state.base,
        leaves(state_ty),
        event.base,
        leaves(&event.ty),
        timestamp.base,
        gps.base,
        out("state")?,
        out_view.unwrap_or(NONE),
        view.map_or(0, leaves),
        out("wake")?,
        pub_base,
        leaves(published),
        ev(&["gps"])?,
        ev(&["gps", "fix"])?,
        ev(&["gps", "fix", "t"])?,
        ev(&["gps", "fix", "lat"])?,
        ev(&["gps", "fix", "lon"])?,
        ev(&["gps", "fix", "alt"])?,
        ev(&["gps", "velocity"])?,
        ev(&["gps", "velocity", "speed"])?,
        ev(&["gps", "velocity", "heading"])?,
        ev(&["wake"])?,
        leaves(fix),
        fix_at("t")?,
        fix_at("lat")?,
        fix_at("lon")?,
        fix_at("alt")?,
        as_u16("gps maxItems", iface.lists[gps_list as usize].max as u64)?,
        as_u16("the published value's largest encoding", pub_max)?,
        as_u16("the largest event encoding", event_max)?,
        free_strings(state_ty),
    ];
    let mut layout = vec![0u8; LAYOUT_BYTES];
    for (i, f) in fields.iter().enumerate() {
        layout[2 * i..2 * i + 2].copy_from_slice(&f.to_le_bytes());
    }
    layout[HASH_AT..].copy_from_slice(&iface.hash);
    if !state_lists.is_empty() {
        for v in [state_lists.len() as u16, gps_list] {
            layout.extend_from_slice(&v.to_le_bytes());
        }
        for l in &state_lists {
            for v in [l.list, l.leaf, l.max_items, l.item_leaves] {
                layout.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    let enums = (!w.enums.is_empty()).then(|| enums_bin(&w.enums));
    Ok(Tables { layout, view_map, event_map, enums })
}

/// The string leaves of a value of type `t` that aren't enums (any string
/// can be in them, so they'd point into memory the host doesn't keep),
/// those of a list's item included.
fn free_strings(t: &Ty) -> u16 {
    match t {
        Ty::Str(StrTy { values: None, .. }) => 1,
        Ty::Rec(f, _) => f.values().map(free_strings).sum(),
        Ty::Opt(inner) | Ty::List(inner, _) => free_strings(inner),
        _ => 0,
    }
}

fn enums_bin(strings: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(strings.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    let mut off = 4 + 8 * strings.len();
    for s in strings {
        out.extend_from_slice(&(off as u32).to_le_bytes());
        out.extend_from_slice(&(s.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        off += s.len();
    }
    for s in strings {
        out.extend_from_slice(s.as_bytes());
    }
    out
}

// ---- the walk --------------------------------------------------------------

struct Walker<'d> {
    names: Option<Names<'d>>,
    entries: Vec<[u16; 3]>,
    enums: Vec<String>,
    /// Where the walk is, for messages.
    path: Vec<String>,
}

impl<'d> Walker<'d> {
    /// A descriptor's map and its largest encoding in bytes.
    fn map(&mut self, d: &Descriptor<'d>, ty: &Ty, event: bool) -> Result<(Vec<u8>, u64), String> {
        self.names = Some(d.names());
        self.entries.clear();
        self.path.clear();
        let cost = self.node(d.root(), ty, 0, Ctx { event, root: true })?;
        if self.entries.len() != d.node_count() as usize {
            return Err(format!("the map has {} entries for {} nodes", self.entries.len(), d.node_count()));
        }
        let mut out = Vec::with_capacity(4 + MAP_ENTRY_BYTES * self.entries.len());
        out.extend_from_slice(&d.node_count().to_le_bytes());
        for e in &self.entries {
            for v in e {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        Ok((out, cost.max_from(START)))
    }

    fn name(&mut self) -> Result<&'d str, String> {
        self.names.as_mut().and_then(Iterator::next).ok_or_else(|| "the descriptor's names run out".to_string())
    }

    fn err(&self, msg: impl std::fmt::Display) -> String {
        if self.path.is_empty() { format!("the root: {msg}") } else { format!("`{}`: {msg}", self.path.join(".")) }
    }

    /// Maps node `n`, whose value has the type `ty` and starts at `leaf`;
    /// returns its cost (bytes, from each bit-byte state to each).
    fn node(&mut self, n: Node<'d>, ty: &Ty, leaf: u16, ctx: Ctx) -> Result<Cost, String> {
        let at = self.entries.len();
        self.entries.push([NONE, 1, 0]);
        let (inner, optional_record) = match ty {
            Ty::Opt(t) => (&**t, matches!(**t, Ty::Rec(..))),
            t => (t, false),
        };
        let kind = |ok: bool, what: &str| if ok { Ok(()) } else { Err(self.err(format!("the descriptor has {what}, the program {}", ty.describe()))) };
        let cost = match n.op() {
            Op::Bool => {
                kind(matches!(inner, Ty::Bool), "a boolean")?;
                self.entries[at][0] = leaf;
                Cost::bits(1)
            }
            op @ (Op::U8 | Op::I8 | Op::U16 | Op::I16 | Op::U32 | Op::I32 | Op::U64 | Op::I64) => {
                kind(matches!(inner, Ty::Int(_)), "an integer")?;
                self.entries[at][0] = leaf;
                Cost::bytes(match op {
                    Op::U8 | Op::I8 => 1,
                    Op::U16 | Op::I16 => 3,
                    Op::U32 | Op::I32 => 5,
                    _ => 10,
                })
            }
            op @ (Op::F32 | Op::F64) => {
                kind(matches!(inner, Ty::Float), "a number")?;
                self.entries[at][0] = leaf;
                Cost::bytes(if op == Op::F32 { 4 } else { 8 })
            }
            Op::String => {
                // a decoded string would point into the message, which the host doesn't keep
                if ctx.event {
                    return Err(self.err("strings in events aren't supported (enums are)"));
                }
                let Ty::Str(StrTy { max, values: None, .. }) = inner else { return Err(self.err(format!("the descriptor has a string, the program {}", ty.describe()))) };
                self.entries[at][0] = leaf;
                Cost::bytes(varint_len(*max as u64) + *max as u64)
            }
            Op::Enum => {
                let names: Vec<&str> = (0..n.len()).map(|_| self.name()).collect::<Result<_, _>>()?;
                let Ty::Str(StrTy { values: Some(values), .. }) = inner else { return Err(self.err(format!("the descriptor has an enum, the program {}", ty.describe()))) };
                if names.len() != values.len() || !names.iter().all(|v| values.contains(*v)) {
                    return Err(self.err("the descriptor's enum values aren't the program's"));
                }
                self.entries[at] = [leaf, 1, u16::try_from(self.enums.len()).map_err(|_| self.err("too many enum strings"))?];
                self.enums.extend(names.iter().map(|s| s.to_string()));
                Cost::bits(n.bits())
            }
            Op::Object | Op::Variants => {
                let Ty::Rec(fields, group) = inner else { return Err(self.err(format!("the descriptor has an object, the program {}", ty.describe()))) };
                if (n.op() == Op::Variants) != group.is_some() {
                    return Err(self.err("one of the descriptor and the program has variants, the other doesn't"));
                }
                if optional_record {
                    self.entries[at][0] = leaf;
                }
                let base = if optional_record { leaf + 1 } else { leaf };
                let plain: Vec<&str> = (0..n.len()).map(|_| self.name()).collect::<Result<_, _>>()?;
                let variants: Vec<&str> = (0..n.variant_count()).map(|_| self.name()).collect::<Result<_, _>>()?;
                let group = group.clone().unwrap_or_default();
                for f in &plain {
                    if !fields.contains_key(*f) || group.contains(*f) {
                        return Err(self.err(format!("the program has no field `{f}`")));
                    }
                }
                for v in &variants {
                    if !group.contains(*v) {
                        return Err(self.err(format!("the program has no variant `{v}`")));
                    }
                }
                // The event's descriptor holds only the author's variants;
                // the program's event also has the built-in ones.
                let all_variants = !(ctx.event && ctx.root);
                if plain.len() + group.len() != fields.len() || (all_variants && variants.len() != group.len()) {
                    return Err(self.err("the descriptor's fields aren't the program's"));
                }
                if ctx.event && ctx.root && variants.iter().any(|v| matches!(*v, "gps" | "wake")) {
                    return Err(self.err("an author's event can't be a built-in one (`gps`, `wake`)"));
                }
                let child = Ctx { root: false, ..ctx };
                let mut cost = Cost::identity();
                for (f, c) in plain.iter().zip(n.children()) {
                    let ft = &fields[*f];
                    if c.is_optional() != ft.may_be_nil() {
                        return Err(self.err(format!("`{f}` is optional in one of the descriptor and the program only")));
                    }
                    self.path.push(f.to_string());
                    let fc = self.node(c, ft, base + field_offset(fields, f), child)?;
                    self.path.pop();
                    cost = cost.then(&if c.is_optional() { Cost::bits(1).then(&Cost::identity().or(&fc)) } else { fc });
                }
                if n.op() == Op::Variants {
                    let mut any: Option<Cost> = None;
                    for (v, c) in variants.iter().zip(n.variants()) {
                        self.path.push(v.to_string());
                        let vc = self.node(c, &fields[*v], base + field_offset(fields, v), child)?;
                        self.path.pop();
                        any = Some(match any {
                            None => vc,
                            Some(a) => a.or(&vc),
                        });
                    }
                    cost = cost.then(&Cost::bits(n.bits())).then(&any.unwrap_or_else(Cost::identity));
                }
                cost
            }
            Op::Array => {
                if ctx.event {
                    return Err(self.err("lists in events aren't supported"));
                }
                let Ty::List(item, max) = inner else { return Err(self.err(format!("the descriptor has a list, the program {}", ty.describe()))) };
                self.entries[at][0] = leaf;
                self.path.push("[]".into());
                // An item's leaves are counted from its first.
                let ic = self.node(n.item(), item, 0, Ctx { root: false, ..ctx })?;
                self.path.pop();
                Cost::bytes(varint_len(*max as u64)).then(&ic.up_to(*max))
            }
            Op::Union => return Err(self.err("tagged unions aren't supported")),
            Op::Bytes => return Err(self.err("byte strings aren't supported")),
        };
        let span = self.entries.len() - at;
        self.entries[at][1] = u16::try_from(span).map_err(|_| self.err("the subtree is too large"))?;
        Ok(cost)
    }
}

#[derive(Clone, Copy)]
struct Ctx {
    /// Mapping the event's descriptor.
    event: bool,
    /// At the descriptor's root.
    root: bool,
}

fn varint_len(v: u64) -> u64 {
    (64 - v.leading_zeros() as u64).div_ceil(7).max(1)
}

// ---- the largest encoding -------------------------------------------------
//
// Bit fields share one stream-wide bit byte (wire/SPEC.md §3), so how many
// bytes a value takes depends on how full that byte is when it starts. A
// cost is, for each state of the bit byte before (bits used, 0 to 8; 8 = no
// room, as at the start), the most bytes the value can take ending in each
// state after: a max-plus matrix. Values in sequence multiply; alternatives
// (an optional field, the variants) take the larger.

const STATES: usize = 9;
const START: usize = 8;

#[derive(Clone)]
struct Cost([[Option<u64>; STATES]; STATES]);

impl Cost {
    fn identity() -> Cost {
        let mut m = [[None; STATES]; STATES];
        (0..STATES).for_each(|s| m[s][s] = Some(0));
        Cost(m)
    }

    fn bytes(n: u64) -> Cost {
        let mut m = [[None; STATES]; STATES];
        (0..STATES).for_each(|s| m[s][s] = Some(n));
        Cost(m)
    }

    /// A bit field of `w` bits: a new bit byte when it doesn't fit.
    fn bits(w: u32) -> Cost {
        if w == 0 {
            return Cost::identity();
        }
        let w = w as usize;
        let mut m = [[None; STATES]; STATES];
        for (s, row) in m.iter_mut().enumerate() {
            if s + w > 8 { row[w] = Some(1) } else { row[s + w] = Some(0) }
        }
        Cost(m)
    }

    /// `self`, then `o`.
    fn then(&self, o: &Cost) -> Cost {
        let mut m = [[None; STATES]; STATES];
        for (a, row) in m.iter_mut().enumerate() {
            for b in 0..STATES {
                for (out, y) in row.iter_mut().zip(o.0[b]) {
                    if let (Some(x), Some(y)) = (self.0[a][b], y) {
                        *out = Some(out.map_or(x + y, |r: u64| r.max(x + y)));
                    }
                }
            }
        }
        Cost(m)
    }

    /// `self` or `o`.
    fn or(&self, o: &Cost) -> Cost {
        let mut m = self.0;
        for (a, row) in m.iter_mut().enumerate() {
            for (b, v) in row.iter_mut().enumerate() {
                *v = match (*v, o.0[a][b]) {
                    (Some(x), Some(y)) => Some(x.max(y)),
                    (x, y) => x.or(y),
                };
            }
        }
        Cost(m)
    }

    /// 0 to `n` of `self` in sequence.
    fn up_to(&self, n: u32) -> Cost {
        let (mut all, mut k) = (Cost::identity(), Cost::identity());
        for _ in 0..n {
            k = k.then(self);
            all = all.or(&k);
        }
        all
    }

    fn max_from(&self, s: usize) -> u64 {
        self.0[s].iter().flatten().copied().max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_bytes_are_counted() {
        // 9 booleans: two bit bytes from the start
        let mut c = Cost::identity();
        for _ in 0..9 {
            c = c.then(&Cost::bits(1));
        }
        assert_eq!(c.max_from(START), 2);
        // a 3-bit field after 6 bits needs a new byte
        assert_eq!(Cost::bits(6).then(&Cost::bits(3)).max_from(START), 2);
        // optional u8: presence bit + byte
        assert_eq!(Cost::bits(1).then(&Cost::identity().or(&Cost::bytes(1))).max_from(START), 2);
        // up to 3 items of a bool + a byte: 3 bools share a bit byte
        assert_eq!(Cost::bits(1).then(&Cost::bytes(1)).up_to(3).max_from(START), 4);
    }

    /// Tune's interface (lang/examples/tune) with `edit` made to it, and its
    /// program compiled against that.
    fn tune(edit: impl FnOnce(&mut Json)) -> (Json, Vec<u8>) {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../lang/examples/tune");
        let mut iface: Json = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/interface.json")).unwrap()).unwrap();
        edit(&mut iface);
        let source = std::fs::read_to_string(format!("{dir}/program.expr")).unwrap();
        let program = compiler::compile(&source, &iface, &Default::default()).unwrap_or_else(|d| panic!("{d:#?}")).bytes;
        (iface, program)
    }

    /// A state list the output could make longer than the next evaluation
    /// takes is refused (the descriptors aren't looked at before it).
    #[test]
    fn a_state_list_must_fit_the_next_input() {
        let (iface, program) = tune(|i| i["output"]["properties"]["state"]["properties"]["samples"]["maxItems"] = 65.into());
        let e = tables(&iface, &program, &[], None).unwrap_err();
        assert!(e.contains("the output's `state.samples` holds up to 65 items, the input's up to 64"), "{e}");
    }

    #[test]
    fn varints() {
        assert_eq!(varint_len(0), 1);
        assert_eq!(varint_len(127), 1);
        assert_eq!(varint_len(128), 2);
        assert_eq!(varint_len(57_344), 3);
    }
}
