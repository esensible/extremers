//! JS and TypeScript from postcard-schema schemas.
//!
//! For every named type (struct or enum) reachable from the exported
//! messages this emits a reader `r_<Name>(r)` and a writer
//! `w_<Name>(w, v)`; each exported message gets `decode<Name>(bytes)` and
//! `encode<Name>(value)`. The JS value of a Rust value (also in
//! `protocol.d.ts`):
//!
//! | Rust | JS |
//! |---|---|
//! | struct | object with every field, in order |
//! | enum | `{ tag: "Variant", value }`: `value` is `null` for a unit variant, the data for a newtype variant, an array for a tuple variant, an object for a struct variant |
//! | `Option<T>` | the value, or `null` |
//! | `()`, unit struct | `null` |
//! | newtype struct | its field |
//! | tuple, tuple struct, `[T; N]`, sequence | array |
//! | `bool`, integers, floats | boolean, number |
//! | `u64`/`i64`/`usize`/`isize` | number: decoding throws past 2^53 (2^52 for signed); no BigInt, which old browsers lack |
//! | `String`, `char` | string |
//! | byte array | array of numbers |
//!
//! Every field is always present, so the shapes are stable (what Solid's
//! `reconcile` wants). Maps and 128-bit integers are not supported.
//!
//! The runtime helpers at the top of the file use nothing newer than ES5
//! plus typed arrays: no BigInt, no TextDecoder (strings are UTF-8 coded by
//! hand), so the same file runs in the Kindle's old WebKit and in QuickJS.

use std::collections::BTreeMap;
use std::fmt::Write;

use postcard_schema::schema::{DataModelType as T, DataModelVariant as V, NamedType};

/// One exported message: its JS name and schema.
pub struct Export {
    pub name: &'static str,
    pub schema: &'static NamedType,
    /// What the message is, for the generated file's comments.
    pub doc: &'static str,
}

/// The named types reachable from the exports, by name, in first-seen
/// order (a BTreeMap of name -> schema, plus an order list).
struct Named {
    order: Vec<&'static str>,
    by_name: BTreeMap<&'static str, &'static NamedType>,
}

fn is_named(ty: &T) -> bool {
    matches!(
        ty,
        T::Struct(_) | T::Enum(_) | T::TupleStruct(_) | T::NewtypeStruct(_) | T::UnitStruct
    )
}

/// A JS identifier for a schema's type name: the name up to any generic
/// arguments (`State<T>` is `State`).
fn ident(nt: &NamedType) -> &'static str {
    let name = nt.name;
    let end = name.find(['<', ' ']).unwrap_or(name.len());
    &name[..end]
}

impl Named {
    fn collect(exports: &[Export]) -> Self {
        let mut named = Named {
            order: Vec::new(),
            by_name: BTreeMap::new(),
        };
        for e in exports {
            named.visit(e.schema);
        }
        named
    }

    fn visit(&mut self, nt: &'static NamedType) {
        if is_named(nt.ty) {
            let name = ident(nt);
            if let Some(seen) = self.by_name.get(name) {
                assert!(
                    *seen == nt,
                    "two different types are both named {name}: rename one"
                );
                return;
            }
            self.by_name.insert(name, nt);
            self.order.push(name);
        }
        match nt.ty {
            T::Option(inner) => {
                assert!(
                    !matches!(inner.ty, T::Option(_)),
                    "Option<Option<_>> has no unambiguous JS form"
                );
                self.visit(inner)
            }
            T::NewtypeStruct(inner) | T::Seq(inner) => self.visit(inner),
            T::Tuple(items) | T::TupleStruct(items) => items.iter().for_each(|i| self.visit(i)),
            T::Struct(fields) => fields.iter().for_each(|f| self.visit(f.ty)),
            T::Enum(variants) => {
                for v in variants.iter() {
                    match v.ty {
                        V::UnitVariant => {}
                        V::NewtypeVariant(inner) => self.visit(inner),
                        V::TupleVariant(items) => items.iter().for_each(|i| self.visit(i)),
                        V::StructVariant(fields) => fields.iter().for_each(|f| self.visit(f.ty)),
                    }
                }
            }
            T::Map { .. } => panic!("{}: maps are not supported", nt.name),
            T::I128 | T::U128 => panic!("{}: 128-bit integers are not supported", nt.name),
            T::Schema => panic!("{}: schema values are not supported", nt.name),
            _ => {}
        }
    }
}

/// The primitive reader and writer for a type, if it is a primitive.
fn primitive(ty: &T) -> Option<(&'static str, &'static str, &'static str)> {
    // (reader, writer, TS type)
    Some(match ty {
        T::Bool => ("rbool", "wbool", "boolean"),
        T::I8 => ("ri8", "wi8", "number"),
        T::U8 => ("ru8", "wu8", "number"),
        T::I16 => ("ri16", "wi16", "number"),
        T::I32 => ("ri32", "wi32", "number"),
        T::I64 | T::Isize => ("ri64", "wi64", "number"),
        T::U16 => ("ru16", "wu16", "number"),
        T::U32 => ("ru32", "wu32", "number"),
        T::U64 | T::Usize => ("ru64", "wu64", "number"),
        T::F32 => ("rf32", "wf32", "number"),
        T::F64 => ("rf64", "wf64", "number"),
        T::Char => ("rchar", "wchar", "string"),
        T::String => ("rstr", "wstr", "string"),
        T::ByteArray => ("rbytes", "wbytes", "number[]"),
        T::Unit | T::UnitStruct => ("runit", "wunit", "null"),
        _ => return None,
    })
}

/// A JS expression reading a value of `nt` from reader `r`.
fn read_expr(nt: &NamedType) -> String {
    if is_named(nt.ty) {
        return format!("r_{}(r)", ident(nt));
    }
    if let Some((reader, _, _)) = primitive(nt.ty) {
        return format!("{reader}(r)");
    }
    match nt.ty {
        T::Option(inner) => format!("ropt(r, {})", read_fn(inner)),
        T::Seq(inner) => format!("rseq(r, {})", read_fn(inner)),
        T::Tuple(items) => {
            let items: Vec<String> = items.iter().map(|i| read_expr(i)).collect();
            format!("[{}]", items.join(", "))
        }
        other => unreachable!("{other:?}"),
    }
}

/// A JS expression for a function reading a value of `nt`.
fn read_fn(nt: &NamedType) -> String {
    if is_named(nt.ty) {
        return format!("r_{}", ident(nt));
    }
    if let Some((reader, _, _)) = primitive(nt.ty) {
        return reader.to_string();
    }
    format!("function (r) {{ return {}; }}", read_expr(nt))
}

/// A JS statement writing `v` (an expression) as a value of `nt` to `w`.
fn write_stmt(nt: &NamedType, v: &str) -> String {
    if is_named(nt.ty) {
        return format!("w_{}(w, {v});", ident(nt));
    }
    if let Some((_, writer, _)) = primitive(nt.ty) {
        return format!("{writer}(w, {v});");
    }
    match nt.ty {
        T::Option(inner) => format!("wopt(w, {v}, {});", write_fn(inner)),
        T::Seq(inner) => format!("wseq(w, {v}, {});", write_fn(inner)),
        T::Tuple(items) => {
            let mut s = format!("wlen({v}, {}, \"{}\");", items.len(), nt.name);
            for (i, item) in items.iter().enumerate() {
                s.push(' ');
                s.push_str(&write_stmt(item, &format!("{v}[{i}]")));
            }
            s
        }
        other => unreachable!("{other:?}"),
    }
}

/// A JS expression for a function writing a value of `nt`.
fn write_fn(nt: &NamedType) -> String {
    if is_named(nt.ty) {
        return format!("w_{}", ident(nt));
    }
    if let Some((_, writer, _)) = primitive(nt.ty) {
        return writer.to_string();
    }
    format!("function (w, v) {{ {} }}", write_stmt(nt, "v"))
}

/// The TypeScript type of `nt`.
fn ts_type(nt: &NamedType) -> String {
    if is_named(nt.ty) {
        return ident(nt).to_string();
    }
    if let Some((_, _, ts)) = primitive(nt.ty) {
        return ts.to_string();
    }
    match nt.ty {
        T::Option(inner) => format!("{} | null", ts_type(inner)),
        T::Seq(inner) => format!("Array<{}>", ts_type(inner)),
        T::Tuple(items) => {
            let items: Vec<String> = items.iter().map(|i| ts_type(i)).collect();
            format!("[{}]", items.join(", "))
        }
        other => unreachable!("{other:?}"),
    }
}

fn reader_body(name: &str, nt: &NamedType) -> String {
    match nt.ty {
        T::Struct(fields) => {
            let fields: Vec<String> = fields
                .iter()
                .map(|f| format!("{}: {}", f.name, read_expr(f.ty)))
                .collect();
            format!("return {{ {} }};", fields.join(", "))
        }
        T::UnitStruct => "return null;".to_string(),
        T::NewtypeStruct(inner) => format!("return {};", read_expr(inner)),
        T::TupleStruct(items) => {
            let items: Vec<String> = items.iter().map(|i| read_expr(i)).collect();
            format!("return [{}];", items.join(", "))
        }
        T::Enum(variants) => {
            let mut s = String::from("var i = rtag(r);\n  switch (i) {\n");
            for (i, v) in variants.iter().enumerate() {
                let value = match v.ty {
                    V::UnitVariant => "null".to_string(),
                    V::NewtypeVariant(inner) => read_expr(inner),
                    V::TupleVariant(items) => {
                        let items: Vec<String> = items.iter().map(|i| read_expr(i)).collect();
                        format!("[{}]", items.join(", "))
                    }
                    V::StructVariant(fields) => {
                        let fields: Vec<String> = fields
                            .iter()
                            .map(|f| format!("{}: {}", f.name, read_expr(f.ty)))
                            .collect();
                        format!("{{ {} }}", fields.join(", "))
                    }
                };
                writeln!(s, "    case {i}: return {{ tag: \"{}\", value: {value} }};", v.name).unwrap();
            }
            write!(
                s,
                "    default: throw new Error(\"{name}: no variant \" + i);\n  }}"
            )
            .unwrap();
            s
        }
        other => unreachable!("{other:?}"),
    }
}

fn writer_body(name: &str, nt: &NamedType) -> String {
    match nt.ty {
        T::Struct(fields) => {
            let mut s = format!("wobj(v, \"{name}\");");
            for f in fields.iter() {
                s.push_str("\n  ");
                s.push_str(&write_stmt(f.ty, &format!("v.{}", f.name)));
            }
            s
        }
        T::UnitStruct => String::new(),
        T::NewtypeStruct(inner) => write_stmt(inner, "v"),
        T::TupleStruct(items) => {
            let mut s = format!("wlen(v, {}, \"{name}\");", items.len());
            for (i, item) in items.iter().enumerate() {
                s.push_str("\n  ");
                s.push_str(&write_stmt(item, &format!("v[{i}]")));
            }
            s
        }
        T::Enum(variants) => {
            let mut s = format!("wobj(v, \"{name}\");\n  switch (v.tag) {{\n");
            for (i, v) in variants.iter().enumerate() {
                write!(s, "    case \"{}\": wtag(w, {i});", v.name).unwrap();
                match v.ty {
                    V::UnitVariant => {}
                    V::NewtypeVariant(inner) => {
                        write!(s, " {}", write_stmt(inner, "v.value")).unwrap();
                    }
                    V::TupleVariant(items) => {
                        write!(s, " wlen(v.value, {}, \"{name}::{}\");", items.len(), v.name).unwrap();
                        for (j, item) in items.iter().enumerate() {
                            write!(s, " {}", write_stmt(item, &format!("v.value[{j}]"))).unwrap();
                        }
                    }
                    V::StructVariant(fields) => {
                        write!(s, " wobj(v.value, \"{name}::{}\");", v.name).unwrap();
                        for f in fields.iter() {
                            write!(s, " {}", write_stmt(f.ty, &format!("v.value.{}", f.name))).unwrap();
                        }
                    }
                }
                s.push_str(" return;\n");
            }
            write!(
                s,
                "    default: throw new Error(\"{name}: no variant \" + v.tag);\n  }}"
            )
            .unwrap();
            s
        }
        other => unreachable!("{other:?}"),
    }
}

fn ts_decl(name: &str, nt: &NamedType) -> String {
    let fields_ts = |fields: &[&postcard_schema::schema::NamedValue]| -> String {
        let fields: Vec<String> = fields
            .iter()
            .map(|f| format!("{}: {}", f.name, ts_type(f.ty)))
            .collect();
        format!("{{ {} }}", fields.join("; "))
    };
    let body = match nt.ty {
        T::Struct(fields) => fields_ts(fields),
        T::UnitStruct => "null".to_string(),
        T::NewtypeStruct(inner) => ts_type(inner),
        T::TupleStruct(items) => {
            let items: Vec<String> = items.iter().map(|i| ts_type(i)).collect();
            format!("[{}]", items.join(", "))
        }
        T::Enum(variants) => {
            let arms: Vec<String> = variants
                .iter()
                .map(|v| {
                    let value = match v.ty {
                        V::UnitVariant => "null".to_string(),
                        V::NewtypeVariant(inner) => ts_type(inner),
                        V::TupleVariant(items) => {
                            let items: Vec<String> = items.iter().map(|i| ts_type(i)).collect();
                            format!("[{}]", items.join(", "))
                        }
                        V::StructVariant(fields) => fields_ts(fields),
                    };
                    format!("\n  | {{ tag: \"{}\"; value: {value} }}", v.name)
                })
                .collect();
            arms.concat()
        }
        other => unreachable!("{other:?}"),
    };
    let sep = if body.starts_with('\n') { "" } else { " " };
    format!("export type {name} ={sep}{body};\n")
}

/// The runtime helpers every generated file starts with.
const RUNTIME: &str = include_str!("runtime.js");

pub struct Generated {
    pub js: String,
    pub dts: String,
}

pub fn generate(exports: &[Export], header: &str) -> Generated {
    let named = Named::collect(exports);

    let mut js = String::new();
    writeln!(js, "{header}").unwrap();
    js.push_str(RUNTIME);

    let mut dts = String::new();
    writeln!(dts, "{header}").unwrap();

    js.push_str("\n// ---- types ----\n");
    for name in &named.order {
        let nt = named.by_name[name];
        write!(
            js,
            "\nfunction r_{name}(r) {{\n  {}\n}}\nfunction w_{name}(w, v) {{\n  {}\n}}\n",
            reader_body(name, nt),
            writer_body(name, nt)
        )
        .unwrap();
        dts.push('\n');
        dts.push_str(&ts_decl(name, nt));
    }

    js.push_str("\n// ---- messages ----\n");
    dts.push_str("\n/** Bytes as received: a Uint8Array, an ArrayBuffer or an array of numbers. */\nexport type Bytes = Uint8Array | ArrayBuffer | ArrayLike<number>;\n");
    for e in exports {
        let ty = ident(e.schema);
        assert!(is_named(e.schema.ty), "{}: exports must be structs or enums", e.name);
        write!(
            js,
            "\n/** {doc} Throws on bytes that are not exactly one {name}. */\nexport function decode{name}(bytes) {{\n  var r = reader(bytes);\n  var v = r_{ty}(r);\n  done(r, \"{name}\");\n  return v;\n}}\n/** {doc} As an array of byte values. */\nexport function encode{name}(value) {{\n  var w = writer();\n  w_{ty}(w, value);\n  return w.b;\n}}\n",
            doc = e.doc,
            name = e.name,
        )
        .unwrap();
        write!(
            dts,
            "\n/** {doc} */\nexport function decode{name}(bytes: Bytes): {ty};\n/** {doc} */\nexport function encode{name}(value: {ty}): number[];\n",
            doc = e.doc,
            name = e.name,
        )
        .unwrap();
    }

    // variant names of every enum, in wire order
    js.push_str("\n/** Every enum's variant names, in wire order. */\nexport var VARIANTS = {\n");
    dts.push_str("\n/** Every enum's variant names, in wire order. */\nexport const VARIANTS: {\n");
    for name in &named.order {
        if let T::Enum(variants) = named.by_name[name].ty {
            let names: Vec<String> = variants.iter().map(|v| format!("\"{}\"", v.name)).collect();
            writeln!(js, "  {name}: [{}],", names.join(", ")).unwrap();
            writeln!(dts, "  {name}: Array<{name}[\"tag\"]>;").unwrap();
        }
    }
    js.push_str("};\n");
    dts.push_str("};\n");

    Generated { js, dts }
}
