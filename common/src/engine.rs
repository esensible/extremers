//! A package's engine on `lang/vm` (engine/PROFILE.md §3): its program,
//! validated when the bundle was loaded, evaluated once per event as
//! `(state, event, timestamp, gps) → {state, view?, wake?}`.
//!
//! The host's inputs and outputs are among the program's leaves where the
//! bundle's `layout.bin` says; an author's event is decoded straight into
//! its input leaves (`bundle::EventSink`) and the published value (the
//! view, or the state without one) encoded straight from the output leaves
//! (`bundle::ViewSource`), both with bitwire and the bundle's descriptors.
//!
//! Every buffer is the host's, sized once ([`EngineMemory`], [`LIMITS`]):
//! nothing is allocated. The program runs in place from the bundle; the
//! bundle is `'static` (flash on the C6), so the values the buffers hold
//! are too, and every string in them points into the bundle (the program's
//! constants, `enums.bin`). The state may hold no other strings: the
//! install check refuses free strings in the state, and each evaluation
//! checks its state's strings again ([`Rejected::Output`]).
//!
//! A list in the state (the layout's list table) is a handle in the
//! output: its items are in the evaluation's locals (a list the program
//! built) or an input list (one it passed through), and the next
//! evaluation clears the locals. So after each accepted evaluation its
//! items are copied into a region of their own
//! ([`EngineMemory::state_lists`], each list's at its most), and the next
//! evaluation reads them from there as that input list ([`SavedList`]).
//!
//! An evaluation that fails (`R-…`) changes nothing: the state stays, the
//! event is dropped, and the engine keeps running.

use bundle::{
    EventSink, Layout, Limits, StateLists, ViewSource,
    map::Enums,
    validate::Codec,
};
use extreme_protocol::STATE_MAX;
use vm::{Fail, HostFn, ListInput, ListRef, LoadError, Program, Stats, Value};

use crate::{
    fmt::Dbg,
    package::Package,
    track::{FixLeaves, GpsList, Track},
};

/// Value stack slots.
pub const STACK_SLOTS: usize = 128;
/// Local slots.
pub const LOCAL_SLOTS: usize = 256;
/// Output leaves.
pub const OUTPUT_SLOTS: usize = 128;
/// Input leaves.
pub const INPUT_SLOTS: usize = 192;
/// The state's leaves.
pub const STATE_SLOTS: usize = 128;
/// Code bytes (the load scratch holds a u16 for each).
pub const CODE_BYTES: usize = 8192;
/// Builtins a program may import.
pub const IMPORTS: usize = 8;
/// Lists in the state.
pub const STATE_LISTS: usize = 4;
/// The state lists' items at their most (each `maxItems × item leaves`):
/// 16 KiB.
pub const STATE_LIST_SLOTS: usize = 1024;

/// What a bundle's engine must fit: [`EngineMemory`]'s buffers. Checked
/// when it is loaded ([`Package::load`]).
pub const LIMITS: Limits = Limits {
    code_bytes: CODE_BYTES as u32,
    max_stack: STACK_SLOTS as u16,
    max_locals: LOCAL_SLOTS as u16,
    inputs: INPUT_SLOTS as u16,
    outputs: OUTPUT_SLOTS as u16,
    state_leaves: STATE_SLOTS as u16,
    imports: IMPORTS as u16,
    pub_bytes: STATE_MAX as u16,
    state_lists: STATE_LISTS as u16,
    state_list_slots: STATE_LIST_SLOTS as u32,
};

/// Every buffer an engine needs, for one engine at a time: the host owns
/// one, `'static` (on the C6 a static, not the heap or the main stack), and
/// lends it to the active engine's every call.
pub struct EngineMemory {
    stack: [Value<'static>; STACK_SLOTS],
    locals: [Value<'static>; LOCAL_SLOTS],
    output: [Value<'static>; OUTPUT_SLOTS],
    inputs: [Value<'static>; INPUT_SLOTS],
    /// The last evaluation's state. A state list's leaf holds its input
    /// handle (`ListRef::Input`), or nil when the list is absent.
    state: [Value<'static>; STATE_SLOTS],
    /// The last evaluation's state lists: each list's items in its region,
    /// one after another (the regions in the list table's order, each
    /// `max_items × item_leaves` slots: `StateLists::offset`).
    state_lists: [Value<'static>; STATE_LIST_SLOTS],
    /// Each state list's items; `None` when it is absent (in an optional
    /// record that is nil).
    state_list_lens: [Option<u16>; STATE_LISTS],
    /// `vm::load`'s while a bundle is loaded ([`Package::load`]).
    pub scratch: [u16; CODE_BYTES],
    /// The published value, bitwire: the last evaluation's, and the one
    /// being encoded.
    view: [u8; STATE_MAX],
    scratch_view: [u8; STATE_MAX],
}

impl EngineMemory {
    pub const fn new() -> Self {
        EngineMemory {
            stack: [Value::Nil; STACK_SLOTS],
            locals: [Value::Nil; LOCAL_SLOTS],
            output: [Value::Nil; OUTPUT_SLOTS],
            inputs: [Value::Nil; INPUT_SLOTS],
            state: [Value::Nil; STATE_SLOTS],
            state_lists: [Value::Nil; STATE_LIST_SLOTS],
            state_list_lens: [None; STATE_LISTS],
            scratch: [0; CODE_BYTES],
            view: [0; STATE_MAX],
            scratch_view: [0; STATE_MAX],
        }
    }
}

impl Default for EngineMemory {
    fn default() -> Self {
        Self::new()
    }
}

/// A state list as the last evaluation left it, in its region: the next
/// evaluation's input list.
#[derive(Clone, Copy, Debug)]
pub struct SavedList<'m> {
    /// The items' leaves, one item after another.
    pub items: &'m [Value<'static>],
    /// Leaves per item.
    pub stride: u16,
}

impl SavedList<'_> {
    /// Item `i`'s leaves.
    pub fn item(&self, i: usize) -> &[Value<'static>] {
        &self.items[i * self.stride as usize..][..self.stride as usize]
    }
}

impl ListInput<'static> for SavedList<'_> {
    fn len(&self) -> u32 {
        (self.items.len() / self.stride.max(1) as usize) as u32
    }

    fn read(&self, item: u32, leaf: u16) -> Value<'static> {
        self.items.get(item as usize * self.stride as usize + leaf as usize).copied().unwrap_or(Value::Nil)
    }
}

/// Where a state list's next items come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// Nil: the list is absent.
    Absent,
    /// Built by the program: `len` items from local `base`.
    Local { base: u16, len: u16 },
    /// `gps`, passed through: its first `len` fixes.
    Gps { len: u16 },
    /// A state list (its place in the table), passed through: its first
    /// `len` items, as the last evaluation left them.
    Saved { from: usize, len: u16 },
}

impl Source {
    fn len(self) -> Option<u16> {
        match self {
            Source::Absent => None,
            Source::Local { len, .. } | Source::Gps { len } | Source::Saved { len, .. } => Some(len),
        }
    }
}

/// What an engine is told.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event<'e> {
    /// A GPS update (`{gps: {fix?, velocity?}}`): the fix `(t, lat, lon)`,
    /// already the track's last; the velocity `(speed, heading)`.
    Gps {
        fix: Option<(i64, f64, f64)>,
        velocity: Option<(f64, f64)>,
    },
    /// The wake it asked for is due (`{wake: {}}`).
    Wake,
    /// A client's event, bitwire, one of the engine's own (`event.desc`).
    Client(&'e [u8]),
}

/// An evaluation that happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Evaluated {
    /// The published value's bytes changed.
    pub changed: bool,
    /// When it wants [`Event::Wake`], device clock; `None`: no wake.
    pub wake: Option<i64>,
}

/// An evaluation that did not happen; nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// A client's bytes are not one of the engine's events (or it takes
    /// none).
    Undecodable,
    /// The program failed (`R-…`; an input outside its declaration is
    /// `R-INPUT`).
    Failed(Fail),
    /// The output can't be published: the view doesn't encode, or the
    /// state holds a string outside the bundle. A broken bundle.
    Output,
}

/// Why an engine did not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartError {
    /// The package has no engine.
    NoEngine,
    /// The program imports a builtin the host doesn't have.
    Link(LoadError),
    /// Its first evaluation (`{gps: {}}`) was rejected.
    First(Rejected),
}

/// A running engine. Its buffers are the host's [`EngineMemory`], lent to
/// each call.
pub struct VmEngine {
    name: &'static str,
    /// The whole bundle: where the state's strings must point.
    bundle: &'static [u8],
    program: Program<'static>,
    linked: [Option<&'static HostFn>; IMPORTS],
    layout: Layout,
    state_lists: StateLists<'static>,
    view_codec: Codec<'static>,
    event_codec: Option<Codec<'static>>,
    enums: Option<Enums<'static>>,
    /// Whether `EngineMemory::state` holds a state (none before the first
    /// evaluation).
    has_state: bool,
    view_len: usize,
    wake: Option<i64>,
    stats: Stats,
    /// Microseconds the last evaluation spent in the VM proper (`vm::run`);
    /// the rest of its time is input and output handling.
    vm_us: u32,
    /// Evaluations so far, accepted or not (the first included).
    evaluations: u32,
    /// The last evaluation's outcome.
    last: Option<Result<Evaluated, Rejected>>,
}

impl VmEngine {
    /// Links `package`'s program (validated when it was loaded) and runs
    /// its first evaluation, `{gps: {}}` at `now`, so a view exists before
    /// the first event.
    pub fn start(package: &Package, now: i64, memory: &mut EngineMemory, track: &dyn Track) -> Result<VmEngine, StartError> {
        let mut e = Self::link(package)?;
        e.evaluate(memory, track, now, Event::Gps { fix: None, velocity: None })
            .map_err(StartError::First)?;
        Ok(e)
    }

    /// [`start`](Self::start) for `package` replacing the package
    /// `previous` runs, keeping `previous`'s state when both programs have
    /// the same interface hash (a new version whose UIs or code changed but
    /// not its state): the first evaluation is then given that state. Its
    /// strings (enum values, which point into the old bundle) are pointed
    /// at the same strings in the new one. If the state can't be carried,
    /// or the first evaluation with it is rejected (logged), the engine
    /// starts afresh.
    pub fn resume(previous: VmEngine, package: &Package, now: i64, memory: &mut EngineMemory, track: &dyn Track) -> Result<VmEngine, StartError> {
        let mut e = Self::link(package)?;
        let n = e.layout.state_leaves as usize;
        let same = previous.has_state && previous.program.interface_hash == e.program.interface_hash;
        // (the same interface: the same list table, so the regions are where
        // the new engine reads them)
        let lists = e.state_lists;
        let in_lists = |memory: &mut EngineMemory| {
            lists.iter().enumerate().all(|(k, s)| {
                let used = memory.state_list_lens[k].unwrap_or(0) as usize * s.item_leaves as usize;
                repoint(&mut memory.state_lists[lists.offset(k) as usize..][..used], &e)
            })
        };
        if same && repoint(&mut memory.state[..n], &e) && in_lists(memory) {
            e.has_state = true;
            match e.evaluate(memory, track, now, Event::Gps { fix: None, velocity: None }) {
                Ok(_) => {
                    info!("engine {}: state kept from the version it replaces", e.name);
                    return Ok(e);
                }
                Err(r) => warn!("engine {}: the state kept is rejected ({:?}); starting afresh", e.name, Dbg(&r)),
            }
        } else if previous.has_state {
            info!("engine {}: interface changed; starting afresh", e.name);
        }
        Self::start(package, now, memory, track)
    }

    /// `package`'s engine linked, before its first evaluation.
    fn link(package: &Package) -> Result<VmEngine, StartError> {
        let engine = package.engine.ok_or(StartError::NoEngine)?;
        let mut linked = [None; IMPORTS];
        vm::link(&engine.program, &sailing::BUILTINS, &mut linked).map_err(StartError::Link)?;
        Ok(VmEngine {
            name: package.name,
            bundle: package.archive.bytes(),
            program: engine.program,
            linked,
            layout: engine.layout,
            state_lists: engine.state_lists,
            view_codec: engine.view,
            event_codec: engine.event,
            enums: engine.enums,
            has_state: false,
            view_len: 0,
            wake: None,
            stats: Stats::default(),
            vm_us: 0,
            evaluations: 0,
            last: None,
        })
    }

    /// The published value, bitwire (the view, or the state without one),
    /// as of the last evaluation.
    pub fn view<'m>(&self, memory: &'m EngineMemory) -> &'m [u8] {
        &memory.view[..self.view_len]
    }

    /// When it wants [`Event::Wake`], as of the last evaluation.
    pub fn wake(&self) -> Option<i64> {
        self.wake
    }

    /// What the last evaluation used: instructions (`steps`) and the
    /// deepest the value stack went.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Microseconds the last evaluation spent in the VM proper.
    pub fn vm_micros(&self) -> u32 {
        self.vm_us
    }

    /// The state the last accepted evaluation left, as the program's leaves
    /// (`layout.state_leaves` of them, the state schema's in leaf order);
    /// `None` before the first.
    pub fn state<'m>(&self, memory: &'m EngineMemory) -> Option<&'m [Value<'static>]> {
        self.has_state.then(|| &memory.state[..self.layout.state_leaves as usize])
    }

    /// The state list `list` (a program's list id: what a list leaf of
    /// [`state`](Self::state) holds, `ListRef::Input(list)`), as the last
    /// accepted evaluation left it; `None` before the first, when it is
    /// absent, or for an id that isn't a state list's.
    pub fn state_list<'m>(&self, memory: &'m EngineMemory, list: u16) -> Option<SavedList<'m>> {
        let (k, s) = self.state_lists.of_list(list)?;
        saved(&self.state_lists, memory.state_list_lens[k]?, k, s.item_leaves, &memory.state_lists)
            .filter(|_| self.has_state)
    }

    /// Evaluations so far, accepted or not (the first, at the start,
    /// included).
    pub fn evaluations(&self) -> u32 {
        self.evaluations
    }

    /// The last evaluation's outcome; `None` before the first.
    pub fn last(&self) -> Option<Result<Evaluated, Rejected>> {
        self.last
    }

    /// The program's size, bytes.
    pub fn program_bytes(&self) -> usize {
        self.program.code.len() + self.program.consts.len()
    }

    /// The slots the program needs: (stack, locals, inputs, outputs).
    pub fn slots(&self) -> (u16, u16, u16, u16) {
        let p = &self.program;
        (p.max_stack, p.locals, p.input_count, p.output_count)
    }

    /// Evaluates `event` at `now` (device clock, epoch ms) with `track` as
    /// `gps`. On `Ok` the state, the view and the wake are the new ones.
    /// Counted, and its outcome kept ([`evaluations`](Self::evaluations),
    /// [`last`](Self::last)).
    pub fn evaluate(&mut self, memory: &mut EngineMemory, track: &dyn Track, now: i64, event: Event) -> Result<Evaluated, Rejected> {
        let outcome = self.run(memory, track, now, event);
        self.evaluations = self.evaluations.wrapping_add(1);
        self.last = Some(outcome);
        outcome
    }

    fn run(&mut self, memory: &mut EngineMemory, track: &dyn Track, now: i64, event: Event) -> Result<Evaluated, Rejected> {
        let l = self.layout;
        let t = self.state_lists;
        let EngineMemory { stack, locals, output, inputs, state, state_lists: regions, state_list_lens: lens, view, scratch_view, .. } = memory;
        let inputs = &mut inputs[..l.input_count as usize];
        let output = &mut output[..l.output_count as usize];
        let n = l.state_leaves as usize;

        inputs.fill(Value::Nil);
        if self.has_state {
            inputs[l.in_state as usize] = Value::Bool(true);
            inputs[l.in_state as usize + 1..][..n].copy_from_slice(&state[..n]);
        }
        match event {
            Event::Gps { fix, velocity } => {
                let mut set = |leaf: u16, v: Value<'static>| inputs[leaf as usize] = v;
                set(l.ev_gps, Value::Bool(true));
                if let Some((t, lat, lon)) = fix {
                    set(l.ev_gps_fix, Value::Bool(true));
                    set(l.ev_gps_fix_t, Value::Int(t));
                    set(l.ev_gps_fix_lat, Value::Float(lat));
                    set(l.ev_gps_fix_lon, Value::Float(lon));
                }
                if let Some((speed, heading)) = velocity {
                    set(l.ev_gps_velocity, Value::Bool(true));
                    set(l.ev_gps_velocity_speed, Value::Float(speed));
                    set(l.ev_gps_velocity_heading, Value::Float(heading));
                }
            }
            Event::Wake => inputs[l.ev_wake as usize] = Value::Bool(true),
            Event::Client(bytes) => {
                let codec = self.event_codec.ok_or(Rejected::Undecodable)?;
                let leaves = &mut inputs[l.in_event as usize..][..l.event_leaves as usize];
                let mut sink = EventSink::new(codec.map, self.enums, leaves);
                bitwire::decode(&codec.desc, bytes, &mut sink).map_err(|_| Rejected::Undecodable)?;
            }
        }
        inputs[l.in_timestamp as usize] = Value::Int(now);

        let gps = GpsList {
            track,
            leaves: FixLeaves::of(&l),
        };
        // The input lists by id: `gps`, and the state's as the last
        // evaluation left them (none without a state).
        let saved_lists: [Option<SavedList>; STATE_LISTS] = core::array::from_fn(|k| {
            let s = t.get(k).filter(|_| self.has_state)?;
            saved(&t, lens[k]?, k, s.item_leaves, regions)
        });
        let mut all: [Option<&dyn ListInput<'static>>; 1 + STATE_LISTS] = [None; 1 + STATE_LISTS];
        all[t.gps_list as usize] = Some(&gps);
        for (k, s) in t.iter().enumerate() {
            all[s.list as usize] = saved_lists[k].as_ref().map(|x| x as &dyn ListInput<'static>);
        }
        let lists = &all[..l.list_count as usize];
        let began = embassy_time::Instant::now();
        let ran = vm::run(&self.program, &self.linked, inputs, lists, output, stack, locals);
        self.vm_us = began.elapsed().as_micros() as u32;
        let stats = match ran {
            Ok(stats) => stats,
            Err(f) => {
                error!(
                    "engine {}: {} at pc {}, source bytes {:?}",
                    self.name,
                    f.code.as_str(),
                    f.pc,
                    Dbg(&self.program.span(f.pc))
                );
                return Err(Rejected::Failed(f));
            }
        };

        // Encoded before anything is kept, so a rejected output keeps the
        // last state and view.
        let new_state = &output[l.out_state as usize..][..n];
        if let Some(leaf) = new_state.iter().position(|v| matches!(v, Value::Str(s) if !within(s, self.bundle))) {
            error!("engine {}: state leaf {} is a string outside the bundle", self.name, leaf);
            return Err(Rejected::Output);
        }
        // Where each state list's items come from, checked before anything
        // is kept.
        let mut sources = [Source::Absent; STATE_LISTS];
        for (k, s) in t.iter().enumerate() {
            let at = s.leaf - l.in_state - 1;
            sources[k] = self.source(new_state[at as usize], s, locals, &gps, lens).map_err(|why| {
                error!("engine {}: state list {} (leaf {}): {}", self.name, k, at, why);
                Rejected::Output
            })?;
        }
        let published = &output[l.pub_base as usize..][..l.pub_leaves as usize];
        let mut source = ViewSource::new(self.view_codec.map, self.enums, published, locals, lists);
        let len = match bitwire::encode(&self.view_codec.desc, &mut source, scratch_view) {
            Ok(len) => len,
            Err(e) => {
                error!("engine {}: the view does not encode: {}", self.name, e.code());
                return Err(Rejected::Output);
            }
        };

        state[..n].copy_from_slice(new_state);
        for (k, s) in t.iter().enumerate() {
            // the next evaluation's handle: the list comes through `lists`
            let at = (s.leaf - l.in_state - 1) as usize;
            state[at] = if sources[k] == Source::Absent { Value::Nil } else { Value::List(ListRef::Input(s.list)) };
        }
        keep_lists(&t, &sources, locals, &gps, regions, lens);
        self.has_state = true;
        self.wake = match output[l.out_wake as usize] {
            Value::Int(t) => Some(t),
            // nil (checked at install: a nullable int)
            _ => None,
        };
        let changed = scratch_view[..len] != view[..self.view_len];
        if changed {
            view[..len].copy_from_slice(&scratch_view[..len]);
            self.view_len = len;
        }
        self.stats = stats;
        Ok(Evaluated { changed, wake: self.wake })
    }
}

impl VmEngine {
    /// Where the state list `s`'s next items are, from its handle `h` in
    /// the output: in range, as many as its region holds, items of its
    /// leaves, their strings in the bundle; or why not.
    fn source(&self, h: Value<'static>, s: bundle::StateList, locals: &[Value<'static>], gps: &GpsList, lens: &[Option<u16>; STATE_LISTS]) -> Result<Source, &'static str> {
        let fits = |len: u32| if len <= s.max_items as u32 { Ok(len as u16) } else { Err("more items than the list holds") };
        match h {
            Value::Nil => Ok(Source::Absent),
            Value::List(ListRef::Local { base, len, stride }) => {
                let len = fits(len as u32)?;
                if len > 0 && stride != s.item_leaves {
                    return Err("items of other leaves");
                }
                let items = locals
                    .get(base as usize..base as usize + len as usize * s.item_leaves as usize)
                    .ok_or("items outside the locals")?;
                if items.iter().any(|v| match v {
                    Value::Str(x) => !within(x, self.bundle),
                    Value::List(_) => true,
                    _ => false,
                }) {
                    return Err("an item holds a string outside the bundle, or a list");
                }
                Ok(Source::Local { base, len })
            }
            Value::List(ListRef::Input(id)) if id == self.state_lists.gps_list => {
                if self.layout.fix_leaves != s.item_leaves {
                    return Err("`gps`'s items have other leaves");
                }
                Ok(Source::Gps { len: fits(gps.len())? })
            }
            Value::List(ListRef::Input(id)) => {
                let (from, other) = self.state_lists.of_list(id).ok_or("a list that isn't an input")?;
                if other.item_leaves != s.item_leaves {
                    return Err("items of other leaves");
                }
                // a handle to an input list means it was there
                let len = lens[from].ok_or("an absent list")?;
                Ok(Source::Saved { from, len: fits(len as u32)? })
            }
            _ => Err("not a list"),
        }
    }
}

/// State list `k` (of `t`, `stride` leaves an item) in `regions`, `len`
/// items.
fn saved<'m>(t: &StateLists, len: u16, k: usize, stride: u16, regions: &'m [Value<'static>]) -> Option<SavedList<'m>> {
    let items = regions.get(t.offset(k) as usize..)?.get(..len as usize * stride as usize)?;
    Some(SavedList { items, stride })
}

/// Keeps each state list's items in its region, from `sources` (checked:
/// each within its region and its source): what the next evaluation reads.
///
/// A list passed through from another state list is copied from that
/// one's region, so a region is written only once no list still to be
/// kept reads it; what is left then are lists that pass each other round
/// in cycles (`a` takes `b`'s, `b` takes `a`'s), each turned round slot by
/// slot through one spare value.
fn keep_lists(t: &StateLists, sources: &[Source; STATE_LISTS], locals: &[Value<'static>], gps: &GpsList, regions: &mut [Value<'static>], lens: &mut [Option<u16>; STATE_LISTS]) {
    let n = t.len();
    let stride = |k: usize| t.get(k).map_or(0, |s| s.item_leaves as usize);
    let off = |k: usize| t.offset(k) as usize;
    let slots = |k: usize| sources[k].len().unwrap_or(0) as usize * stride(k);
    // A list that passes itself through keeps its items where they are.
    let mut pending: [bool; STATE_LISTS] = core::array::from_fn(|k| k < n && !matches!(sources[k], Source::Saved { from, .. } if from == k));
    let read_by_pending = |pending: &[bool; STATE_LISTS], k: usize| (0..n).any(|m| m != k && pending[m] && matches!(sources[m], Source::Saved { from, .. } if from == k));
    loop {
        let mut progress = false;
        for k in 0..n {
            if !pending[k] || read_by_pending(&pending, k) {
                continue;
            }
            let (at, len) = (off(k), slots(k));
            match sources[k] {
                Source::Absent => {}
                Source::Local { base, .. } => regions[at..at + len].copy_from_slice(&locals[base as usize..base as usize + len]),
                Source::Gps { .. } => {
                    for (i, v) in regions[at..at + len].iter_mut().enumerate() {
                        *v = gps.read((i / stride(k)) as u32, (i % stride(k)) as u16);
                    }
                }
                Source::Saved { from, .. } => regions.copy_within(off(from)..off(from) + len, at),
            }
            pending[k] = false;
            progress = true;
        }
        if !progress {
            break;
        }
    }
    // Cycles: k takes cycle[1]'s, which takes cycle[2]'s, …, the last k's.
    for k in 0..n {
        if !pending[k] {
            continue;
        }
        let mut cycle = [0usize; STATE_LISTS];
        let (mut c, mut a) = (0, k);
        loop {
            cycle[c] = a;
            c += 1;
            pending[a] = false;
            let Source::Saved { from, .. } = sources[a] else { break };
            a = from;
            if a == k || c == STATE_LISTS {
                break;
            }
        }
        let most = cycle[..c].iter().map(|&a| slots(a)).max().unwrap_or(0);
        let last = cycle[c - 1];
        for i in 0..most {
            // k's item slot, which the last takes (within k's items)
            let spare = (i < slots(last)).then(|| regions[off(k) + i]);
            for w in 0..c - 1 {
                let (a, b) = (cycle[w], cycle[w + 1]);
                if i < slots(a) {
                    regions[off(a) + i] = regions[off(b) + i];
                }
            }
            if let Some(v) = spare {
                regions[off(last) + i] = v;
            }
        }
    }
    for (k, len) in lens.iter_mut().enumerate().take(n) {
        *len = sources[k].len();
    }
}

/// Points every string in `state` at the same bytes in `engine`'s bundle:
/// its enum strings, else its program's constants. `false` if one isn't
/// there (`state` then partly changed: not to be used).
fn repoint(state: &mut [Value<'static>], engine: &VmEngine) -> bool {
    state.iter_mut().all(|v| {
        let Value::Str(s) = *v else {
            return true;
        };
        let in_enums = engine.enums.and_then(|en| (0..en.len()).filter_map(|i| en.get(i)).find(|e| *e == s));
        let in_consts = || {
            let c = engine.program.consts;
            (0..c.len().saturating_sub(s.len()) + 1).find(|&i| c.get(i..i + s.len()) == Some(s)).map(|i| &c[i..i + s.len()])
        };
        match in_enums.or_else(in_consts) {
            Some(new) => {
                *v = Value::Str(new);
                true
            }
            None => false,
        }
    })
}

/// Whether `s` lies within `bundle`.
fn within(s: &[u8], bundle: &[u8]) -> bool {
    let (start, end) = (bundle.as_ptr() as usize, bundle.as_ptr() as usize + bundle.len());
    let at = s.as_ptr() as usize;
    at >= start && at + s.len() <= end
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::track::{RamTrack, TrackLog};
    use std::vec::Vec;

    /// A list table of `lists` ((max_items, item_leaves) each; ids from 1,
    /// `gps` 0), after a fixed part `StateLists` doesn't read.
    fn table(lists: &[(u16, u16)]) -> Vec<u8> {
        let mut b = std::vec![0u8; bundle::layout::BYTES];
        for v in [lists.len() as u16, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for (k, (max, leaves)) in lists.iter().enumerate() {
            for v in [k as u16 + 1, 2 + k as u16, *max, *leaves] {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        b
    }

    /// Each list's items (as ints) after keeping `sources`, the regions
    /// holding `before` (each list's items, as ints).
    fn keep(lists: &[(u16, u16)], before: &[&[i64]], sources: &[Source], locals: &[i64]) -> Vec<Option<Vec<i64>>> {
        let bytes = table(lists);
        let t = StateLists::parse(&bytes).unwrap();
        let mut regions = [Value::Nil; 64];
        let mut lens = [None; STATE_LISTS];
        for (k, items) in before.iter().enumerate() {
            let at = t.offset(k) as usize;
            for (i, x) in items.iter().enumerate() {
                regions[at + i] = Value::Int(*x);
            }
            lens[k] = Some((items.len() / lists[k].1 as usize) as u16);
        }
        let mut s = [Source::Absent; STATE_LISTS];
        s[..sources.len()].copy_from_slice(sources);
        let locals: Vec<Value<'static>> = locals.iter().map(|x| Value::Int(*x)).collect();
        let mut track = RamTrack::<4>::new();
        track.append(7, 1.0, 2.0);
        let gps = GpsList { track: &track, leaves: FixLeaves { t: 0, lat: 1, lon: 2, alt: 3 } };
        keep_lists(&t, &s, &locals, &gps, &mut regions, &mut lens);
        (0..lists.len())
            .map(|k| {
                let n = lens[k]? as usize * lists[k].1 as usize;
                Some(regions[t.offset(k) as usize..][..n].iter().map(|v| if let Value::Int(x) = v { *x } else { -1 }).collect())
            })
            .collect()
    }

    #[test]
    fn lists_are_kept_from_each_source() {
        // built, absent, itself, `gps` (its one fix: t, lat, lon, alt)
        let got = keep(&[(4, 2), (4, 2), (4, 2), (2, 4)], &[&[1, 2], &[3, 4], &[5, 6, 7, 8], &[]], &[
            Source::Local { base: 1, len: 2 },
            Source::Absent,
            Source::Saved { from: 2, len: 2 },
            Source::Gps { len: 1 },
        ], &[0, 10, 11, 12, 13]);
        assert_eq!(got, [Some(std::vec![10, 11, 12, 13]), None, Some(std::vec![5, 6, 7, 8]), Some(std::vec![7, -1, -1, -1])]);
    }

    #[test]
    fn lists_passed_round_are_copied_before_they_are_overwritten() {
        // a chain: 0 takes 1's, 1 takes 2's, 2 is built
        let got = keep(&[(3, 1), (3, 1), (3, 1)], &[&[1], &[2, 2], &[3, 3, 3]], &[
            Source::Saved { from: 1, len: 2 },
            Source::Saved { from: 2, len: 3 },
            Source::Local { base: 0, len: 1 },
        ], &[9]);
        assert_eq!(got, [Some(std::vec![2, 2]), Some(std::vec![3, 3, 3]), Some(std::vec![9])]);
        // two taking the same one, which is built anew
        let got = keep(&[(3, 1), (3, 1), (3, 1)], &[&[1], &[2], &[3, 3]], &[
            Source::Saved { from: 2, len: 2 },
            Source::Saved { from: 2, len: 2 },
            Source::Local { base: 0, len: 1 },
        ], &[9]);
        assert_eq!(got, [Some(std::vec![3, 3]), Some(std::vec![3, 3]), Some(std::vec![9])]);
    }

    #[test]
    fn lists_passed_round_in_cycles_are_turned_round() {
        // a swap, of different lengths
        let got = keep(&[(4, 1), (4, 1)], &[&[1], &[2, 2, 2]], &[Source::Saved { from: 1, len: 3 }, Source::Saved { from: 0, len: 1 }], &[]);
        assert_eq!(got, [Some(std::vec![2, 2, 2]), Some(std::vec![1])]);
        // three round, of different bounds, and one taking from the cycle
        let got = keep(&[(3, 2), (3, 2), (1, 2), (3, 2)], &[&[1, 1], &[2, 2, 2, 2, 2, 2], &[], &[4, 4]], &[
            Source::Saved { from: 1, len: 3 },
            Source::Saved { from: 2, len: 0 },
            Source::Saved { from: 0, len: 1 },
            Source::Saved { from: 0, len: 1 },
        ], &[]);
        assert_eq!(got, [Some(std::vec![2, 2, 2, 2, 2, 2]), Some(std::vec![]), Some(std::vec![1, 1]), Some(std::vec![1, 1])]);
    }
}
