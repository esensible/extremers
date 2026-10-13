//! Test-only adapter between bitwire's Sink/Source walk and JSON values in
//! the vector form of SPEC §8 (byte strings as hex, exact float bits in a
//! side map keyed by JSON pointer, integers as exact decimals).
//!
//! The library has no names and no value tree, so the adapter builds a
//! named shadow of the descriptor once (from `Descriptor::names`) and uses
//! it to follow the walk:
//!
//! - decode: a `Recorder` sink logs the calls; `build` then replays them
//!   along the shadow into a `V`, checking that every call is the one the
//!   shadow expects at that point;
//! - encode: `script` turns a JSON value into the answers a Source gives,
//!   in walk order, stopping at the first value it cannot give (a `shape`
//!   or `range` it detects itself); `Scripted` hands them out and panics if
//!   the walk asks a different question.
#![allow(dead_code)]

use bitwire::{Descriptor, Error, Names, Node, Op, Sink, Source};
use serde_json::{Map, Value as Json};
use std::collections::VecDeque;

// ---- the named shadow of a descriptor ---------------------------------------

#[derive(Debug)]
pub enum Shadow {
    Leaf(Op),
    Enum(Vec<String>),
    Object(Vec<Field>),
    Array(Box<Shadow>),
    Union { tag: String, values: Vec<String>, branches: Vec<Vec<Field>> },
    Variants { fields: Vec<Field>, variants: Vec<(String, Shadow)> },
}

#[derive(Debug)]
pub struct Field {
    pub name: String,
    pub optional: bool,
    pub node: Shadow,
}

pub fn shadow(desc: &Descriptor<'_>) -> Shadow {
    let mut names = desc.names();
    let s = build_shadow(desc.root(), &mut names);
    assert!(names.next().is_none(), "names left over");
    s
}

fn take(names: &mut Names<'_>, n: usize) -> Vec<String> {
    (0..n).map(|_| names.next().expect("name").to_string()).collect()
}

fn build_shadow(n: Node<'_>, names: &mut Names<'_>) -> Shadow {
    match n.op() {
        Op::Enum => Shadow::Enum(take(names, n.len())),
        Op::Object => Shadow::Object(shadow_fields(n, names)),
        Op::Array => Shadow::Array(Box::new(build_shadow(n.item(), names))),
        Op::Union => {
            let tag = names.next().expect("tag").to_string();
            let values = take(names, n.len());
            let branches = n.children().map(|b| shadow_fields(b, names)).collect();
            Shadow::Union { tag, values, branches }
        }
        Op::Variants => {
            let fnames = take(names, n.len());
            let vnames = take(names, n.variant_count());
            let fields = n
                .children()
                .zip(fnames)
                .map(|(f, name)| Field { name, optional: f.is_optional(), node: build_shadow(f, names) })
                .collect();
            let variants = n.variants().zip(vnames).map(|(v, name)| (name, build_shadow(v, names))).collect();
            Shadow::Variants { fields, variants }
        }
        op => Shadow::Leaf(op),
    }
}

fn shadow_fields(obj: Node<'_>, names: &mut Names<'_>) -> Vec<Field> {
    let fnames = take(names, obj.len());
    obj.children()
        .zip(fnames)
        .map(|(f, name)| Field { name, optional: f.is_optional(), node: build_shadow(f, names) })
        .collect()
}

// ---- comparable values -------------------------------------------------------

/// A value as decoded: floats compared by bits (any NaN equals any NaN),
/// object keys in order.
#[derive(Debug, Clone)]
pub enum V {
    Bool(bool),
    Int(i128),
    F(f64),
    Str(String),
    Bytes(Vec<u8>),
    Arr(Vec<V>),
    Obj(Vec<(String, V)>),
}

impl PartialEq for V {
    fn eq(&self, o: &V) -> bool {
        match (self, o) {
            (V::Bool(a), V::Bool(b)) => a == b,
            (V::Int(a), V::Int(b)) => a == b,
            (V::F(a), V::F(b)) => (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits(),
            (V::Str(a), V::Str(b)) => a == b,
            (V::Bytes(a), V::Bytes(b)) => a == b,
            (V::Arr(a), V::Arr(b)) => a == b,
            (V::Obj(a), V::Obj(b)) => a == b,
            _ => false,
        }
    }
}

pub type F64Map = Map<String, Json>;

fn ptr(p: &str, k: &str) -> String {
    format!("{}/{}", p, k.replace('~', "~0").replace('/', "~1"))
}

fn map_f64(map: &F64Map, p: &str) -> Option<f64> {
    let h = map.get(p)?.as_str().expect("f64 hex");
    Some(f64::from_bits(u64::from_str_radix(h, 16).expect("f64 hex")))
}

pub fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0, "odd hex");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex")).collect()
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn json_int(j: &Json) -> i128 {
    if let Some(u) = j.as_u64() {
        u as i128
    } else if let Some(i) = j.as_i64() {
        i as i128
    } else {
        panic!("expected an exact integer, got {j}")
    }
}

/// The expected value of a vector (`value`/`decoded` with its f64 map).
pub fn expected(sh: &Shadow, j: &Json, map: &F64Map) -> V {
    exp(sh, j, map, "")
}

fn exp(sh: &Shadow, j: &Json, map: &F64Map, p: &str) -> V {
    match sh {
        Shadow::Leaf(op) => match op {
            Op::Bool => V::Bool(j.as_bool().expect("bool")),
            Op::F32 | Op::F64 => V::F(map_f64(map, p).unwrap_or_else(|| j.as_f64().expect("number"))),
            Op::String => V::Str(j.as_str().expect("string").to_string()),
            Op::Bytes => V::Bytes(unhex(j.as_str().expect("hex"))),
            _ => V::Int(json_int(j)),
        },
        Shadow::Enum(_) => V::Str(j.as_str().expect("enum string").to_string()),
        Shadow::Object(fields) => V::Obj(exp_fields(fields, j.as_object().expect("object"), map, p, 0)),
        Shadow::Array(item) => V::Arr(
            j.as_array().expect("array").iter().enumerate().map(|(i, x)| exp(item, x, map, &ptr(p, &i.to_string()))).collect(),
        ),
        Shadow::Union { tag, values, branches } => {
            let o = j.as_object().expect("union object");
            let t = o.get(tag).and_then(Json::as_str).expect("tag");
            let i = values.iter().position(|v| v == t).expect("known tag");
            let mut out = vec![(tag.clone(), V::Str(t.to_string()))];
            let (first, _) = o.iter().next().expect("tag first");
            assert_eq!(first, tag, "decoded union lists its tag first");
            out.extend(exp_fields(&branches[i], o, map, p, 1));
            V::Obj(out)
        }
        Shadow::Variants { fields, variants } => {
            let o = j.as_object().expect("record object");
            V::Obj(
                o.iter()
                    .map(|(k, x)| {
                        let at = ptr(p, k);
                        if let Some(f) = fields.iter().find(|f| &f.name == k) {
                            (k.clone(), exp(&f.node, x, map, &at))
                        } else if let Some((_, v)) = variants.iter().find(|(name, _)| name == k) {
                            (k.clone(), exp(v, x, map, &at))
                        } else {
                            panic!("undeclared key {k} in an expected value")
                        }
                    })
                    .collect(),
            )
        }
    }
}

fn exp_fields(fields: &[Field], o: &Map<String, Json>, map: &F64Map, p: &str, skip: usize) -> Vec<(String, V)> {
    o.iter()
        .skip(skip)
        .map(|(k, x)| {
            let f = fields.iter().find(|f| &f.name == k).unwrap_or_else(|| panic!("undeclared key {k} in an expected value"));
            (k.clone(), exp(&f.node, x, map, &ptr(p, k)))
        })
        .collect()
}

// ---- decode: record, then build ---------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Ev {
    Bool(bool),
    Uint(Op, u64),
    Int(Op, i64),
    F32(u32),
    F64(u64),
    Str(String),
    Bytes(Vec<u8>),
    Enum(u32),
    EnterObject,
    Field(u8, bool, bool),
    EnterArray(u32),
    Variant(u32),
    Leave,
}

#[derive(Default)]
pub struct Recorder(pub Vec<Ev>);

impl Sink for Recorder {
    type Error = Error;
    fn bool(&mut self, v: bool) -> Result<(), Error> {
        self.0.push(Ev::Bool(v));
        Ok(())
    }
    fn uint(&mut self, op: Op, v: u64) -> Result<(), Error> {
        self.0.push(Ev::Uint(op, v));
        Ok(())
    }
    fn int(&mut self, op: Op, v: i64) -> Result<(), Error> {
        self.0.push(Ev::Int(op, v));
        Ok(())
    }
    fn f32(&mut self, v: f32) -> Result<(), Error> {
        self.0.push(Ev::F32(v.to_bits()));
        Ok(())
    }
    fn f64(&mut self, v: f64) -> Result<(), Error> {
        self.0.push(Ev::F64(v.to_bits()));
        Ok(())
    }
    fn string(&mut self, v: &str) -> Result<(), Error> {
        self.0.push(Ev::Str(v.to_string()));
        Ok(())
    }
    fn bytes(&mut self, v: &[u8]) -> Result<(), Error> {
        self.0.push(Ev::Bytes(v.to_vec()));
        Ok(())
    }
    fn enum_index(&mut self, i: u8) -> Result<(), Error> {
        self.0.push(Ev::Enum(i as u32));
        Ok(())
    }
    fn enter_object(&mut self) -> Result<(), Error> {
        self.0.push(Ev::EnterObject);
        Ok(())
    }
    fn field(&mut self, index: u8, optional: bool, present: bool) -> Result<(), Error> {
        self.0.push(Ev::Field(index, optional, present));
        Ok(())
    }
    fn enter_array(&mut self, len: u32) -> Result<(), Error> {
        self.0.push(Ev::EnterArray(len));
        Ok(())
    }
    fn variant(&mut self, i: u8) -> Result<(), Error> {
        self.0.push(Ev::Variant(i as u32));
        Ok(())
    }
    fn leave(&mut self) -> Result<(), Error> {
        self.0.push(Ev::Leave);
        Ok(())
    }
}

/// Decodes `msg` into a value, checking the call sequence against the shadow.
pub fn decode(desc: &Descriptor<'_>, sh: &Shadow, msg: &[u8]) -> Result<V, Error> {
    let mut rec = Recorder::default();
    bitwire::decode(desc, msg, &mut rec)?;
    let mut evs: VecDeque<Ev> = rec.0.into();
    let v = build(sh, &mut evs);
    assert!(evs.is_empty(), "calls left over: {evs:?}");
    Ok(v)
}

fn next(evs: &mut VecDeque<Ev>) -> Ev {
    evs.pop_front().expect("sink call")
}

fn build(sh: &Shadow, evs: &mut VecDeque<Ev>) -> V {
    match sh {
        Shadow::Leaf(op) => match (op, next(evs)) {
            (Op::Bool, Ev::Bool(b)) => V::Bool(b),
            (Op::F32, Ev::F32(b)) => V::F(f32::from_bits(b) as f64),
            (Op::F64, Ev::F64(b)) => V::F(f64::from_bits(b)),
            (Op::String, Ev::Str(s)) => V::Str(s),
            (Op::Bytes, Ev::Bytes(b)) => V::Bytes(b),
            (op, Ev::Uint(o, v)) if *op == o && matches!(o, Op::U8 | Op::U16 | Op::U32 | Op::U64) => {
                assert!(v <= o.uint_max());
                V::Int(v as i128)
            }
            (op, Ev::Int(o, v)) if *op == o && matches!(o, Op::I8 | Op::I16 | Op::I32 | Op::I64) => {
                let (lo, hi) = o.int_range();
                assert!(v >= lo && v <= hi);
                V::Int(v as i128)
            }
            (op, e) => panic!("{op:?} got sink call {e:?}"),
        },
        Shadow::Enum(values) => match next(evs) {
            Ev::Enum(i) => V::Str(values[i as usize].clone()),
            e => panic!("enum got {e:?}"),
        },
        Shadow::Object(fields) => {
            assert_eq!(next(evs), Ev::EnterObject);
            let o = build_fields(fields, evs);
            assert_eq!(next(evs), Ev::Leave);
            V::Obj(o)
        }
        Shadow::Array(item) => {
            let Ev::EnterArray(len) = next(evs) else { panic!("array") };
            let a = (0..len).map(|_| build(item, evs)).collect();
            assert_eq!(next(evs), Ev::Leave);
            V::Arr(a)
        }
        Shadow::Union { tag, values, branches } => {
            let Ev::Variant(i) = next(evs) else { panic!("union") };
            let mut o = vec![(tag.clone(), V::Str(values[i as usize].clone()))];
            o.extend(build_fields(&branches[i as usize], evs));
            assert_eq!(next(evs), Ev::Leave);
            V::Obj(o)
        }
        Shadow::Variants { fields, variants } => {
            assert_eq!(next(evs), Ev::EnterObject);
            let mut o = build_fields(fields, evs);
            let Ev::Variant(i) = next(evs) else { panic!("record variant") };
            let (name, v) = &variants[i as usize];
            o.push((name.clone(), build(v, evs)));
            assert_eq!(next(evs), Ev::Leave);
            V::Obj(o)
        }
    }
}

fn build_fields(fields: &[Field], evs: &mut VecDeque<Ev>) -> Vec<(String, V)> {
    let mut o = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        match next(evs) {
            Ev::Field(idx, opt, present) => {
                assert_eq!((idx as usize, opt), (i, f.optional), "field call");
                assert!(present || opt, "required field reported absent");
                if present {
                    o.push((f.name.clone(), build(&f.node, evs)));
                }
            }
            e => panic!("field {i} got {e:?}"),
        }
    }
    o
}

// ---- encode: script the answers, then hand them out --------------------------

/// The answers a Source gives for `j`, in walk order. Stops (with the error
/// as the last answer) at a value it cannot give: wrong JSON type (`shape`),
/// a non-whole number or one no 64-bit integer holds (`range`). Names that
/// are not enum values or union tags become index `u32::MAX`, so that the
/// walk itself refuses them.
pub fn script(sh: &Shadow, j: &Json, map: &F64Map) -> VecDeque<Result<Ev, Error>> {
    let mut out = Vec::new();
    if let Err(e) = sc(sh, j, map, "", &mut out) {
        out.push(Err(e));
    }
    out.into()
}

fn whole(f: f64) -> bool {
    f.is_finite() && f.trunc() == f
}

fn sc(sh: &Shadow, j: &Json, map: &F64Map, p: &str, out: &mut Vec<Result<Ev, Error>>) -> Result<(), Error> {
    match sh {
        Shadow::Leaf(op) => {
            let ev = match op {
                Op::Bool => Ev::Bool(j.as_bool().ok_or(Error::Shape)?),
                Op::F32 | Op::F64 => {
                    let f = match map_f64(map, p) {
                        Some(f) => f,
                        None => j.as_f64().ok_or(Error::Shape)?,
                    };
                    if *op == Op::F32 { Ev::F32((f as f32).to_bits()) } else { Ev::F64(f.to_bits()) }
                }
                Op::String => Ev::Str(j.as_str().ok_or(Error::Shape)?.to_string()),
                Op::Bytes => Ev::Bytes(unhex(j.as_str().ok_or(Error::Shape)?)),
                Op::U8 | Op::U16 | Op::U32 | Op::U64 => {
                    if !j.is_number() {
                        return Err(Error::Shape);
                    }
                    let v = if let Some(u) = j.as_u64() {
                        u
                    } else if j.as_i64().is_some() {
                        return Err(Error::Range); // negative
                    } else {
                        let f = j.as_f64().unwrap();
                        if !whole(f) || f < 0.0 || f >= 18446744073709551616.0 {
                            return Err(Error::Range);
                        }
                        f as u64
                    };
                    Ev::Uint(*op, v)
                }
                _ => {
                    if !j.is_number() {
                        return Err(Error::Shape);
                    }
                    let v = if let Some(i) = j.as_i64() {
                        i
                    } else if j.as_u64().is_some() {
                        return Err(Error::Range); // >= 2^63
                    } else {
                        let f = j.as_f64().unwrap();
                        if !whole(f) || f < -9223372036854775808.0 || f >= 9223372036854775808.0 {
                            return Err(Error::Range);
                        }
                        f as i64
                    };
                    Ev::Int(*op, v)
                }
            };
            out.push(Ok(ev));
        }
        Shadow::Enum(values) => {
            let s = j.as_str().ok_or(Error::Shape)?;
            let i = values.iter().position(|v| v == s).map_or(u32::MAX, |i| i as u32);
            out.push(Ok(Ev::Enum(i)));
        }
        Shadow::Object(fields) => {
            let o = j.as_object().ok_or(Error::Shape)?;
            out.push(Ok(Ev::EnterObject));
            sc_fields(fields, o, map, p, out)?;
            out.push(Ok(Ev::Leave));
        }
        Shadow::Array(item) => {
            let a = j.as_array().ok_or(Error::Shape)?;
            out.push(Ok(Ev::EnterArray(a.len() as u32)));
            for (i, x) in a.iter().enumerate() {
                sc(item, x, map, &ptr(p, &i.to_string()), out)?;
            }
            out.push(Ok(Ev::Leave));
        }
        Shadow::Union { tag, values, branches } => {
            let o = j.as_object().ok_or(Error::Shape)?;
            let t = o.get(tag).and_then(Json::as_str).ok_or(Error::Shape)?;
            let Some(i) = values.iter().position(|v| v == t) else {
                out.push(Ok(Ev::Variant(u32::MAX)));
                return Ok(());
            };
            out.push(Ok(Ev::Variant(i as u32)));
            sc_fields(&branches[i], o, map, p, out)?;
            out.push(Ok(Ev::Leave));
        }
        Shadow::Variants { fields, variants } => {
            let o = j.as_object().ok_or(Error::Shape)?;
            out.push(Ok(Ev::EnterObject));
            sc_fields(fields, o, map, p, out)?;
            // exactly one variant key (an explicit null counts as present)
            let mut present = variants.iter().enumerate().filter(|(_, (name, _))| o.contains_key(name));
            let (i, (name, v)) = present.next().ok_or(Error::Shape)?;
            if present.next().is_some() {
                return Err(Error::Shape);
            }
            out.push(Ok(Ev::Variant(i as u32)));
            sc(v, &o[name], map, &ptr(p, name), out)?;
            out.push(Ok(Ev::Leave));
        }
    }
    Ok(())
}

fn sc_fields(fields: &[Field], o: &Map<String, Json>, map: &F64Map, p: &str, out: &mut Vec<Result<Ev, Error>>) -> Result<(), Error> {
    for (i, f) in fields.iter().enumerate() {
        match o.get(&f.name) {
            Some(x) => {
                out.push(Ok(Ev::Field(i as u8, f.optional, true)));
                sc(&f.node, x, map, &ptr(p, &f.name), out)?;
            }
            None => out.push(Ok(Ev::Field(i as u8, f.optional, false))),
        }
    }
    Ok(())
}

/// A Source that hands out scripted answers, checking each question.
pub struct Scripted {
    answers: VecDeque<Result<Ev, Error>>,
    cur_str: String,
    cur_bytes: Vec<u8>,
}

impl Scripted {
    pub fn new(answers: VecDeque<Result<Ev, Error>>) -> Self {
        Scripted { answers, cur_str: String::new(), cur_bytes: Vec::new() }
    }

    fn next(&mut self, what: &str) -> Result<Ev, Error> {
        match self.answers.pop_front() {
            Some(a) => a,
            None => panic!("source asked for {what} after the last answer"),
        }
    }

    pub fn left(&self) -> usize {
        self.answers.len()
    }
}

impl Source for Scripted {
    type Error = Error;
    fn bool(&mut self) -> Result<bool, Error> {
        match self.next("bool")? {
            Ev::Bool(b) => Ok(b),
            e => panic!("asked bool, have {e:?}"),
        }
    }
    fn uint(&mut self, op: Op) -> Result<u64, Error> {
        match self.next("uint")? {
            Ev::Uint(o, v) if o == op => Ok(v),
            e => panic!("asked uint {op:?}, have {e:?}"),
        }
    }
    fn int(&mut self, op: Op) -> Result<i64, Error> {
        match self.next("int")? {
            Ev::Int(o, v) if o == op => Ok(v),
            e => panic!("asked int {op:?}, have {e:?}"),
        }
    }
    fn f32(&mut self) -> Result<f32, Error> {
        match self.next("f32")? {
            Ev::F32(b) => Ok(f32::from_bits(b)),
            e => panic!("asked f32, have {e:?}"),
        }
    }
    fn f64(&mut self) -> Result<f64, Error> {
        match self.next("f64")? {
            Ev::F64(b) => Ok(f64::from_bits(b)),
            e => panic!("asked f64, have {e:?}"),
        }
    }
    fn string(&mut self) -> Result<&str, Error> {
        match self.next("string")? {
            Ev::Str(s) => {
                self.cur_str = s;
                Ok(&self.cur_str)
            }
            e => panic!("asked string, have {e:?}"),
        }
    }
    fn bytes(&mut self) -> Result<&[u8], Error> {
        match self.next("bytes")? {
            Ev::Bytes(b) => {
                self.cur_bytes = b;
                Ok(&self.cur_bytes)
            }
            e => panic!("asked bytes, have {e:?}"),
        }
    }
    fn enum_index(&mut self) -> Result<u32, Error> {
        match self.next("enum")? {
            Ev::Enum(i) => Ok(i),
            e => panic!("asked enum, have {e:?}"),
        }
    }
    fn enter_object(&mut self) -> Result<(), Error> {
        match self.next("object")? {
            Ev::EnterObject => Ok(()),
            e => panic!("asked object, have {e:?}"),
        }
    }
    fn field(&mut self, index: u8, optional: bool) -> Result<bool, Error> {
        match self.next("field")? {
            Ev::Field(i, o, present) if i == index && o == optional => Ok(present),
            e => panic!("asked field {index} ({optional}), have {e:?}"),
        }
    }
    fn enter_array(&mut self) -> Result<u32, Error> {
        match self.next("array")? {
            Ev::EnterArray(n) => Ok(n),
            e => panic!("asked array, have {e:?}"),
        }
    }
    fn variant(&mut self) -> Result<u32, Error> {
        match self.next("variant")? {
            Ev::Variant(i) => Ok(i),
            e => panic!("asked variant, have {e:?}"),
        }
    }
    fn leave(&mut self) -> Result<(), Error> {
        match self.next("leave")? {
            Ev::Leave => Ok(()),
            e => panic!("asked leave, have {e:?}"),
        }
    }
}

/// Encodes a vector value; the scripted answers must all be used on success.
pub fn encode(desc: &Descriptor<'_>, sh: &Shadow, j: &Json, map: &F64Map) -> Result<Vec<u8>, Error> {
    let mut src = Scripted::new(script(sh, j, map));
    let mut out = vec![0u8; 1 << 16];
    let n = bitwire::encode(desc, &mut src, &mut out)?;
    assert_eq!(src.left(), 0, "answers left over");
    out.truncate(n);
    Ok(out)
}
