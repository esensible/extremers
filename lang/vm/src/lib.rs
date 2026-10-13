//! The executor. It runs on the device; nothing else in `lang/` does.
//!
//! The host owns every buffer: the inputs, the value stack and the locals.
//! It sizes them from the program header once, at load. Evaluation never
//! allocates and never recurses, and every failure is a returned code, never
//! a panic.

#![no_std]

pub use bytecode::{Check, Import, Kind, LoadError, MAX_HOST_RESULTS, Program};
use bytecode::{Builtin, Op};

/// A builtin the host provides (spec §2.4), like `expr.Function` in Go.
///
/// - `signature` must equal the one the compiler derives from the builtin's
///   declaration, e.g. `distance({lat:f,lon:f},{lat:f,lon:f})f`; programs are
///   linked by it.
/// - `call` receives the arguments' leaves (records flattened in field-name
///   order; a numeric argument may be an int or a float) and writes the
///   result's `results` leaves. Returning `false` fails the evaluation with
///   `R-ARG`.
/// - It must be pure: the same arguments always give the same result.
pub struct HostFn {
    pub signature: &'static str,
    pub args: u16,
    pub results: u16,
    pub call: for<'a> fn(&[Value<'a>], &mut [Value<'a>]) -> bool,
}

/// A value on the stack, in a local, or given as an input.
///
/// On the 32-bit device this is 16 bytes (a tag, then an 8-byte payload);
/// the compiler's memory figures use that size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<'a> {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(&'a [u8]),
    /// A handle to a list: an input list, or items laid out in locals.
    List(ListRef),
}

/// Where a list's items are. Both kinds live for the whole evaluation: inputs
/// don't change during it, and the locals of a list literal or of a list the
/// program builds are reserved for it alone. So a handle can be copied
/// freely. A `Local` handle in the output stays readable after `run`
/// returns, until the locals are reused (the next `run` clears them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListRef {
    /// Input list `id`, read through the host's accessor.
    Input(u16),
    /// `len` items of `stride` leaves each, in locals from `base`.
    Local { base: u16, len: u16, stride: u16 },
}

/// A list input, read in place: the host decides where the items live
/// (e.g. in flash). Items are flattened into leaves like records (spec
/// §7.3): `read(i, k)` is leaf `k` of item `i`.
pub trait ListInput<'a> {
    fn len(&self) -> u32;
    fn read(&self, item: u32, leaf: u16) -> Value<'a>;
}

/// Bytes per stack, local or output slot on the device.
pub const SLOT_BYTES: usize = 16;

#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::size_of::<Value<'static>>() == SLOT_BYTES);

/// Why an evaluation failed (spec §5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// `nil` where a value is needed.
    Nil,
    /// `% 0`.
    Div0,
    /// An index out of range.
    Index,
    /// A builtin was given an argument it can't use.
    Arg,
    /// The output is outside its schema (a safety net: the compiler proves it can't be).
    Range,
    /// An input doesn't match its declaration, or a host buffer is too small.
    Input,
    /// A value of the wrong kind: a compiler bug.
    Type,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Nil => "R-NIL",
            ErrorCode::Div0 => "R-DIV0",
            ErrorCode::Index => "R-INDEX",
            ErrorCode::Arg => "R-ARG",
            ErrorCode::Range => "R-RANGE",
            ErrorCode::Input => "R-INPUT",
            ErrorCode::Type => "R-TYPE",
        }
    }
}

/// A failed evaluation: the code, and the offset of the instruction that
/// failed (the program's span table maps it to source).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fail {
    pub code: ErrorCode,
    pub pc: u32,
}

/// What an evaluation used, measured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// The deepest the value stack went, in slots.
    pub stack_high: u16,
    /// Instructions executed.
    pub steps: u32,
}

/// Limits a host puts on programs it will load.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_stack: u16,
    pub max_locals: u16,
}

/// Parses and validates a program. `scratch` must hold at least as many
/// entries as the program has code bytes (the host can lend any buffer: it
/// is only used during this call).
pub fn load<'a>(bytes: &'a [u8], scratch: &mut [u16], limits: &Limits) -> Result<Program<'a>, LoadError> {
    let p = Program::parse(bytes)?;
    if p.max_stack > limits.max_stack || p.locals > limits.max_locals {
        return Err(LoadError::OverLimit);
    }
    p.validate(scratch)?;
    Ok(p)
}

/// Links a program's imports to the host's builtins: `linked[i]` becomes the
/// builtin for import `i`, matched by signature. `linked` must hold at least
/// `import_count` entries.
pub fn link<'h>(p: &Program, registry: &[&'h HostFn], linked: &mut [Option<&'h HostFn>]) -> Result<(), LoadError> {
    if linked.len() < p.import_count as usize {
        return Err(LoadError::OverLimit);
    }
    for i in 0..p.import_count {
        let im = p.import(i);
        let f = registry
            .iter()
            .find(|f| f.signature.as_bytes() == im.signature)
            .ok_or(LoadError::Unlinked(i))?;
        if f.args != im.args || f.results != im.results {
            return Err(LoadError::BadImport(i));
        }
        linked[i as usize] = Some(*f);
    }
    Ok(())
}

/// Evaluates a validated program.
///
/// - `inputs` are the input leaves (spec §7: records flattened in field-name
///   order, an optional record led by a presence leaf); each is checked
///   against its declaration first.
/// - `output` receives the output's leaves.
/// - `host` is the table `link` filled in.
/// - `lists[i]` is input list `i`, or `None` when it's absent (it sits in an
///   optional record that's nil). The leaves at list positions in `inputs`
///   are ignored.
/// - `stack` and `locals` must hold at least `max_stack` and `locals` slots.
pub fn run<'a>(
    p: &Program<'a>,
    host: &[Option<&HostFn>],
    inputs: &[Value<'a>],
    lists: &[Option<&dyn ListInput<'a>>],
    output: &mut [Value<'a>],
    stack: &mut [Value<'a>],
    locals: &mut [Value<'a>],
) -> Result<Stats, Fail> {
    let fail = |code, pc: usize| Fail { code, pc: pc as u32 };
    if inputs.len() != p.input_count as usize
        || output.len() != p.output_count as usize
        || stack.len() < p.max_stack as usize
        || locals.len() < p.locals as usize
        || host.len() < p.import_count as usize
        || host[..p.import_count as usize].iter().any(Option::is_none)
        || lists.len() != p.list_count as usize
    {
        return Err(fail(ErrorCode::Input, 0));
    }
    // One pass over the checks (they're variable-length: `p.input(i)` would
    // re-read the `i` before each one).
    for (v, c) in inputs.iter().zip(p.input_checks()) {
        if !fits(p, &c, v) {
            return Err(fail(ErrorCode::Input, 0));
        }
    }
    if !groups_hold(p.input_groups(), inputs) {
        return Err(fail(ErrorCode::Input, 0));
    }
    locals.fill(Value::Nil);

    let code = p.code;
    let mut stats = Stats::default();
    let mut sp: usize = 0;
    // `stats.stack_high`, kept as a usize until `Return` (the stack is at
    // most `max_stack`, a u16, so the conversion is exact).
    let mut high: usize = 0;
    let mut pc: usize = 0;
    loop {
        stats.steps += 1;
        // Validated at load, so decoding can't fail; treat it as a type error if it does.
        let (op, next) = Op::decode(code, pc).map_err(|_| fail(ErrorCode::Type, pc))?;
        let at = pc;
        pc = next;
        macro_rules! pop {
            () => {{
                sp -= 1;
                stack[sp]
            }};
        }
        macro_rules! push {
            ($v:expr) => {{
                stack[sp] = $v;
                sp += 1;
                if sp > high {
                    high = sp;
                }
            }};
        }
        match op {
            Op::PushNil => push!(Value::Nil),
            Op::PushTrue => push!(Value::Bool(true)),
            Op::PushFalse => push!(Value::Bool(false)),
            Op::PushInt(v) => push!(Value::Int(v)),
            Op::PushFloat(v) => push!(Value::Float(v)),
            Op::PushStr(o) => push!(Value::Str(p.string(o).ok_or(fail(ErrorCode::Type, at))?)),
            Op::Input(i) => push!(inputs[i as usize]),
            Op::Load(i) => push!(locals[i as usize]),
            Op::Store(i) => locals[i as usize] = pop!(),
            Op::Pop => sp -= 1,
            Op::Dup => push!(stack[sp - 1]),
            // Validated at load: `s` is below the height here.
            Op::Peek(s) => push!(stack[s as usize]),
            Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod | Op::Pow => {
                let b = pop!();
                let a = pop!();
                push!(arith(op, a, b).map_err(|c| fail(c, at))?);
            }
            Op::Eq => {
                let b = pop!();
                let a = pop!();
                push!(Value::Bool(equal(&a, &b)));
            }
            Op::Ne => {
                let b = pop!();
                let a = pop!();
                push!(Value::Bool(!equal(&a, &b)));
            }
            Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                let b = pop!();
                let a = pop!();
                let (x, y) = numbers(a, b).map_err(|c| fail(c, at))?;
                let r = match (x, y) {
                    (Num::I(x), Num::I(y)) => cmp_with(op, x.partial_cmp(&y)),
                    (x, y) => cmp_with(op, x.f().partial_cmp(&y.f())),
                };
                push!(Value::Bool(r));
            }
            Op::Neg => match pop!() {
                Value::Int(v) => push!(Value::Int(v.wrapping_neg())),
                Value::Float(v) => push!(Value::Float(-v)),
                Value::Nil => return Err(fail(ErrorCode::Nil, at)),
                _ => return Err(fail(ErrorCode::Type, at)),
            },
            Op::Not => match pop!() {
                Value::Bool(v) => push!(Value::Bool(!v)),
                Value::Nil => return Err(fail(ErrorCode::Nil, at)),
                _ => return Err(fail(ErrorCode::Type, at)),
            },
            Op::Jump(t) => pc = t as usize,
            Op::JumpIfFalse(t) => match pop!() {
                Value::Bool(false) => pc = t as usize,
                Value::Bool(true) => {}
                Value::Nil => return Err(fail(ErrorCode::Nil, at)),
                _ => return Err(fail(ErrorCode::Type, at)),
            },
            Op::JumpIfNotNil(t) => {
                if pop!() != Value::Nil {
                    pc = t as usize;
                }
            }
            Op::Call(b, n) => {
                let n = n as usize;
                let args = &stack[sp - n..sp];
                let r = call(b, args).map_err(|c| fail(c, at))?;
                sp -= n;
                push!(r);
            }
            Op::CallHost { import, args, results } => {
                let f = host[import as usize].ok_or(fail(ErrorCode::Type, at))?;
                let (args, results) = (args as usize, results as usize);
                let mut out = [Value::Nil; MAX_HOST_RESULTS as usize];
                if !(f.call)(&stack[sp - args..sp], &mut out[..results]) {
                    return Err(fail(ErrorCode::Arg, at));
                }
                sp -= args;
                for v in &out[..results] {
                    push!(*v);
                }
            }
            Op::PushInputList(id) => match lists[id as usize] {
                Some(_) => push!(Value::List(ListRef::Input(id))),
                None => push!(Value::Nil),
            },
            Op::MakeList { base, len, stride } => push!(Value::List(ListRef::Local { base, len, stride })),
            // A built list's item `i` into its block; the operands were
            // checked at load (the block lies inside the locals).
            Op::ListStore { base, cap, stride } => {
                let i = match pop!() {
                    Value::Int(i) if i >= 0 && i < cap as i64 => i as usize,
                    _ => return Err(fail(ErrorCode::Type, at)),
                };
                let at_item = base as usize + i * stride as usize;
                for k in (0..stride as usize).rev() {
                    locals[at_item + k] = pop!();
                }
            }
            Op::ListMake { base, cap, stride } => match pop!() {
                Value::Int(n) if n >= 0 && n <= cap as i64 => push!(Value::List(ListRef::Local { base, len: n as u16, stride })),
                _ => return Err(fail(ErrorCode::Type, at)),
            },
            Op::ListLen => {
                let n = list_len(p, lists, pop!()).map_err(|c| fail(c, at))?;
                push!(Value::Int(n as i64));
            }
            Op::ListRead(leaf) => {
                let i = pop!();
                let h = pop!();
                let n = list_len(p, lists, h).map_err(|c| fail(c, at))?;
                let i = match i {
                    Value::Int(i) if i >= 0 && (i as u64) < n as u64 => i as u32,
                    Value::Int(_) => return Err(fail(ErrorCode::Index, at)),
                    Value::Nil => return Err(fail(ErrorCode::Nil, at)),
                    _ => return Err(fail(ErrorCode::Type, at)),
                };
                let v = match h {
                    Value::List(ListRef::Input(id)) => {
                        let v = lists[id as usize].ok_or(fail(ErrorCode::Type, at))?.read(i, leaf);
                        let (_, leaves) = p.list(id);
                        if leaf >= leaves || !fits(p, &p.list_item_check(id, leaf), &v) {
                            return Err(fail(ErrorCode::Input, at));
                        }
                        v
                    }
                    Value::List(ListRef::Local { base, stride, .. }) => {
                        if leaf >= stride {
                            return Err(fail(ErrorCode::Type, at));
                        }
                        *locals.get(base as usize + i as usize * stride as usize + leaf as usize).ok_or(fail(ErrorCode::Type, at))?
                    }
                    _ => return Err(fail(ErrorCode::Type, at)),
                };
                push!(v);
            }
            Op::Error(code) => {
                let code = match code {
                    1 => ErrorCode::Nil,
                    2 => ErrorCode::Index,
                    _ => ErrorCode::Arg,
                };
                return Err(fail(code, at));
            }
            Op::AssertNotNil => {
                if pop!() == Value::Nil {
                    return Err(fail(ErrorCode::Nil, at));
                }
            }
            Op::Return(n) => {
                let n = n as usize;
                for ((slot, c), &v) in output.iter_mut().zip(p.output_checks()).zip(&stack[sp - n..sp]) {
                    let mut v = v;
                    // A position typed float may receive an int: expr keeps the
                    // winner's type (`c ? 1 : 2.5`, `min(1, 1.0)` give int 1).
                    if let (Kind::Float, Value::Int(x)) = (c.kind, v) {
                        v = Value::Float(x as f64);
                    }
                    if !fits(p, &c, &v) {
                        let code = if kind_of(&v) == Some(c.kind) { ErrorCode::Range } else { ErrorCode::Type };
                        return Err(fail(code, at));
                    }
                    *slot = v;
                }
                if !groups_hold(p.output_groups(), output) {
                    return Err(fail(ErrorCode::Range, at));
                }
                stats.stack_high = high as u16;
                return Ok(stats);
            }
        }
    }
}

/// A list handle's length. An input list longer than its `maxItems` is a
/// host error: the checker's bounds rely on it.
fn list_len(p: &Program, lists: &[Option<&dyn ListInput>], h: Value) -> Result<u32, ErrorCode> {
    match h {
        Value::List(ListRef::Input(id)) => {
            let n = lists[id as usize].ok_or(ErrorCode::Type)?.len();
            let (max, _) = p.list(id);
            if n > max as u32 { Err(ErrorCode::Input) } else { Ok(n) }
        }
        Value::List(ListRef::Local { len, .. }) => Ok(len as u32),
        Value::Nil => Err(ErrorCode::Nil),
        _ => Err(ErrorCode::Type),
    }
}

fn kind_of(v: &Value) -> Option<Kind> {
    match v {
        Value::Nil => None,
        Value::Bool(_) => Some(Kind::Bool),
        Value::Int(_) => Some(Kind::Int),
        Value::Float(_) => Some(Kind::Float),
        Value::Str(_) => Some(Kind::Str),
        Value::List(_) => Some(Kind::List),
    }
}

/// Whether every variant group has exactly one variant present (spec §7.3):
/// of its members' presence leaves exactly one is non-nil, unless the
/// group's guard leaf is nil (the optional record it sits in is absent).
/// The leaf numbers were checked at load.
#[inline]
fn groups_hold(groups: bytecode::Groups, leaves: &[Value]) -> bool {
    for g in groups {
        if g.guard != bytecode::NO_GUARD && leaves[g.guard as usize] == Value::Nil {
            continue;
        }
        let mut present = 0u16;
        for m in g.members() {
            if leaves[m as usize] != Value::Nil {
                present += 1;
            }
        }
        if present != 1 {
            return false;
        }
    }
    true
}

/// Whether `v` satisfies `c`.
#[inline]
fn fits(p: &Program, c: &Check, v: &Value) -> bool {
    match (c.kind, v) {
        // A list position in an input record: the list comes through `lists`.
        (Kind::List, _) => true,
        (_, Value::Nil) => c.nullable,
        (Kind::Bool, Value::Bool(_)) => true,
        (Kind::Float, Value::Float(_)) => true,
        (Kind::Int, Value::Int(x)) => c.lo <= *x && *x <= c.hi,
        (Kind::Str, Value::Str(s)) => s.len() <= c.max_len as usize && p.in_enum(c, s),
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum Num {
    I(i64),
    F(f64),
}

impl Num {
    fn f(self) -> f64 {
        match self {
            Num::I(v) => v as f64,
            Num::F(v) => v,
        }
    }
}

fn number(v: Value) -> Result<Num, ErrorCode> {
    match v {
        Value::Int(x) => Ok(Num::I(x)),
        Value::Float(x) => Ok(Num::F(x)),
        Value::Nil => Err(ErrorCode::Nil),
        _ => Err(ErrorCode::Type),
    }
}

fn numbers(a: Value, b: Value) -> Result<(Num, Num), ErrorCode> {
    Ok((number(a)?, number(b)?))
}

fn cmp_with(op: Op, o: Option<core::cmp::Ordering>) -> bool {
    use core::cmp::Ordering::*;
    match (op, o) {
        (_, None) => false, // NaN compares false
        (Op::Lt, Some(o)) => o == Less,
        (Op::Le, Some(o)) => o != Greater,
        (Op::Gt, Some(o)) => o == Greater,
        (Op::Ge, Some(o)) => o != Less,
        _ => false,
    }
}

/// expr's `==`: ints and floats compare numerically; nil equals only nil;
/// values of different kinds are unequal.
fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Nil, Value::Nil) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Int(x), Value::Float(y)) => (*x as f64) == *y,
        (Value::Float(x), Value::Int(y)) => *x == (*y as f64),
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        _ => false,
    }
}

fn arith<'a>(op: Op, a: Value<'a>, b: Value<'a>) -> Result<Value<'a>, ErrorCode> {
    let (x, y) = numbers(a, b)?;
    Ok(match (op, x, y) {
        (Op::Add, Num::I(x), Num::I(y)) => Value::Int(x.wrapping_add(y)),
        (Op::Sub, Num::I(x), Num::I(y)) => Value::Int(x.wrapping_sub(y)),
        (Op::Mul, Num::I(x), Num::I(y)) => Value::Int(x.wrapping_mul(y)),
        (Op::Mod, Num::I(_), Num::I(0)) => return Err(ErrorCode::Div0),
        (Op::Mod, Num::I(x), Num::I(y)) => Value::Int(x.wrapping_rem(y)),
        (Op::Mod, _, _) => return Err(ErrorCode::Type),
        (Op::Add, x, y) => Value::Float(x.f() + y.f()),
        (Op::Sub, x, y) => Value::Float(x.f() - y.f()),
        (Op::Mul, x, y) => Value::Float(x.f() * y.f()),
        (Op::Div, x, y) => Value::Float(x.f() / y.f()),
        (Op::Pow, x, y) => Value::Float(libm::pow(x.f(), y.f())),
        _ => return Err(ErrorCode::Type),
    })
}

fn call<'a>(b: Builtin, args: &[Value<'a>]) -> Result<Value<'a>, ErrorCode> {
    let one = || number(args[0]);
    Ok(match b {
        Builtin::Abs => match one()? {
            Num::I(v) => Value::Int(v.wrapping_abs()),
            Num::F(v) => Value::Float(libm::fabs(v)),
        },
        // `as` truncates toward zero, saturates, and gives 0 for NaN (spec §2.2).
        Builtin::Int => match one()? {
            Num::I(v) => Value::Int(v),
            Num::F(v) => Value::Int(v as i64),
        },
        Builtin::Float => Value::Float(one()?.f()),
        Builtin::Round => Value::Float(libm::round(one()?.f())),
        Builtin::Floor => Value::Float(libm::floor(one()?.f())),
        Builtin::Ceil => Value::Float(libm::ceil(one()?.f())),
        // The first winner stays (strict comparison), so a NaN in first place
        // sticks and later NaNs are skipped (spec §2.3).
        Builtin::Min | Builtin::Max => {
            let mut best = args[0];
            let mut best_n = number(best)?;
            for &v in &args[1..] {
                let n = number(v)?;
                let better = match (b, n, best_n) {
                    (Builtin::Min, Num::I(x), Num::I(y)) => x < y,
                    (Builtin::Max, Num::I(x), Num::I(y)) => x > y,
                    (Builtin::Min, x, y) => x.f() < y.f(),
                    (_, x, y) => x.f() > y.f(),
                };
                if better {
                    best = v;
                    best_n = n;
                }
            }
            best
        }
    })
}
