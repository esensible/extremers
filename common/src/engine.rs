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
//! An evaluation that fails (`R-…`) changes nothing: the state stays, the
//! event is dropped, and the engine keeps running.

use bundle::{
    EventSink, Layout, Limits, ViewSource,
    map::Enums,
    validate::Codec,
};
use extreme_protocol::STATE_MAX;
use vm::{Fail, HostFn, ListInput, LoadError, Program, Stats, Value};

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
};

/// Every buffer an engine needs, for one engine at a time: the host owns
/// one, `'static` (on the C6 a static, not the heap or the main stack), and
/// lends it to the active engine's every call.
pub struct EngineMemory {
    stack: [Value<'static>; STACK_SLOTS],
    locals: [Value<'static>; LOCAL_SLOTS],
    output: [Value<'static>; OUTPUT_SLOTS],
    inputs: [Value<'static>; INPUT_SLOTS],
    /// The last evaluation's state.
    state: [Value<'static>; STATE_SLOTS],
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
    view_codec: Codec<'static>,
    event_codec: Option<Codec<'static>>,
    enums: Option<Enums<'static>>,
    /// Whether `EngineMemory::state` holds a state (none before the first
    /// evaluation).
    has_state: bool,
    view_len: usize,
    wake: Option<i64>,
    stats: Stats,
}

impl VmEngine {
    /// Links `package`'s program (validated when it was loaded) and runs
    /// its first evaluation, `{gps: {}}` at `now`, so a view exists before
    /// the first event.
    pub fn start(package: &Package, now: i64, memory: &mut EngineMemory, track: &dyn Track) -> Result<VmEngine, StartError> {
        let engine = package.engine.ok_or(StartError::NoEngine)?;
        let mut linked = [None; IMPORTS];
        vm::link(&engine.program, &sailing::BUILTINS, &mut linked).map_err(StartError::Link)?;
        let mut e = VmEngine {
            name: package.name,
            bundle: package.archive.bytes(),
            program: engine.program,
            linked,
            layout: engine.layout,
            view_codec: engine.view,
            event_codec: engine.event,
            enums: engine.enums,
            has_state: false,
            view_len: 0,
            wake: None,
            stats: Stats::default(),
        };
        e.evaluate(memory, track, now, Event::Gps { fix: None, velocity: None })
            .map_err(StartError::First)?;
        Ok(e)
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
    pub fn evaluate(&mut self, memory: &mut EngineMemory, track: &dyn Track, now: i64, event: Event) -> Result<Evaluated, Rejected> {
        let l = self.layout;
        let EngineMemory { stack, locals, output, inputs, state, view, scratch_view, .. } = memory;
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
        let lists: [Option<&dyn ListInput<'static>>; 1] = [Some(&gps)];
        let stats = match vm::run(&self.program, &self.linked, inputs, &lists, output, stack, locals) {
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
        let published = &output[l.pub_base as usize..][..l.pub_leaves as usize];
        let mut source = ViewSource::new(self.view_codec.map, self.enums, published, locals, &lists);
        let len = match bitwire::encode(&self.view_codec.desc, &mut source, scratch_view) {
            Ok(len) => len,
            Err(e) => {
                error!("engine {}: the view does not encode: {}", self.name, e.code());
                return Err(Rejected::Output);
            }
        };

        state[..n].copy_from_slice(new_state);
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

/// Whether `s` lies within `bundle`.
fn within(s: &[u8], bundle: &[u8]) -> bool {
    let (start, end) = (bundle.as_ptr() as usize, bundle.as_ptr() as usize + bundle.len());
    let at = s.as_ptr() as usize;
    at >= start && at + s.len() <= end
}
