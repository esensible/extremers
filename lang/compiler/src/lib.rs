//! The compiler and checker: source + interface → bytecode, or diagnostics.
//! Runs off-device only. `run` evaluates a program on the real VM (the same
//! no_std executor the device uses), for `langc check` sample runs and tests.

pub mod check;
pub mod diag;
pub mod emit;
pub mod schema;
pub mod syntax;
pub mod types;

use diag::Diag;
use serde_json::{Value as Json, json};
use types::{Ty, leaves};
use vm::{SLOT_BYTES, Value};

/// What the compiler is told about the host.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// The most RAM the program may need, in bytes (`E-LIMIT` above it).
    pub limit_bytes: Option<u32>,
    /// Whether the host loads code into RAM (otherwise it runs from flash).
    pub code_in_ram: bool,
}

/// The memory a program needs on the device, worked out at compile time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    pub stack_slots: u16,
    pub locals: u16,
    /// The output's leaves, which the host holds for the VM to write.
    pub output_slots: u16,
    pub slot_bytes: u32,
    /// Code and constants (the whole program image).
    pub program_bytes: u32,
    /// What counts against the limit: stack + locals + output (+ program if in RAM).
    pub total_bytes: u32,
}

impl Memory {
    pub fn to_json(&self) -> Json {
        json!({
            "stack_slots": self.stack_slots, "locals": self.locals, "output_slots": self.output_slots,
            "slot_bytes": self.slot_bytes, "program_bytes": self.program_bytes, "total_bytes": self.total_bytes,
        })
    }
}

pub struct Compiled {
    pub bytes: Vec<u8>,
    pub memory: Memory,
}

pub fn compile(source: &str, interface: &Json, opts: &Options) -> Result<Compiled, Vec<Diag>> {
    let iface = schema::parse_interface(interface)?;
    let ast = syntax::parse(source).map_err(|d| vec![d])?;
    let checked = check::check(&ast, &iface)?;
    let em = emit::emit(&checked, &iface);
    let slot_bytes = SLOT_BYTES as u32;
    let program_bytes = em.bytes.len() as u32;
    let mut total = (em.max_stack as u32 + em.locals as u32 + em.output_leaves as u32) * slot_bytes;
    if opts.code_in_ram {
        total += program_bytes;
    }
    let memory = Memory {
        stack_slots: em.max_stack,
        locals: em.locals,
        output_slots: em.output_leaves,
        slot_bytes,
        program_bytes,
        total_bytes: total,
    };
    if let Some(limit) = opts.limit_bytes {
        if total > limit {
            let code = if opts.code_in_ram { format!(", program {program_bytes}") } else { String::new() };
            return Err(vec![Diag::error(
                "E-LIMIT",
                ast.span,
                format!(
                    "the program needs {total} bytes (stack {} × {slot_bytes}, locals {} × {slot_bytes}, output {} × {slot_bytes}{code}); the limit is {limit}",
                    em.max_stack, em.locals, em.output_leaves
                ),
            )
            .help("nest to the left: `a + b + c` needs less stack than `a + (b + c)`")]);
        }
    }
    Ok(Compiled { bytes: em.bytes, memory })
}

/// A program's code as one instruction per line (`pc: op`), after a line
/// with its header figures. For reading what the compiler emitted.
pub fn disassemble(bytes: &[u8]) -> Result<String, String> {
    let p = bytecode::Program::parse(bytes).map_err(|e| format!("parse: {e:?}"))?;
    let mut out = format!(
        "max_stack {} locals {} inputs {} outputs {} code {} B\n",
        p.max_stack,
        p.locals,
        p.input_count,
        p.output_count,
        p.code.len()
    );
    let mut pc = 0;
    while pc < p.code.len() {
        let (op, next) = bytecode::Op::decode(p.code, pc).map_err(|e| format!("decode at {pc}: {e:?}"))?;
        out += &format!("{pc:5}: {op:?}\n");
        pc = next;
    }
    Ok(out)
}

/// The result of one evaluation on the VM.
#[derive(Debug)]
pub enum RunResult {
    Ok { value: Json, stack_high: u16, steps: u32 },
    Failed { code: &'static str, span: Option<(u32, u32)> },
}

/// Loads `bytes`, links it to the host's builtins, and evaluates it with
/// inputs given as JSON (by name), on the real VM.
pub fn run(bytes: &[u8], interface: &Json, inputs: &serde_json::Map<String, Json>, host: &[&vm::HostFn]) -> Result<RunResult, String> {
    let iface = schema::parse_interface(interface).map_err(|d| format!("{d:?}"))?;
    let program = bytes.to_vec();
    let mut scratch = vec![0u16; program.len()];
    let limits = vm::Limits { max_stack: u16::MAX, max_locals: u16::MAX };
    let p = vm::load(&program, &mut scratch, &limits).map_err(|e| format!("load: {e:?}"))?;
    let mut linked = vec![None; p.import_count as usize];
    vm::link(&p, host, &mut linked).map_err(|e| match e {
        vm::LoadError::Unlinked(i) => format!("the host has no builtin `{}`", String::from_utf8_lossy(p.import(i).signature)),
        e => format!("link: {e:?}"),
    })?;
    // Strings in the inputs are borrowed from these JSON values for the run.
    let mut values = Vec::new();
    for inp in &iface.inputs {
        let v = inputs.get(&inp.name).unwrap_or(&Json::Null);
        flatten(v, &inp.ty, &mut values).ok_or_else(|| format!("input `{}`: {v} doesn't fit its schema", inp.name))?;
    }
    // Each input list, as an in-memory accessor over its items' leaves (a
    // device host would read them from wherever they're stored).
    let mut accessors: Vec<Option<JsonList>> = Vec::new();
    for l in &iface.lists {
        let mut v = inputs.get(&iface.inputs[l.input].name).unwrap_or(&Json::Null);
        for k in &l.path {
            v = v.get(k).unwrap_or(&Json::Null);
        }
        accessors.push(match v {
            Json::Array(items) => {
                let mut flat = Vec::new();
                for it in items {
                    let mut leaves_of = Vec::new();
                    flatten(it, &l.item, &mut leaves_of).ok_or_else(|| format!("a list item {it} doesn't fit its schema"))?;
                    flat.push(leaves_of);
                }
                Some(JsonList { items: flat })
            }
            Json::Null => None,
            other => return Err(format!("{other} isn't a list")),
        });
    }
    let lists: Vec<Option<&dyn vm::ListInput>> = accessors.iter().map(|a| a.as_ref().map(|l| l as &dyn vm::ListInput)).collect();
    let mut output = vec![Value::Nil; p.output_count as usize];
    let mut stack = vec![Value::Nil; p.max_stack as usize];
    let mut locals = vec![Value::Nil; p.locals as usize];
    Ok(match vm::run(&p, &linked, &values, &lists, &mut output, &mut stack, &mut locals) {
        Ok(stats) => {
            let mut leaves_iter = output.iter();
            RunResult::Ok { value: unflatten(&mut leaves_iter, &iface.output), stack_high: stats.stack_high, steps: stats.steps }
        }
        Err(f) => RunResult::Failed { code: f.code.as_str(), span: p.span(f.pc) },
    })
}

/// A list input held in memory: each item's leaves.
struct JsonList<'a> {
    items: Vec<Vec<Value<'a>>>,
}

impl<'a> vm::ListInput<'a> for JsonList<'a> {
    fn len(&self) -> u32 {
        self.items.len() as u32
    }
    fn read(&self, item: u32, leaf: u16) -> Value<'a> {
        self.items[item as usize][leaf as usize]
    }
}

/// A JSON value as leaves of type `t` (spec §7): records in field-name
/// order; an optional record is a presence leaf, then its fields (all nil
/// when it's absent).
fn flatten<'a>(v: &'a Json, t: &Ty, out: &mut Vec<Value<'a>>) -> Option<()> {
    match (t, v) {
        // A list's leaf holds nothing: the list comes through its accessor.
        (Ty::List(..), _) => out.push(Value::Nil),
        (Ty::Opt(inner), Json::Null) => {
            out.extend(std::iter::repeat_n(Value::Nil, leaves(t) as usize));
            let _ = inner;
        }
        (Ty::Opt(inner), _) if matches!(**inner, Ty::Rec(..)) => {
            out.push(Value::Bool(true));
            flatten(v, inner, out)?;
        }
        (Ty::Opt(inner), _) => flatten(v, inner, out)?,
        (Ty::Rec(f, _), Json::Object(o)) => {
            for (k, ft) in f {
                flatten(o.get(k).unwrap_or(&Json::Null), ft, out)?;
            }
        }
        (Ty::Bool, Json::Bool(b)) => out.push(Value::Bool(*b)),
        (Ty::Int(_), Json::Number(n)) => out.push(Value::Int(n.as_i64()?)),
        (Ty::Float, Json::Number(n)) => out.push(Value::Float(n.as_f64()?)),
        (Ty::Str(_), Json::String(s)) => out.push(Value::Str(s.as_bytes())),
        _ => return None,
    }
    Some(())
}

/// Output leaves back into a value, as tagged JSON (floats as IEEE bits so
/// nothing is lost). Absent optional fields are left out of records.
fn unflatten<'a, 'b>(leaves_iter: &mut impl Iterator<Item = &'b Value<'a>>, t: &Ty) -> Json
where
    'a: 'b,
{
    match t {
        Ty::Rec(f, _) => {
            let mut fields = serde_json::Map::new();
            for (k, ft) in f {
                let v = unflatten(leaves_iter, ft);
                if v["kind"] != "nil" {
                    fields.insert(k.clone(), v);
                }
            }
            json!({"kind": "record", "fields": fields})
        }
        Ty::Opt(inner) if matches!(**inner, Ty::Rec(..)) => {
            let present = *leaves_iter.next().unwrap() != Value::Nil;
            let v = unflatten(leaves_iter, inner);
            if present { v } else { json!({"kind": "nil"}) }
        }
        _ => from_value(leaves_iter.next().unwrap()),
    }
}

/// A VM value as JSON, with floats as IEEE bits so nothing is lost.
pub fn from_value(v: &Value) -> Json {
    match v {
        Value::Nil => json!({"kind": "nil"}),
        Value::Bool(b) => json!({"kind": "bool", "bool": b}),
        Value::Int(i) => json!({"kind": "int", "int": i.to_string()}),
        Value::Float(f) => json!({"kind": "float", "bits": format!("{:016x}", f.to_bits())}),
        Value::Str(s) => json!({"kind": "string", "string": String::from_utf8_lossy(s)}),
        Value::List(_) => json!({"kind": "list-handle"}),
    }
}
