//! The install check (the plan's A4, A8): a bundle is validated once, when
//! it is installed, and its tables are never trusted before that. Every
//! leaf the host sets or reads must be in range and of the kind the
//! program declares; every map must follow its descriptor node for node;
//! and the engine must fit the host's buffers.

use crate::archive::Archive;
use crate::layout::{self, Layout, NONE, StateLists};
use crate::map::{Enums, Map};
use bitwire::{Descriptor, Node, Op};
use bytecode::{Check, Kind, Program};
use extreme_protocol::{Manifest, NO_WATCH_BUNDLE};

/// The `abi` of a bundle whose engine is lang bytecode.
pub const ABI: u16 = 2;

/// Why a bundle is refused: [`Invalid::Reason`] with one of [`reason`]'s
/// codes and a detail (which file, field or map entry), or
/// [`Invalid::OverLimit`] with one of [`limit`]'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    Reason { reason: u8, detail: u32 },
    OverLimit { which: u8 },
}

/// [`Invalid::Reason`] codes.
pub mod reason {
    /// The archive doesn't parse; detail: [`crate::archive::Error`] as u32.
    pub const ARCHIVE: u8 = 1;
    /// `manifest.bin` doesn't decode, or its `abi` isn't [`crate::validate::ABI`] (detail 1),
    /// or (checked at an upload, [`crate::hash::check`]) its `content_hash`, `ui.web` or
    /// `ui.watch` isn't the files' (detail 2, 3, 4).
    pub const MANIFEST: u8 = 2;
    /// A required file is missing (or one of a pair); detail: a [`super::file`] code.
    pub const MISSING: u8 = 3;
    /// `engine.bin` doesn't load (parse and validate); detail: the load error.
    pub const PROGRAM: u8 = 4;
    /// `layout.bin` doesn't parse; detail: [`crate::layout::Error`] as u32.
    pub const LAYOUT: u8 = 5;
    /// The program's and the layout's interface hashes differ.
    pub const HASH: u8 = 6;
    /// The program's leaf or list counts aren't the layout's, or its lists
    /// aren't `gps` and the list table's.
    pub const COUNTS: u8 = 7;
    /// A layout leaf is out of range or of the wrong kind; detail: the
    /// field's index in `layout.bin`.
    pub const LEAF: u8 = 8;
    /// A descriptor doesn't load; detail: 0 the view's, 1 the event's.
    pub const DESCRIPTOR: u8 = 9;
    /// A map doesn't follow its descriptor (size, spans, a leaf out of
    /// range); detail: the entry, plus [`super::EVENT`] for the event's map.
    pub const MAP: u8 = 10;
    /// A node's leaf has the wrong kind (bool, int, float, string, enum,
    /// presence, list, nullable); detail as for [`MAP`].
    pub const KIND: u8 = 11;
    /// `enums.bin` doesn't parse, or an enum's strings aren't the program's;
    /// detail as for [`MAP`] (0 when the table itself is malformed).
    pub const ENUMS: u8 = 12;
    /// The state has string leaves that aren't enums.
    pub const STATE_STRINGS: u8 = 13;
    /// `watch.js` and the manifest's `ui.watch`/`watch_len` disagree.
    pub const WATCH: u8 = 14;
    /// A node the host doesn't support: a tagged union or a byte string, or
    /// in an event a list or a free string; detail as for [`MAP`].
    pub const UNSUPPORTED: u8 = 15;
    /// A state list in the list table isn't the program's: its id is
    /// `gps`'s or another entry's, its leaf isn't a list handle in the
    /// state, or its bound or item leaves aren't the program's; detail: the
    /// entry.
    pub const STATE_LIST: u8 = 16;
}

/// [`Invalid::OverLimit`] codes: which of [`Limits`] is exceeded.
pub mod limit {
    pub const CODE_BYTES: u8 = 1;
    pub const STACK: u8 = 2;
    pub const LOCALS: u8 = 3;
    pub const INPUTS: u8 = 4;
    pub const OUTPUTS: u8 = 5;
    pub const STATE_LEAVES: u8 = 6;
    pub const IMPORTS: u8 = 7;
    pub const PUB_BYTES: u8 = 8;
    /// The state lists' items at their most, slots.
    pub const STATE_LIST_SLOTS: u8 = 9;
    /// The state lists.
    pub const STATE_LISTS: u8 = 10;
}

/// [`reason::MISSING`] details.
pub mod file {
    pub const MANIFEST: u32 = 1;
    pub const ENGINE: u32 = 2;
    pub const LAYOUT: u32 = 3;
    pub const VIEW_DESC: u32 = 4;
    pub const VIEW_MAP: u32 = 5;
    /// `event.desc` without `event.map` or the other way round.
    pub const EVENT: u32 = 6;
    pub const INDEX_HTML: u32 = 7;
    /// `enums.bin`, which a map's enum needs.
    pub const ENUMS: u32 = 8;
}

/// Added to a map entry in a detail when the map is the event's.
pub const EVENT: u32 = 0x8000_0000;

/// What the host has room for (its buffers, sized once).
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Code bytes (the load scratch holds one u16 per byte).
    pub code_bytes: u32,
    pub max_stack: u16,
    pub max_locals: u16,
    /// Input and output leaves.
    pub inputs: u16,
    pub outputs: u16,
    pub state_leaves: u16,
    /// Host builtins the program may import.
    pub imports: u16,
    /// The largest encoded view (a host's `STATE_MAX`).
    pub pub_bytes: u16,
    /// Lists in the state, and the slots their items take at their most
    /// (each list `max_items × item_leaves`): where the host keeps them
    /// between evaluations.
    pub state_lists: u16,
    pub state_list_slots: u32,
}

/// A descriptor and its map.
#[derive(Clone, Copy, Debug)]
pub struct Codec<'a> {
    pub desc: Descriptor<'a>,
    pub map: Map<'a>,
}

/// A bundle's engine, validated.
#[derive(Clone, Copy, Debug)]
pub struct Engine<'a> {
    /// Loaded (parsed and validated by `vm::load`).
    pub program: Program<'a>,
    pub layout: Layout,
    /// `layout.bin`'s list table: `gps`'s id and the lists in the state.
    pub state_lists: StateLists<'a>,
    pub view: Codec<'a>,
    pub event: Option<Codec<'a>>,
    pub enums: Option<Enums<'a>>,
}

/// A validated bundle.
#[derive(Clone, Debug)]
pub struct Validated<'a> {
    pub manifest: Manifest,
    /// None for a bundle without an engine (a page only).
    pub engine: Option<Engine<'a>>,
}

fn why(reason: u8, detail: u32) -> Invalid {
    Invalid::Reason { reason, detail }
}

/// Validates `archive` for a host with `limits`. `scratch` is lent to
/// `vm::load` (at least `limits.code_bytes` entries).
pub fn validate<'a>(archive: &Archive<'a>, limits: &Limits, scratch: &mut [u16]) -> Result<Validated<'a>, Invalid> {
    let manifest_bin = archive.file("manifest.bin").ok_or(why(reason::MISSING, file::MANIFEST))?;
    let manifest = match postcard::take_from_bytes::<Manifest>(manifest_bin) {
        Ok((m, [])) => m,
        _ => return Err(why(reason::MANIFEST, 0)),
    };
    archive.file("web/index.html").ok_or(why(reason::MISSING, file::INDEX_HTML))?;
    match (archive.file("watch.js"), manifest.ui.watch != NO_WATCH_BUNDLE) {
        (None, false) => {}
        (Some(w), true) if w.len() as u64 == manifest.watch_len as u64 => {}
        _ => return Err(why(reason::WATCH, 0)),
    }
    let Some(engine_bin) = archive.file("engine.bin") else {
        // tables without the engine they're for
        if ["layout.bin", "view.desc", "view.map", "event.desc", "event.map"].iter().any(|n| archive.file(n).is_some()) {
            return Err(why(reason::MISSING, file::ENGINE));
        }
        return Ok(Validated { manifest, engine: None });
    };
    if manifest.abi != ABI {
        return Err(why(reason::MANIFEST, 1));
    }
    let engine = engine(archive, engine_bin, limits, scratch)?;
    Ok(Validated { manifest, engine: Some(engine) })
}

fn engine<'a>(archive: &Archive<'a>, engine_bin: &'a [u8], limits: &Limits, scratch: &mut [u16]) -> Result<Engine<'a>, Invalid> {
    let need = |name: &str, f: u32| archive.file(name).ok_or(why(reason::MISSING, f));
    let layout_bin = need("layout.bin", file::LAYOUT)?;
    let layout = Layout::parse(layout_bin).map_err(|e| why(reason::LAYOUT, e as u32))?;
    let state_lists = StateLists::parse(layout_bin).map_err(|e| why(reason::LAYOUT, e as u32))?;
    let load_err = |e: bytecode::LoadError| why(reason::PROGRAM, load_code(e));
    let p = Program::parse(engine_bin).map_err(load_err)?;
    let over = |which| Err(Invalid::OverLimit { which });
    if p.code.len() as u64 > limits.code_bytes as u64 || p.code.len() > scratch.len() {
        return over(limit::CODE_BYTES);
    }
    if p.max_stack > limits.max_stack {
        return over(limit::STACK);
    }
    if p.locals > limits.max_locals {
        return over(limit::LOCALS);
    }
    if p.input_count > limits.inputs {
        return over(limit::INPUTS);
    }
    if p.output_count > limits.outputs {
        return over(limit::OUTPUTS);
    }
    if p.import_count > limits.imports {
        return over(limit::IMPORTS);
    }
    if layout.state_leaves > limits.state_leaves {
        return over(limit::STATE_LEAVES);
    }
    if layout.pub_max_bytes > limits.pub_bytes {
        return over(limit::PUB_BYTES);
    }
    if state_lists.len() > limits.state_lists as usize {
        return over(limit::STATE_LISTS);
    }
    if state_lists.slots() > limits.state_list_slots {
        return over(limit::STATE_LIST_SLOTS);
    }
    let program = vm::load(engine_bin, scratch, &vm::Limits { max_stack: limits.max_stack, max_locals: limits.max_locals }).map_err(load_err)?;
    if program.interface_hash != layout.interface_hash {
        return Err(why(reason::HASH, 0));
    }
    if program.input_count != layout.input_count
        || program.output_count != layout.output_count
        || program.list_count != layout.list_count
        || layout.list_count as usize != 1 + state_lists.len()
        || state_lists.gps_list >= layout.list_count
    {
        return Err(why(reason::COUNTS, 0));
    }
    if layout.state_free_strings != 0 {
        return Err(why(reason::STATE_STRINGS, layout.state_free_strings as u32));
    }
    leaves(&program, &layout, state_lists.gps_list)?;
    lists(&program, &layout, &state_lists)?;

    let enums = archive.file("enums.bin").map(Enums::parse).transpose().map_err(|_| why(reason::ENUMS, 0))?;
    let codec = |desc: &'a [u8], map: &'a [u8], d: u32| -> Result<Codec<'a>, Invalid> {
        let desc = Descriptor::load(desc).map_err(|_| why(reason::DESCRIPTOR, d))?;
        let map = Map::parse(map).map_err(|_| why(reason::MAP, if d == 0 { 0 } else { EVENT }))?;
        Ok(Codec { desc, map })
    };
    let view = codec(need("view.desc", file::VIEW_DESC)?, need("view.map", file::VIEW_MAP)?, 0)?;
    Walk { p: &program, map: view.map, enums: enums.as_ref(), event: false, base: layout.pub_base, leaves: layout.pub_leaves }.all(&view.desc)?;
    let event = match (archive.file("event.desc"), archive.file("event.map")) {
        (None, None) => None,
        (Some(d), Some(m)) => {
            let c = codec(d, m, 1)?;
            Walk { p: &program, map: c.map, enums: enums.as_ref(), event: true, base: layout.in_event, leaves: layout.event_leaves }.all(&c.desc)?;
            Some(c)
        }
        _ => return Err(why(reason::MISSING, file::EVENT)),
    };
    Ok(Engine { program, layout, state_lists, view, event, enums })
}

/// The list table: each state list a list of the program's other than
/// `gps` (each once: with `list_count` 1 + entries, every list is named),
/// its handle a list leaf within the state (the output's, at the same
/// place in the output's state, is one too: `leaves` checked the two
/// states alike), its bound and item leaves the program's.
fn lists(p: &Program, l: &Layout, t: &StateLists) -> Result<(), Invalid> {
    for (k, s) in t.iter().enumerate() {
        let bad = Err(why(reason::STATE_LIST, k as u32));
        let named_before = t.iter().take(k).any(|o| o.list == s.list || o.leaf == s.leaf);
        if s.list >= p.list_count || s.list == t.gps_list || named_before {
            return bad;
        }
        let in_state = s.leaf > l.in_state && (s.leaf as u32) < l.in_state as u32 + 1 + l.state_leaves as u32;
        if !in_state || p.input(s.leaf).kind != Kind::List {
            return bad;
        }
        if p.list(s.list) != (s.max_items, s.item_leaves) {
            return bad;
        }
    }
    Ok(())
}

/// A load error as a detail: its variant (1…), and an offset or index in
/// the high bits where it has one.
fn load_code(e: bytecode::LoadError) -> u32 {
    use bytecode::LoadError::*;
    match e {
        BadMagic => 1,
        BadVersion(_) => 2,
        Truncated => 3,
        TrailingBytes => 4,
        BadKind => 5,
        BadOpcode(at) => 6 | (at as u32) << 8,
        BadOperand(at) => 7 | (at as u32) << 8,
        BadImport(i) => 8 | (i as u32) << 8,
        Unlinked(i) => 9 | (i as u32) << 8,
        BadJump(at) => 10 | (at as u32) << 8,
        BadStack(at) => 11 | (at as u32) << 8,
        BadFlow(at) => 12 | (at as u32) << 8,
        Scratch => 13,
        OverLimit => 14,
    }
}

fn is(c: &Check, kind: Kind, nullable: bool) -> bool {
    c.kind == kind && c.nullable == nullable
}

/// Every leaf the layout names: in range, and of the kind the host gives
/// it (inputs) or takes from it (outputs).
fn leaves(p: &Program, l: &Layout, gps_list: u16) -> Result<(), Invalid> {
    // the field's index in layout.bin, for the detail
    let field = |name: &str| layout::FIELD_NAMES.iter().position(|n| *n == name).unwrap_or(0) as u32;
    let bad = |name: &str| Err(why(reason::LEAF, field(name)));
    let input = |leaf: u16| (leaf < p.input_count).then(|| p.input(leaf));
    let output = |leaf: u16| (leaf < p.output_count).then(|| p.output(leaf));
    let fits = |base: u16, n: u16, count: u16| base as u32 + n as u32 <= count as u32;

    // The state: its presence leaf, its leaves in the input and the output,
    // laid out alike (the host copies the output's into the input).
    if !input(l.in_state).is_some_and(|c| is(&c, Kind::Bool, true)) || !fits(l.in_state + 1, l.state_leaves, p.input_count) {
        return bad("in_state");
    }
    if !fits(l.out_state, l.state_leaves, p.output_count) {
        return bad("out_state");
    }
    for k in 0..l.state_leaves {
        if p.input(l.in_state + 1 + k).kind != p.output(l.out_state + k).kind {
            return bad("state_leaves");
        }
    }
    // The event: the built-in events' leaves, within it.
    if !fits(l.in_event, l.event_leaves, p.input_count) {
        return bad("in_event");
    }
    let in_event = |leaf: u16| leaf >= l.in_event && leaf < l.in_event + l.event_leaves;
    for (name, leaf, kind, nullable) in [
        ("ev_gps", l.ev_gps, Kind::Bool, true),
        ("ev_gps_fix", l.ev_gps_fix, Kind::Bool, true),
        ("ev_gps_fix_t", l.ev_gps_fix_t, Kind::Int, true),
        ("ev_gps_fix_lat", l.ev_gps_fix_lat, Kind::Float, true),
        ("ev_gps_fix_lon", l.ev_gps_fix_lon, Kind::Float, true),
        ("ev_gps_fix_alt", l.ev_gps_fix_alt, Kind::Float, true),
        ("ev_gps_velocity", l.ev_gps_velocity, Kind::Bool, true),
        ("ev_gps_velocity_speed", l.ev_gps_velocity_speed, Kind::Float, true),
        ("ev_gps_velocity_heading", l.ev_gps_velocity_heading, Kind::Float, true),
        ("ev_wake", l.ev_wake, Kind::Bool, true),
    ] {
        if !in_event(leaf) || !input(leaf).is_some_and(|c| is(&c, kind, nullable)) {
            return bad(name);
        }
    }
    if !input(l.in_timestamp).is_some_and(|c| is(&c, Kind::Int, false)) {
        return bad("in_timestamp");
    }
    if !input(l.in_gps).is_some_and(|c| c.kind == Kind::List) {
        return bad("in_gps");
    }
    // A fix in `gps`, as the host's list serves it.
    let (max, item_leaves) = p.list(gps_list);
    if max != l.gps_max_items {
        return bad("gps_max_items");
    }
    if item_leaves != l.fix_leaves {
        return bad("fix_leaves");
    }
    for (name, leaf, kind, nullable) in
        [("fix_t", l.fix_t, Kind::Int, false), ("fix_lat", l.fix_lat, Kind::Float, false), ("fix_lon", l.fix_lon, Kind::Float, false), ("fix_alt", l.fix_alt, Kind::Float, true)]
    {
        if leaf >= item_leaves || !is(&p.list_item_check(gps_list, leaf), kind, nullable) {
            return bad(name);
        }
    }
    // The output: the wake, the view, the published value.
    if !output(l.out_wake).is_some_and(|c| is(&c, Kind::Int, true)) {
        return bad("out_wake");
    }
    if l.out_view != NONE && !fits(l.out_view, l.view_leaves, p.output_count) {
        return bad("out_view");
    }
    let published = if l.out_view == NONE { (l.out_state, l.state_leaves) } else { (l.out_view, l.view_leaves) };
    if (l.pub_base, l.pub_leaves) != published || !fits(l.pub_base, l.pub_leaves, p.output_count) {
        return bad("pub_base");
    }
    Ok(())
}

/// A map checked against its descriptor and the program.
struct Walk<'p, 'a> {
    p: &'p Program<'a>,
    map: Map<'a>,
    enums: Option<&'p Enums<'a>>,
    event: bool,
    /// The value's first leaf among the inputs (event) or outputs (view),
    /// and its leaves.
    base: u16,
    leaves: u16,
}

impl Walk<'_, '_> {
    fn all(&self, d: &Descriptor) -> Result<(), Invalid> {
        let mut at = 0;
        self.node(d.root(), &mut at, false, false, false)?;
        if at as usize != self.map.len() || self.map.len() != d.node_count() as usize {
            return Err(self.why(reason::MAP, at));
        }
        Ok(())
    }

    fn why(&self, reason: u8, entry: u32) -> Invalid {
        why(reason, entry | if self.event { EVENT } else { 0 })
    }

    /// Node `n` at entry `*at` (and its subtree): `optional` an optional
    /// field, `variant` a record's variant, `item` inside an array's item.
    fn node(&self, n: Node, at: &mut u32, optional: bool, variant: bool, item: bool) -> Result<(), Invalid> {
        let i = *at;
        let e = self.map.get(i as usize).ok_or(self.why(reason::MAP, i))?;
        *at += 1;
        let kind_err = Err(self.why(reason::KIND, i));
        let unsupported = Err(self.why(reason::UNSUPPORTED, i));
        // The leaf's check; None inside an array's item, which the program
        // describes only for input lists (a list in the output is a handle).
        let check = if e.leaf == NONE || item {
            None
        } else {
            if e.leaf >= self.leaves {
                return Err(self.why(reason::MAP, i));
            }
            let leaf = self.base + e.leaf;
            Some(if self.event { self.p.input(leaf) } else { self.p.output(leaf) })
        };
        let scalar = |kind: Kind| -> Result<Option<Check>, Invalid> {
            if e.leaf == NONE {
                return Err(self.why(reason::MAP, i));
            }
            match check {
                Some(c) if c.kind != kind || (optional && !c.nullable) => Err(self.why(reason::KIND, i)),
                c => Ok(c),
            }
        };
        match n.op() {
            Op::Bool => {
                scalar(Kind::Bool)?;
            }
            op @ (Op::U8 | Op::I8 | Op::U16 | Op::I16 | Op::U32 | Op::I32 | Op::U64 | Op::I64) => {
                // Every value the program can publish must fit the op.
                if let Some(c) = scalar(Kind::Int)?
                    && !self.event
                {
                    let (lo, hi) = match op {
                        Op::U8 | Op::U16 | Op::U32 | Op::U64 => (0, op.uint_max().min(i64::MAX as u64) as i64),
                        _ => op.int_range(),
                    };
                    if c.lo < lo || c.hi > hi {
                        return kind_err;
                    }
                }
            }
            Op::F32 | Op::F64 => {
                scalar(Kind::Float)?;
            }
            Op::String => {
                if self.event {
                    return unsupported;
                }
                if scalar(Kind::Str)?.is_some_and(|c| c.enum_count != 0) {
                    return kind_err;
                }
            }
            Op::Enum => {
                let c = scalar(Kind::Str)?;
                let enums = self.enums.ok_or(why(reason::MISSING, file::ENUMS))?;
                let strings = (0..n.len()).map(|k| enums.get(e.aux as usize + k));
                if strings.clone().any(|s| s.is_none()) {
                    return Err(self.why(reason::ENUMS, i));
                }
                if let Some(c) = c
                    && (c.enum_count as usize != n.len() || !strings.flatten().all(|s| self.p.in_enum(&c, s)))
                {
                    return Err(self.why(reason::ENUMS, i));
                }
            }
            Op::Object | Op::Variants => {
                // An optional record and a variant have a presence leaf: a
                // nullable bool.
                if (optional || variant) && e.leaf == NONE {
                    return kind_err;
                }
                if e.leaf != NONE && !item && !check.is_some_and(|c| is(&c, Kind::Bool, true)) {
                    return kind_err;
                }
                for c in n.children() {
                    self.node(c, at, c.is_optional(), false, item)?;
                }
                for v in n.variants() {
                    self.node(v, at, false, true, item)?;
                }
            }
            Op::Array => {
                if self.event {
                    return unsupported;
                }
                if e.leaf == NONE || item || !check.is_some_and(|c| c.kind == Kind::List) {
                    return kind_err;
                }
                self.node(n.item(), at, false, false, true)?;
            }
            Op::Union | Op::Bytes => return unsupported,
        }
        if e.span as u32 != *at - i {
            return Err(self.why(reason::MAP, i));
        }
        Ok(())
    }
}
