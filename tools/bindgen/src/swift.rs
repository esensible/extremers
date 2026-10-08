//! Swift from postcard-schema schemas, for the watch app (host protocol
//! only: the packages' own views are decoded by their JS bundles).
//!
//! Every named type reachable from the exports becomes a Swift type that is
//! `Equatable` and `PostcardCodable` (`init(from:)` reads it, `encode(to:)`
//! writes it); each export gets `decode<Name>(_:)` and `encode<Name>(_:)`.
//!
//! | Rust | Swift |
//! |---|---|
//! | struct | struct with every field, `var`, memberwise `init` |
//! | enum | enum: unit variant `case a`, newtype `case b(T)`, tuple `case c(T, U)`, struct variant `case d(x: T, y: U)` |
//! | `Option<T>` | `T?` |
//! | sequence, byte slice | `[T]` |
//! | `[T; N]` | `[T]` of exactly N (checked on both sides) |
//! | integers, floats, bool, `String` | `UInt8`…`UInt64`, `Int8`…`Int64`, `Float`, `Double`, `Bool`, `String` |
//!
//! Decoding throws on anything that is not exactly one value (short data,
//! unknown tags, bad UTF-8, varints out of range, trailing bytes); encoding
//! traps on a fixed-size array of the wrong length.

use std::fmt::Write;

use postcard_schema::schema::{DataModelType as T, DataModelVariant as V, NamedType, NamedValue};

use crate::js::Export;

fn is_named(ty: &T) -> bool {
    matches!(ty, T::Struct(_) | T::Enum(_) | T::TupleStruct(_) | T::NewtypeStruct(_) | T::UnitStruct)
}

fn ident(nt: &NamedType) -> &'static str {
    let name = nt.name;
    let end = name.find(['<', ' ']).unwrap_or(name.len());
    &name[..end]
}

fn collect(nt: &'static NamedType, order: &mut Vec<&'static NamedType>) {
    if is_named(nt.ty) {
        if let Some(seen) = order.iter().find(|n| ident(n) == ident(nt)) {
            assert!(*seen == nt, "two different types are both named {}", ident(nt));
            return;
        }
        order.push(nt);
    }
    match nt.ty {
        T::Option(inner) | T::NewtypeStruct(inner) | T::Seq(inner) => collect(inner, order),
        T::Tuple(items) | T::TupleStruct(items) => items.iter().for_each(|i| collect(i, order)),
        T::Struct(fields) => fields.iter().for_each(|f| collect(f.ty, order)),
        T::Enum(variants) => {
            for v in variants.iter() {
                match v.ty {
                    V::UnitVariant => {}
                    V::NewtypeVariant(inner) => collect(inner, order),
                    V::TupleVariant(items) => items.iter().for_each(|i| collect(i, order)),
                    V::StructVariant(fields) => fields.iter().for_each(|f| collect(f.ty, order)),
                }
            }
        }
        T::Map { .. } | T::I128 | T::U128 | T::Schema | T::Char => panic!("{}: not supported in Swift", nt.name),
        _ => {}
    }
}

/// (Swift type, reader expression on `r`, writer statement for `v` on `w`).
fn primitive(ty: &T) -> Option<(&'static str, &'static str, &'static str)> {
    Some(match ty {
        T::Bool => ("Bool", "try r.bool()", "w.bool"),
        T::U8 => ("UInt8", "try r.u8()", "w.u8"),
        T::I8 => ("Int8", "try r.i8()", "w.i8"),
        T::U16 => ("UInt16", "try r.u16()", "w.u16"),
        T::I16 => ("Int16", "try r.i16()", "w.i16"),
        T::U32 => ("UInt32", "try r.u32()", "w.u32"),
        T::I32 => ("Int32", "try r.i32()", "w.i32"),
        T::U64 | T::Usize => ("UInt64", "try r.u64()", "w.u64"),
        T::I64 | T::Isize => ("Int64", "try r.i64()", "w.i64"),
        T::F32 => ("Float", "try r.f32()", "w.f32"),
        T::F64 => ("Double", "try r.f64()", "w.f64"),
        T::String => ("String", "try r.string()", "w.string"),
        T::ByteArray => ("[UInt8]", "try r.byteArray()", "w.byteArray"),
        _ => return None,
    })
}

/// A homogeneous tuple (`[T; N]`): its item and N.
fn array(items: &[&'static NamedType]) -> (&'static NamedType, usize) {
    let first = items[0];
    assert!(items.iter().all(|i| *i == first), "mixed tuples are not supported in Swift");
    (first, items.len())
}

fn swift_type(nt: &NamedType) -> String {
    if is_named(nt.ty) {
        return ident(nt).to_string();
    }
    if let Some((t, _, _)) = primitive(nt.ty) {
        return t.to_string();
    }
    match nt.ty {
        T::Option(inner) => format!("{}?", swift_type(inner)),
        T::Seq(inner) => format!("[{}]", swift_type(inner)),
        T::Tuple(items) => format!("[{}]", swift_type(array(items).0)),
        other => panic!("{other:?}: not supported in Swift"),
    }
}

/// An expression reading a value of `nt` from `r`.
fn read(nt: &NamedType) -> String {
    if is_named(nt.ty) {
        return format!("try {}(from: &r)", ident(nt));
    }
    if let Some((_, reader, _)) = primitive(nt.ty) {
        return reader.to_string();
    }
    match nt.ty {
        T::Option(inner) => format!("try r.option {{ r in {} }}", read(inner)),
        T::Seq(inner) => format!("try r.seq {{ r in {} }}", read(inner)),
        T::Tuple(items) => {
            let (item, n) = array(items);
            format!("try r.array({n}) {{ r in {} }}", read(item))
        }
        other => panic!("{other:?}"),
    }
}

/// A statement writing `v` as `nt` to `w`.
fn write(nt: &NamedType, v: &str) -> String {
    if is_named(nt.ty) {
        return format!("{v}.encode(to: &w)");
    }
    if let Some((_, _, writer)) = primitive(nt.ty) {
        return format!("{writer}({v})");
    }
    match nt.ty {
        T::Option(inner) => format!("w.option({v}) {{ w, v in {} }}", write(inner, "v")),
        T::Seq(inner) => format!("w.seq({v}) {{ w, v in {} }}", write(inner, "v")),
        T::Tuple(items) => {
            let (item, n) = array(items);
            format!("w.array({v}, {n}) {{ w, v in {} }}", write(item, "v"))
        }
        other => panic!("{other:?}"),
    }
}

fn fields_decl(fields: &[&NamedValue]) -> String {
    fields.iter().map(|f| format!("{}: {}", esc(f.name), swift_type(f.ty))).collect::<Vec<_>>().join(", ")
}

fn type_decl(nt: &NamedType) -> String {
    let name = ident(nt);
    let mut s = String::new();
    match nt.ty {
        T::Struct(fields) => {
            writeln!(s, "public struct {name}: Equatable, PostcardCodable {{").unwrap();
            for f in fields.iter() {
                writeln!(s, "    public var {}: {}", esc(f.name), swift_type(f.ty)).unwrap();
            }
            writeln!(s, "\n    public init({}) {{", fields_decl(fields)).unwrap();
            for f in fields.iter() {
                writeln!(s, "        self.{0} = {0}", esc(f.name)).unwrap();
            }
            writeln!(s, "    }}\n\n    public init(from r: inout PostcardReader) throws {{").unwrap();
            for f in fields.iter() {
                writeln!(s, "        self.{} = {}", esc(f.name), read(f.ty)).unwrap();
            }
            writeln!(s, "    }}\n\n    public func encode(to w: inout PostcardWriter) {{").unwrap();
            for f in fields.iter() {
                writeln!(s, "        {}", write(f.ty, &format!("self.{}", esc(f.name)))).unwrap();
            }
            writeln!(s, "    }}\n}}").unwrap();
        }
        T::Enum(variants) => {
            writeln!(s, "public enum {name}: Equatable, PostcardCodable {{").unwrap();
            for v in variants.iter() {
                let case = match v.ty {
                    V::UnitVariant => format!("case {}", lower(v.name)),
                    V::NewtypeVariant(inner) => format!("case {}({})", lower(v.name), swift_type(inner)),
                    V::TupleVariant(items) => format!(
                        "case {}({})",
                        lower(v.name),
                        items.iter().map(|i| swift_type(i)).collect::<Vec<_>>().join(", ")
                    ),
                    V::StructVariant(fields) => format!("case {}({})", lower(v.name), fields_decl(fields)),
                };
                writeln!(s, "    {case}").unwrap();
            }
            writeln!(s, "\n    public init(from r: inout PostcardReader) throws {{\n        let tag = try r.u32()\n        switch tag {{").unwrap();
            for (i, v) in variants.iter().enumerate() {
                let value = match v.ty {
                    V::UnitVariant => format!(".{}", lower(v.name)),
                    V::NewtypeVariant(inner) => format!(".{}({})", lower(v.name), read(inner)),
                    V::TupleVariant(items) => format!(
                        ".{}({})",
                        lower(v.name),
                        items.iter().map(|i| read(i)).collect::<Vec<_>>().join(", ")
                    ),
                    V::StructVariant(fields) => format!(
                        ".{}({})",
                        lower(v.name),
                        fields.iter().map(|f| format!("{}: {}", esc(f.name), read(f.ty))).collect::<Vec<_>>().join(", ")
                    ),
                };
                writeln!(s, "        case {i}: self = {value}").unwrap();
            }
            writeln!(s, "        default: throw PostcardError.unknownVariant(\"{name}\", tag)\n        }}\n    }}").unwrap();
            writeln!(s, "\n    public func encode(to w: inout PostcardWriter) {{\n        switch self {{").unwrap();
            for (i, v) in variants.iter().enumerate() {
                let (pattern, body) = match v.ty {
                    V::UnitVariant => (format!(".{}", lower(v.name)), String::new()),
                    V::NewtypeVariant(inner) => (format!(".{}(let v0)", lower(v.name)), format!("; {}", write(inner, "v0"))),
                    V::TupleVariant(items) => {
                        let binds: Vec<String> = (0..items.len()).map(|j| format!("let v{j}")).collect();
                        let body: String = items.iter().enumerate().map(|(j, it)| format!("; {}", write(it, &format!("v{j}")))).collect();
                        (format!(".{}({})", lower(v.name), binds.join(", ")), body)
                    }
                    V::StructVariant(fields) => {
                        let binds: Vec<String> = fields.iter().map(|f| format!("let {}", esc(f.name))).collect();
                        let body: String = fields.iter().map(|f| format!("; {}", write(f.ty, &esc(f.name)))).collect();
                        (format!(".{}({})", lower(v.name), binds.join(", ")), body)
                    }
                };
                writeln!(s, "        case {pattern}: w.u32({i}){body}").unwrap();
            }
            writeln!(s, "        }}\n    }}\n}}").unwrap();
        }
        T::NewtypeStruct(inner) => {
            writeln!(s, "public typealias {name} = {}", swift_type(inner)).unwrap();
        }
        other => panic!("{name}: {other:?} not supported in Swift"),
    }
    s
}

/// A field name as a Swift identifier: keywords in backticks. Contextual
/// keywords (`package`, `open`, …) are plain identifiers in these positions,
/// and Swift warns when they are escaped, so they are not listed.
fn esc(name: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "any", "as", "associatedtype", "break", "case", "catch", "class", "continue", "default",
        "defer", "do", "else", "enum", "extension", "fallthrough", "false", "for", "func",
        "guard", "if", "import", "in", "init", "inout", "internal", "is", "let", "nil",
        "operator", "private", "protocol", "public", "repeat", "return", "self",
        "some", "static", "struct", "subscript", "switch", "throw", "throws", "true", "try",
        "typealias", "var", "where", "while",
    ];
    if KEYWORDS.contains(&name) { format!("`{name}`") } else { name.to_string() }
}

/// `ForgetAll` -> `forgetAll`, `None` -> `none`.
fn lower(name: &str) -> String {
    let mut c = name.chars();
    match c.next() {
        Some(first) => first.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

const RUNTIME: &str = include_str!("runtime.swift");

pub fn generate(exports: &[Export], header: &str) -> String {
    let mut order = Vec::new();
    for e in exports {
        collect(e.schema, &mut order);
    }
    let mut s = String::new();
    writeln!(s, "{header}").unwrap();
    s.push_str(RUNTIME);
    s.push_str("\n// ---- types ----\n");
    for nt in &order {
        s.push('\n');
        s.push_str(&type_decl(nt));
    }
    s.push_str("\n// ---- messages ----\n");
    for e in exports {
        let ty = ident(e.schema);
        write!(
            s,
            "\n/// {doc} Throws unless `bytes` is exactly one {name}.\npublic func decode{name}(_ bytes: [UInt8]) throws -> {ty} {{\n    var r = PostcardReader(bytes)\n    let v = try {ty}(from: &r)\n    try r.done()\n    return v\n}}\n\n/// {doc}\npublic func encode{name}(_ value: {ty}) -> [UInt8] {{\n    var w = PostcardWriter()\n    value.encode(to: &w)\n    return w.bytes\n}}\n",
            doc = e.doc,
            name = e.name,
        )
        .unwrap();
    }
    s
}
