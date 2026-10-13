//! What the host's engine holds, as the studio shows it: the state leaves
//! as plain JSON, and a failed evaluation's source span.
//!
//! A package's state type comes from its bundle's `source.json` (the
//! engine's schemas, verbatim): the interface engine/profile makes of them,
//! whose output's `state` is laid out leaf by leaf as the program keeps it
//! (records in field-name order; an optional record a presence leaf, then
//! its fields). Its hash must be the program's, so the leaves are read with
//! the type they were compiled against.

use common::{engine::SavedList, package::Package};
use compiler::types::{Ty, unopt};
use serde_json::{Map, Value as Json, json};
use vm::{ListInput, ListRef, Value};

/// How to read one package's state leaves.
pub struct Decoder {
    state: Ty,
    /// `engine.bin`, for the spans of failures.
    program: &'static [u8],
}

impl Decoder {
    /// The decoder of `package`'s engine, or why there is none.
    pub fn of(package: &Package) -> Result<Decoder, String> {
        let archive = &package.archive;
        let program = archive.file("engine.bin").ok_or("the bundle has no engine.bin")?;
        let source = archive.file("source.json").ok_or("the bundle has no source.json (the engine's schemas): its state can't be read")?;
        let source: Json = serde_json::from_slice(source).map_err(|e| format!("source.json: {e}"))?;
        let schema = |key: &str| -> Result<Option<Json>, String> {
            match source.get(key) {
                None | Some(Json::Null) => Ok(None),
                Some(Json::String(text)) => serde_json::from_str(text).map(Some).map_err(|e| format!("source.json `{key}`: {e}")),
                Some(_) => Err(format!("source.json `{key}`: the schema's text, a string")),
            }
        };
        let state = schema("state")?.ok_or("source.json has no `state` schema")?;
        let (view, event) = (schema("view")?, schema("event")?);
        let iface = engine_profile::engine_interface(&state, view.as_ref(), event.as_ref())
            .map_err(|d| format!("source.json's schemas: {}", d.iter().map(|d| d.message.as_str()).collect::<Vec<_>>().join("; ")))?;
        let parsed = compiler::schema::parse_interface(&iface).map_err(|d| format!("the interface: {}", d.iter().map(|d| d.message.as_str()).collect::<Vec<_>>().join("; ")))?;
        let compiled = bytecode::Program::parse(program).map_err(|e| format!("engine.bin: {e:?}"))?;
        if compiled.interface_hash != parsed.hash {
            return Err("source.json's schemas are not the interface engine.bin was compiled against".into());
        }
        let Ty::Rec(fields, _) = &parsed.output else {
            return Err("the interface's output is not a record".into());
        };
        let state = fields.get("state").cloned().ok_or("the interface's output has no `state`")?;
        Ok(Decoder { state, program })
    }

    /// `leaves` (the state's, as `VmEngine::state` gives them) as plain JSON,
    /// a list's items read from `list` (`VmEngine::state_list`) by the id
    /// its leaf holds.
    pub fn state<'m>(&self, leaves: &[Value], list: &dyn Fn(u16) -> Option<SavedList<'m>>) -> Json {
        let mut it = leaves.iter();
        plain(&mut it, &self.state, list)
    }

    /// The source bytes (`engine.expr`) of the instruction at `pc`.
    pub fn span(&self, pc: u32) -> Option<(u32, u32)> {
        bytecode::Program::parse(self.program).ok()?.span(pc)
    }
}

/// Leaves of type `t` as plain JSON, as `compiler::plain` gives a run's
/// output: records as objects with their absent (nil) fields left out, ints
/// and floats as numbers, a list as an array of its items (read through
/// `list`; null if it can't be).
fn plain<'v, 'm>(leaves: &mut impl Iterator<Item = &'v Value<'v>>, t: &Ty, list: &dyn Fn(u16) -> Option<SavedList<'m>>) -> Json {
    match t {
        Ty::Rec(fields, _) => {
            let mut out = Map::new();
            for (k, ft) in fields {
                let v = plain(leaves, ft, list);
                if !v.is_null() {
                    out.insert(k.clone(), v);
                }
            }
            Json::Object(out)
        }
        Ty::Opt(inner) if matches!(**inner, Ty::Rec(..)) => {
            let present = !matches!(leaves.next(), None | Some(Value::Nil));
            let v = plain(leaves, inner, list);
            if present { v } else { Json::Null }
        }
        _ if matches!(unopt(t), Ty::List(..)) => {
            let Ty::List(item, _) = unopt(t) else { unreachable!() };
            let Some(Value::List(ListRef::Input(id))) = leaves.next() else { return Json::Null };
            let Some(saved) = list(*id) else { return Json::Null };
            let n = ListInput::len(&saved) as usize;
            Json::Array((0..n).map(|i| plain(&mut saved.item(i).iter(), item, list)).collect())
        }
        _ => match leaves.next() {
            Some(Value::Bool(b)) => json!(b),
            Some(Value::Int(i)) => json!(i),
            Some(Value::Float(f)) => json!(f),
            Some(Value::Str(s)) => json!(String::from_utf8_lossy(s)),
            _ => Json::Null,
        },
    }
}
