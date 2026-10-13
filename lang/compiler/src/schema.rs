//! The interface (spec §1.1): named inputs and an output, each typed by a
//! schema in the wire subset (`wire/SPEC.md` §2), plus the language's own
//! requirements (§3.2).
//!
//! This build handles integer, number, boolean, string, enum, object and
//! array schemas, and `oneOf` as a record with variants.

use crate::diag::Diag;
use crate::types::{Range, StrTy, Ty, leaves};
use serde_json::{Map, Value as Json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Input {
    pub name: String,
    /// The input's type (`T?` when it's declared nullable).
    pub ty: Ty,
    /// Its first leaf among all the inputs' leaves.
    pub base: u16,
}

/// A host builtin's declaration (spec §2.4).
#[derive(Clone, Debug)]
pub struct HostDecl {
    pub name: String,
    /// (name, type, optional). An optional parameter's type is `T?`, and
    /// leaving it out passes nil.
    pub params: Vec<(String, Ty, bool)>,
    pub result: Ty,
    /// How the program names it to the host, e.g.
    /// `distance({lat:f,lon:f},{lat:f,lon:f})f`.
    pub signature: String,
}

#[derive(Clone, Debug)]
pub struct Interface {
    pub inputs: Vec<Input>,
    pub output: Ty,
    pub builtins: Vec<HostDecl>,
    /// The lists inside the inputs, in leaf order; the index is the list's id.
    pub lists: Vec<InputList>,
    /// The first 8 bytes of SHA-256 over the interface's canonical JSON.
    pub hash: [u8; 8],
}

/// What the VM checks one leaf against (before string constants are placed).
#[derive(Clone, Debug, PartialEq)]
pub enum LeafCheck {
    Bool { nullable: bool },
    /// A list position in an input: read through the host's list accessor.
    List,
    Int { range: Range, nullable: bool },
    Float { nullable: bool },
    Str { max_len: u32, values: Option<BTreeSet<String>>, nullable: bool },
}

/// The checks for a value's leaves, in leaf order. Inside an optional
/// record every leaf is nullable (they're all nil when it's absent), and the
/// presence leaf is a nullable bool.
pub fn leaf_checks(t: &Ty, nullable: bool, out: &mut Vec<LeafCheck>) {
    match t {
        Ty::Bool => out.push(LeafCheck::Bool { nullable }),
        Ty::Int(r) => out.push(LeafCheck::Int { range: *r, nullable }),
        Ty::Float => out.push(LeafCheck::Float { nullable }),
        Ty::Str(s) => out.push(LeafCheck::Str { max_len: s.max, values: s.values.clone(), nullable }),
        Ty::List(..) => out.push(LeafCheck::List),
        Ty::Rec(f, _) => f.values().for_each(|ft| leaf_checks(ft, nullable, out)),
        Ty::Opt(inner) => match &**inner {
            Ty::Rec(f, _) => {
                out.push(LeafCheck::Bool { nullable: true });
                f.values().for_each(|ft| leaf_checks(ft, true, out));
            }
            other => leaf_checks(other, true, out),
        },
        Ty::Nil | Ty::Unknown | Ty::Never => unreachable!("schemas never produce these"),
    }
}

/// A variant group's check (spec §7.3): `members` are the presence leaves
/// of its variants, exactly one of which must be non-nil, unless `guard`
/// (the presence leaf of the optional record the group sits in) is nil.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupCheck {
    pub guard: Option<u16>,
    pub members: Vec<u16>,
}

/// The variant groups in a value of type `t` laid out from leaf `base`,
/// innermost first.
pub fn group_checks(t: &Ty, base: u16, guard: Option<u16>, out: &mut Vec<GroupCheck>) {
    match t {
        Ty::Rec(f, group) => {
            let mut at = base;
            let mut members = Vec::new();
            for (k, ft) in f {
                if group.as_ref().is_some_and(|g| g.contains(k)) {
                    members.push(at);
                }
                group_checks(ft, at, guard, out);
                at += leaves(ft);
            }
            if group.is_some() {
                out.push(GroupCheck { guard, members });
            }
        }
        Ty::Opt(inner) if matches!(**inner, Ty::Rec(..)) => group_checks(inner, base + 1, Some(base), out),
        _ => {}
    }
}

/// A list inside an input (the input itself, or a field of an input record):
/// the host gives each one an accessor, by index.
#[derive(Clone, Debug)]
pub struct InputList {
    /// Which input, and the field path to the list within it.
    pub input: usize,
    pub path: Vec<String>,
    /// Its leaf among all the inputs' leaves (where its handle is read).
    pub leaf: u16,
    pub item: Ty,
    pub max: u32,
}

fn find_lists(input: usize, t: &Ty, leaf: u16, path: &mut Vec<String>, out: &mut Vec<InputList>) {
    match t {
        Ty::List(item, max) => out.push(InputList { input, path: path.clone(), leaf, item: (**item).clone(), max: *max }),
        Ty::Rec(f, _) => {
            let mut at = leaf;
            for (k, ft) in f {
                path.push(k.clone());
                find_lists(input, ft, at, path, out);
                path.pop();
                at += leaves(ft);
            }
        }
        Ty::Opt(inner) => match &**inner {
            Ty::Rec(..) => find_lists(input, inner, leaf + 1, path, out),
            other => find_lists(input, other, leaf, path, out),
        },
        _ => {}
    }
}

/// Annotations and validation keywords the wire ignores (`wire/SPEC.md` §2.1).
/// The language reads `minimum`, `maximum`, `exclusiveMinimum`,
/// `exclusiveMaximum` and `maxLength`.
const IGNORED: &[&str] = &[
    "title", "description", "$comment", "examples", "default", "deprecated", "readOnly", "writeOnly", "minimum",
    "maximum", "exclusiveMinimum", "exclusiveMaximum", "multipleOf", "minLength", "maxLength", "pattern",
    "contentMediaType", "minItems", "maxItems", "uniqueItems", "contains", "minContains", "maxContains",
    "minProperties", "maxProperties", "dependentRequired", "propertyNames", "additionalProperties",
    "unevaluatedProperties",
];

/// The longest string a layout can hold, in bytes, and so the largest `maxLength` (spec §3.2).
const MAX_STRING_BYTES: u32 = 65_535;

pub fn parse_interface(json: &Json) -> Result<Interface, Vec<Diag>> {
    let mut diags = Vec::new();
    let obj = match json.as_object() {
        Some(o) => o,
        None => return Err(vec![Diag::interface("E-SCHEMA", "the interface must be a JSON object")]),
    };
    for k in obj.keys() {
        if !matches!(k.as_str(), "inputs" | "output" | "builtins") {
            diags.push(Diag::interface("E-SCHEMA", format!("unknown interface key `{k}`")).help("the keys are `inputs`, `output` and `builtins`"));
        }
    }
    let mut inputs = Vec::new();
    let mut seen = BTreeSet::new();
    let mut base: u32 = 0;
    for (i, inp) in obj.get("inputs").and_then(Json::as_array).map(Vec::as_slice).unwrap_or(&[]).iter().enumerate() {
        let path = format!("/inputs/{i}");
        let Some(name) = inp.get("name").and_then(Json::as_str) else {
            diags.push(Diag::interface("E-SCHEMA", format!("{path}: an input needs a `name`")));
            continue;
        };
        if !is_identifier(name) {
            diags.push(Diag::interface("E-SCHEMA", format!("{path}: `{name}` isn't a valid name")).help("use letters, digits and `_`, not starting with a digit"));
        }
        if !seen.insert(name.to_string()) {
            diags.push(Diag::interface("E-SCHEMA", format!("{path}: the input `{name}` is declared twice")));
        }
        let nullable = match inp.get("nullable") {
            None => false,
            Some(Json::Bool(b)) => *b,
            Some(_) => {
                diags.push(Diag::interface("E-SCHEMA", format!("{path}/nullable: must be true or false")));
                false
            }
        };
        match inp.get("schema") {
            None => diags.push(Diag::interface("E-SCHEMA", format!("{path}: an input needs a `schema`"))),
            Some(s) => match schema(&format!("{path}/schema"), s) {
                Ok(ty) => {
                    let ty = if nullable { ty.nullable() } else { ty };
                    let n = leaves(&ty) as u32;
                    inputs.push(Input { name: name.to_string(), ty, base: base as u16 });
                    base += n;
                }
                Err(d) => diags.push(d),
            },
        }
    }
    if base > u16::MAX as u32 {
        diags.push(Diag::interface("E-SCHEMA", format!("the inputs have {base} leaves; the most is {}", u16::MAX)));
    }
    let output = match obj.get("output") {
        None => {
            diags.push(Diag::interface("E-SCHEMA", "the interface needs an `output` schema"));
            None
        }
        Some(s) => match schema("/output", s) {
            Ok(ty) => Some(ty),
            Err(d) => {
                diags.push(d);
                None
            }
        },
    };
    let mut builtins: Vec<HostDecl> = Vec::new();
    for (i, b) in obj.get("builtins").and_then(Json::as_array).map(Vec::as_slice).unwrap_or(&[]).iter().enumerate() {
        match builtin(&format!("/builtins/{i}"), b) {
            Ok(d) => {
                if builtins.iter().any(|o| o.name == d.name) {
                    diags.push(Diag::interface("E-SCHEMA", format!("/builtins/{i}: the builtin `{}` is declared twice", d.name)));
                } else if seen.contains(&d.name) {
                    diags.push(Diag::interface("E-SCHEMA", format!("/builtins/{i}: `{}` is also an input's name", d.name)));
                }
                builtins.push(d);
            }
            Err(d) => diags.push(d),
        }
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    // serde_json's maps are ordered by key, so this serialisation is canonical.
    let canonical = serde_json::to_string(json).expect("serialisable");
    let digest = Sha256::digest(canonical.as_bytes());
    let mut hash = [0u8; 8];
    hash.copy_from_slice(&digest[..8]);
    let mut lists = Vec::new();
    for (i, inp) in inputs.iter().enumerate() {
        find_lists(i, &inp.ty, inp.base, &mut Vec::new(), &mut lists);
    }
    Ok(Interface { inputs, output: output.unwrap(), builtins, lists, hash })
}

/// expr's own builtins, which a host builtin can't be named after.
pub const EXPR_BUILTINS: &[&str] = &[
    "all", "none", "any", "one", "filter", "map", "find", "findIndex", "findLast", "findLastIndex", "count", "sum",
    "groupBy", "sortBy", "reduce", "len", "abs", "ceil", "floor", "round", "int", "float", "string", "trim",
    "trimPrefix", "trimSuffix", "upper", "lower", "split", "splitAfter", "replace", "repeat", "join", "indexOf",
    "lastIndexOf", "hasPrefix", "hasSuffix", "max", "min", "mean", "median", "toJSON", "fromJSON", "toBase64",
    "fromBase64", "now", "duration", "date", "timezone", "first", "last", "get", "take", "keys", "values", "toPairs",
    "fromPairs", "sort", "bitand", "bitor", "bitxor", "bitnand", "bitnot", "bitshl", "bitshr", "bitushr", "type",
    "concat", "flatten", "uniq", "reverse",
];

/// `{"name": …, "params": [{"name", "schema", "optional"?}], "result": schema, "nullable"?: bool}`
fn builtin(path: &str, b: &Json) -> Result<HostDecl, Diag> {
    let err = |msg: String| Diag::interface("E-SCHEMA", format!("{path}: {msg}"));
    let o = b.as_object().ok_or_else(|| err("a builtin is a JSON object".into()))?;
    for k in o.keys() {
        if !matches!(k.as_str(), "name" | "params" | "result" | "nullable" | "description") {
            return Err(err(format!("unknown key `{k}`")).help("a builtin has `name`, `params`, `result` and optionally `nullable`"));
        }
    }
    let name = o.get("name").and_then(Json::as_str).ok_or_else(|| err("a builtin needs a `name`".into()))?;
    if !is_identifier(name) {
        return Err(err(format!("`{name}` isn't a valid name")));
    }
    if EXPR_BUILTINS.contains(&name) {
        return Err(err(format!("`{name}` is one of expr's own builtins")));
    }
    let mut params = Vec::new();
    let mut seen_optional = false;
    for (i, p) in o.get("params").and_then(Json::as_array).map(Vec::as_slice).unwrap_or(&[]).iter().enumerate() {
        let pp = format!("{path}/params/{i}");
        let pname = p.get("name").and_then(Json::as_str).ok_or_else(|| Diag::interface("E-SCHEMA", format!("{pp}: a parameter needs a `name`")))?;
        let optional = p.get("optional").and_then(Json::as_bool).unwrap_or(false);
        if seen_optional && !optional {
            return Err(Diag::interface("E-SCHEMA", format!("{pp}: optional parameters must come last")));
        }
        seen_optional |= optional;
        let s = p.get("schema").ok_or_else(|| Diag::interface("E-SCHEMA", format!("{pp}: a parameter needs a `schema`")))?;
        let t = schema(&format!("{pp}/schema"), s)?;
        if contains_list(&t) {
            return Err(Diag::interface("E-SCHEMA", format!("{pp}: builtins that take lists aren't supported yet")));
        }
        if contains_group(&t) {
            return Err(Diag::interface("E-SCHEMA", format!("{pp}: builtins that take variants (`oneOf`) aren't supported yet")));
        }
        params.push((pname.to_string(), if optional { t.nullable() } else { t }, optional));
    }
    let result = schema(&format!("{path}/result"), o.get("result").ok_or_else(|| err("a builtin needs a `result` schema".into()))?)?;
    if contains_group(&result) {
        return Err(err("builtins that give variants (`oneOf`) aren't supported yet".into()));
    }
    let result = if o.get("nullable").and_then(Json::as_bool).unwrap_or(false) { result.nullable() } else { result };
    let signature = format!(
        "{name}({}){}",
        params.iter().map(|(_, t, _)| signature(t)).collect::<Vec<_>>().join(","),
        signature(&result)
    );
    Ok(HostDecl { name: name.to_string(), params, result, signature })
}

/// A type's part of a signature: `f`, `i`, `b`, `s`, `{k:…,…}` (fields in
/// name order, which is also leaf order), `?` before a nullable type.
/// Ranges, enums and lengths aren't part of it: they don't change the leaves.
pub fn signature(t: &Ty) -> String {
    match t {
        Ty::Bool => "b".into(),
        Ty::Int(_) => "i".into(),
        Ty::Float => "f".into(),
        Ty::Str(_) => "s".into(),
        Ty::Rec(f, _) => format!("{{{}}}", f.iter().map(|(k, v)| format!("{k}:{}", signature(v))).collect::<Vec<_>>().join(",")),
        Ty::Opt(inner) => format!("?{}", signature(inner)),
        Ty::List(item, _) => format!("[{}]", signature(item)),
        Ty::Nil | Ty::Unknown | Ty::Never => unreachable!("schemas never produce these"),
    }
}

fn is_identifier(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_') && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// A schema's type.
fn schema(path: &str, s: &Json) -> Result<Ty, Diag> {
    let err = |msg: String| Diag::interface("E-SCHEMA", format!("{path}: {msg}"));
    let o = s.as_object().ok_or_else(|| err("a schema must be a JSON object".into()))?;
    for structural in ["$ref", "anyOf", "allOf", "not", "if", "nullable"] {
        if o.contains_key(structural) {
            return Err(match structural {
                "$ref" => err(format!("`{structural}` isn't implemented yet")),
                "nullable" => err("`nullable` belongs on the input, beside `schema`; inside a record, leave the field out of `required`".into()),
                _ => err(format!("`{structural}` isn't in the wire subset")),
            });
        }
    }
    if o.contains_key("oneOf") {
        return one_of(path, o);
    }
    if o.contains_key("enum") || o.contains_key("const") {
        return string_enum(path, o);
    }
    let ty = match o.get("type") {
        Some(Json::String(t)) => t.as_str(),
        Some(Json::Array(_)) => return Err(err("type lists aren't in the wire subset".into()).help("declare the input `nullable`, or leave a record field out of `required`")),
        _ => return Err(err("a schema needs a `type`".into())),
    };
    let allowed: &[&str] = match ty {
        "integer" | "number" | "string" => &["type", "format"],
        "boolean" => &["type"],
        "object" => &["type", "properties", "required"],
        "array" => &["type", "items"],
        other => return Err(err(format!("unknown type `{other}`"))),
    };
    for k in o.keys() {
        if !allowed.contains(&k.as_str()) && !IGNORED.contains(&k.as_str()) {
            return Err(err(format!("`{k}` isn't allowed on a `{ty}` schema")));
        }
    }
    match ty {
        "boolean" => Ok(Ty::Bool),
        "number" => {
            match o.get("format").map(|f| f.as_str()) {
                None | Some(Some("double" | "float")) => {}
                Some(f) => return Err(err(format!("unknown number format {:?}", f.unwrap_or("(not a string)"))).help("use \"double\" or \"float\"")),
            }
            Ok(Ty::Float)
        }
        "integer" => integer(path, o),
        "string" => {
            if o.contains_key("contentEncoding") {
                return Err(err("byte strings aren't supported".into()));
            }
            let Some(n) = o.get("maxLength") else {
                return Err(err("a string needs `maxLength`".into()).help("every string needs a maximum length, e.g. \"maxLength\": 32"));
            };
            let n = n.as_u64().ok_or_else(|| err("`maxLength` must be a whole number".into()))?;
            // A character is at most 4 bytes in UTF-8.
            let bytes = n.saturating_mul(4);
            if bytes > MAX_STRING_BYTES as u64 {
                return Err(err(format!("`maxLength` is at most {} characters", MAX_STRING_BYTES / 4)));
            }
            Ok(Ty::Str(StrTy { max: bytes as u32, chars: n as u32, values: None }))
        }
        "object" => Ok(Ty::Rec(record_fields(path, o)?, None)),
        "array" => {
            let Some(n) = o.get("maxItems") else {
                return Err(err("a list needs `maxItems`".into()).help("every list needs a maximum length, e.g. \"maxItems\": 100"));
            };
            let n = n.as_u64().ok_or_else(|| err("`maxItems` must be a whole number".into()))?;
            if n > u16::MAX as u64 {
                return Err(err(format!("`maxItems` is at most {}", u16::MAX)));
            }
            let items = o.get("items").ok_or_else(|| err("a list needs an `items` schema".into()))?;
            let item = schema(&format!("{path}/items"), items)?;
            if contains_list(&item) {
                return Err(err("lists inside list items aren't supported yet".into()));
            }
            if contains_group(&item) {
                return Err(err("variants (`oneOf`) inside list items aren't supported yet".into()));
            }
            Ok(Ty::List(Box::new(item), n as u32))
        }
        _ => unreachable!(),
    }
}

/// An object schema's `properties` and `required`, as record fields.
fn record_fields(path: &str, o: &Map<String, Json>) -> Result<BTreeMap<String, Ty>, Diag> {
    let err = |msg: String| Diag::interface("E-SCHEMA", format!("{path}: {msg}"));
    if matches!(o.get("additionalProperties"), Some(v) if v != &Json::Bool(false)) {
        return Err(err("`additionalProperties` must be absent or false: records have a fixed set of fields".into()));
    }
    let props = match o.get("properties") {
        None => &Map::new(),
        Some(Json::Object(p)) => p,
        Some(_) => return Err(err("`properties` must be an object".into())),
    };
    let required: BTreeSet<&str> = match o.get("required") {
        None => BTreeSet::new(),
        Some(Json::Array(r)) => r.iter().map(|v| v.as_str().ok_or_else(|| err("`required` lists field names".into()))).collect::<Result<_, _>>()?,
        Some(_) => return Err(err("`required` must be a list of field names".into())),
    };
    for r in &required {
        if !props.contains_key(*r) {
            return Err(err(format!("`required` names `{r}`, which isn't in `properties`")));
        }
    }
    let mut fields = BTreeMap::new();
    for (k, v) in props {
        if !is_identifier(k) {
            return Err(err(format!("the field name `{k}` can't be written in an expression")).help("use letters, digits and `_`, not starting with a digit"));
        }
        let t = schema(&format!("{path}/properties/{k}"), v)?;
        fields.insert(k.clone(), if required.contains(k.as_str()) { t } else { t.nullable() });
    }
    Ok(fields)
}

/// A record with variants (spec §3.2): `oneOf` alternatives, each
/// `{"properties": {"<name>": <object schema>}, "required": ["<name>"]}`,
/// optionally beside ordinary `properties` and `required`. Exactly one
/// variant is present; each is an optional record field named after it.
fn one_of(path: &str, o: &Map<String, Json>) -> Result<Ty, Diag> {
    let err = |msg: String| Diag::interface("E-SCHEMA", format!("{path}: {msg}"));
    const SHAPE: &str = "each alternative names one variant: {\"properties\": {\"<name>\": {\"type\": \"object\", …}}, \"required\": [\"<name>\"]}";
    match o.get("type") {
        None => {}
        Some(t) if t == "object" => {}
        Some(_) => return Err(err("a schema with `oneOf` is a record of variants: its `type`, if given, must be \"object\"".into()).help(SHAPE)),
    }
    for k in o.keys() {
        if !matches!(k.as_str(), "type" | "properties" | "required" | "oneOf") && !IGNORED.contains(&k.as_str()) {
            return Err(err(format!("`{k}` isn't allowed beside `oneOf`")));
        }
    }
    let mut fields = record_fields(path, o)?;
    let alts = match o.get("oneOf") {
        Some(Json::Array(a)) if !a.is_empty() && a.len() <= 256 => a,
        _ => return Err(err("`oneOf` must be a list of 1 to 256 alternatives".into()).help(SHAPE)),
    };
    let mut names = BTreeSet::new();
    for (i, alt) in alts.iter().enumerate() {
        let ap = format!("{path}/oneOf/{i}");
        let aerr = |msg: String| Diag::interface("E-SCHEMA", format!("{ap}: {msg}")).help(SHAPE);
        let Some(ao) = alt.as_object() else {
            return Err(aerr("an alternative must be a JSON object".into()));
        };
        match ao.get("type") {
            None => {}
            Some(t) if t == "object" => {}
            Some(t) => return Err(aerr(format!("an alternative of type {t} isn't supported: every alternative is a record that names its variant"))),
        }
        if ao.contains_key("enum") || ao.contains_key("const") || ao.contains_key("oneOf") {
            return Err(aerr("only record alternatives are supported: every alternative is a record that names its variant".into()));
        }
        for k in ao.keys() {
            if !matches!(k.as_str(), "type" | "properties" | "required") && !IGNORED.contains(&k.as_str()) {
                return Err(aerr(format!("`{k}` isn't allowed in a `oneOf` alternative")));
            }
        }
        let props = match ao.get("properties") {
            Some(Json::Object(p)) => p,
            _ => return Err(aerr("an alternative needs `properties` with one property, the variant's name".into())),
        };
        if props.len() != 1 {
            let tag = props.iter().find(|(_, s)| s.get("const").is_some() || s.get("enum").is_some()).map(|(k, _)| k);
            return Err(match tag {
                Some(t) => aerr(format!(
                    "this alternative looks like a union tagged by a property (`{t}`); variants are named by their alternative's one property instead"
                )),
                None => aerr(format!("an alternative has exactly one property, the variant's name, but this has {}", props.len())),
            });
        }
        let (name, vs) = props.iter().next().unwrap();
        let required: Vec<&str> = match ao.get("required") {
            Some(Json::Array(r)) => r.iter().filter_map(Json::as_str).collect(),
            _ => Vec::new(),
        };
        if required != [name.as_str()] {
            return Err(aerr(format!("the alternative must list its variant in `required`: \"required\": [\"{name}\"]")));
        }
        if !is_identifier(name) {
            return Err(aerr(format!("the variant name `{name}` can't be written in an expression")));
        }
        if names.contains(name) {
            return Err(aerr(format!("the variant `{name}` appears twice")));
        }
        if fields.contains_key(name) {
            return Err(aerr(format!("`{name}` is both a property and a variant")));
        }
        names.insert(name.clone());
        let t = schema(&format!("{ap}/properties/{name}"), vs)?;
        if !matches!(t, Ty::Rec(..)) {
            return Err(Diag::interface("E-SCHEMA", format!("{ap}/properties/{name}: the variant `{name}` must be an object schema (a record), but this is {}", t.describe()))
                .help("put the value in a record: {\"type\": \"object\", \"properties\": {\"value\": …}, \"required\": [\"value\"]}"));
        }
        fields.insert(name.clone(), t.nullable());
    }
    Ok(Ty::Rec(fields, Some(names)))
}

pub fn contains_list(t: &Ty) -> bool {
    match t {
        Ty::List(..) => true,
        Ty::Rec(f, _) => f.values().any(contains_list),
        Ty::Opt(inner) => contains_list(inner),
        _ => false,
    }
}

/// Whether a type has a variant group anywhere in it.
pub fn contains_group(t: &Ty) -> bool {
    match t {
        Ty::Rec(f, g) => g.is_some() || f.values().any(contains_group),
        Ty::Opt(inner) => contains_group(inner),
        Ty::List(item, _) => contains_group(item),
        _ => false,
    }
}

fn integer(path: &str, o: &Map<String, Json>) -> Result<Ty, Diag> {
    let err = |msg: String| Diag::interface("E-SCHEMA", format!("{path}: {msg}"));
    let width = match o.get("format").map(|f| f.as_str()) {
        None | Some(Some("int64")) => Range::FULL,
        Some(Some("int32")) => Range { lo: i32::MIN as i128, hi: i32::MAX as i128 },
        Some(Some("int16")) => Range { lo: i16::MIN as i128, hi: i16::MAX as i128 },
        Some(Some("int8")) => Range { lo: i8::MIN as i128, hi: i8::MAX as i128 },
        Some(Some("uint32")) => Range { lo: 0, hi: u32::MAX as i128 },
        Some(Some("uint16")) => Range { lo: 0, hi: u16::MAX as i128 },
        Some(Some("uint8")) => Range { lo: 0, hi: u8::MAX as i128 },
        Some(Some("uint64")) => return Err(err("`uint64` can exceed the largest integer the language has (int64)".into()).help("use `int64` or a narrower format")),
        Some(f) => return Err(err(format!("unknown integer format {:?}", f.unwrap_or("(not a string)")))),
    };
    let bound = |k: &str| -> Result<Option<i128>, Diag> {
        match o.get(k) {
            None => Ok(None),
            Some(v) => match (v.as_i64(), v.as_f64()) {
                (Some(i), _) => Ok(Some(i as i128)),
                (None, Some(f)) if f.fract() == 0.0 && f.abs() < 9.2e18 => Ok(Some(f as i128)),
                _ => Err(err(format!("`{k}` must be a whole number"))),
            },
        }
    };
    let mut r = width;
    if let Some(v) = bound("minimum")? {
        r.lo = r.lo.max(v);
    }
    if let Some(v) = bound("exclusiveMinimum")? {
        r.lo = r.lo.max(v + 1);
    }
    if let Some(v) = bound("maximum")? {
        r.hi = r.hi.min(v);
    }
    if let Some(v) = bound("exclusiveMaximum")? {
        r.hi = r.hi.min(v - 1);
    }
    if r.lo > r.hi {
        return Err(err(format!("no integer satisfies this schema (the range is empty: {}..{})", r.lo, r.hi)));
    }
    Ok(Ty::Int(r))
}

fn string_enum(path: &str, o: &Map<String, Json>) -> Result<Ty, Diag> {
    let err = |msg: String| Diag::interface("E-SCHEMA", format!("{path}: {msg}"));
    for k in o.keys() {
        if !matches!(k.as_str(), "enum" | "const" | "type") && !IGNORED.contains(&k.as_str()) {
            return Err(err(format!("`{k}` isn't allowed on an enum")));
        }
    }
    if let Some(t) = o.get("type") {
        if t != "string" {
            return Err(err("an enum's `type`, if given, must be \"string\"".into()));
        }
    }
    let values: Vec<&Json> = match (o.get("enum"), o.get("const")) {
        (Some(Json::Array(v)), None) => v.iter().collect(),
        (None, Some(c)) => vec![c],
        (Some(_), Some(_)) => return Err(err("use `enum` or `const`, not both".into())),
        _ => return Err(err("`enum` must be a list of strings".into())),
    };
    let mut set = BTreeSet::new();
    for v in values {
        let s = v.as_str().ok_or_else(|| err("enum values must be strings".into()))?;
        if !set.insert(s.to_string()) {
            return Err(err(format!("the enum value {s:?} appears twice")));
        }
    }
    if set.is_empty() || set.len() > 256 {
        return Err(err("an enum has 1 to 256 values".into()));
    }
    let max = set.iter().map(|s| s.len() as u32).max().unwrap();
    let chars = set.iter().map(|s| s.chars().count() as u32).max().unwrap();
    if max > MAX_STRING_BYTES {
        return Err(err("an enum value is too long".into()));
    }
    Ok(Ty::Str(StrTy { max, chars, values: Some(set) }))
}
