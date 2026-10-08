//! The JS value a Rust value decodes to (see the table in `js.rs`), as
//! JSON, computed from the Rust value through serde: independent of the
//! bytes, so the golden vectors check the generated decoder against the
//! types rather than against another decoder.

use serde::ser::{self, Serialize};
use serde_json::{Map, Number, Value, json};

#[derive(Debug)]
pub struct Error(String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl ser::Error for Error {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Error(msg.to_string())
    }
}

/// `value` as the generated decoder returns it.
pub fn to_js<T: Serialize + ?Sized>(value: &T) -> Result<Value, Error> {
    value.serialize(Shape)
}

const MAX_SAFE: u64 = (1 << 53) - 1;

fn tagged(tag: &str, value: Value) -> Value {
    json!({ "tag": tag, "value": value })
}

fn float(v: f64) -> Result<Value, Error> {
    Number::from_f64(v)
        .map(Value::Number)
        .ok_or_else(|| Error(format!("{v} has no JSON form")))
}

struct Shape;

pub struct Seq {
    items: Vec<Value>,
    tag: Option<&'static str>,
}

pub struct Struct {
    fields: Map<String, Value>,
    tag: Option<&'static str>,
}

impl ser::Serializer for Shape {
    type Ok = Value;
    type Error = Error;
    type SerializeSeq = Seq;
    type SerializeTuple = Seq;
    type SerializeTupleStruct = Seq;
    type SerializeTupleVariant = Seq;
    type SerializeMap = ser::Impossible<Value, Error>;
    type SerializeStruct = Struct;
    type SerializeStructVariant = Struct;

    fn serialize_bool(self, v: bool) -> Result<Value, Error> {
        Ok(Value::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> Result<Value, Error> {
        Ok(v.into())
    }
    fn serialize_i16(self, v: i16) -> Result<Value, Error> {
        Ok(v.into())
    }
    fn serialize_i32(self, v: i32) -> Result<Value, Error> {
        Ok(v.into())
    }
    fn serialize_i64(self, v: i64) -> Result<Value, Error> {
        if v.unsigned_abs() > (1 << 52) {
            return Err(Error(format!("{v} is beyond the decoder's 2^52")));
        }
        Ok(v.into())
    }
    fn serialize_u8(self, v: u8) -> Result<Value, Error> {
        Ok(v.into())
    }
    fn serialize_u16(self, v: u16) -> Result<Value, Error> {
        Ok(v.into())
    }
    fn serialize_u32(self, v: u32) -> Result<Value, Error> {
        Ok(v.into())
    }
    fn serialize_u64(self, v: u64) -> Result<Value, Error> {
        if v > MAX_SAFE {
            return Err(Error(format!("{v} is beyond the decoder's 2^53")));
        }
        Ok(v.into())
    }
    fn serialize_f32(self, v: f32) -> Result<Value, Error> {
        float(v as f64)
    }
    fn serialize_f64(self, v: f64) -> Result<Value, Error> {
        float(v)
    }
    fn serialize_char(self, v: char) -> Result<Value, Error> {
        Ok(Value::String(v.to_string()))
    }
    fn serialize_str(self, v: &str) -> Result<Value, Error> {
        Ok(Value::String(v.to_string()))
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<Value, Error> {
        Ok(Value::Array(v.iter().map(|&b| b.into()).collect()))
    }
    fn serialize_none(self) -> Result<Value, Error> {
        Ok(Value::Null)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Value, Error> {
        let v = value.serialize(Shape)?;
        if v.is_null() {
            return Err(Error("Some(null) would read as None".into()));
        }
        Ok(v)
    }
    fn serialize_unit(self) -> Result<Value, Error> {
        Ok(Value::Null)
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Value, Error> {
        Ok(Value::Null)
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, variant: &'static str) -> Result<Value, Error> {
        Ok(tagged(variant, Value::Null))
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, value: &T) -> Result<Value, Error> {
        value.serialize(Shape)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Value, Error> {
        Ok(tagged(variant, value.serialize(Shape)?))
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<Seq, Error> {
        Ok(Seq {
            items: Vec::with_capacity(len.unwrap_or(0)),
            tag: None,
        })
    }
    fn serialize_tuple(self, len: usize) -> Result<Seq, Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(self, _: &'static str, len: usize) -> Result<Seq, Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Seq, Error> {
        Ok(Seq {
            items: Vec::with_capacity(len),
            tag: Some(variant),
        })
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Err(Error("maps are not supported".into()))
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Struct, Error> {
        Ok(Struct {
            fields: Map::new(),
            tag: None,
        })
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Struct, Error> {
        Ok(Struct {
            fields: Map::new(),
            tag: Some(variant),
        })
    }
}

impl Seq {
    fn push<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.items.push(value.serialize(Shape)?);
        Ok(())
    }
    fn finish(self) -> Value {
        let array = Value::Array(self.items);
        match self.tag {
            Some(tag) => tagged(tag, array),
            None => array,
        }
    }
}

impl ser::SerializeSeq for Seq {
    type Ok = Value;
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.push(value)
    }
    fn end(self) -> Result<Value, Error> {
        Ok(self.finish())
    }
}

impl ser::SerializeTuple for Seq {
    type Ok = Value;
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.push(value)
    }
    fn end(self) -> Result<Value, Error> {
        Ok(self.finish())
    }
}

impl ser::SerializeTupleStruct for Seq {
    type Ok = Value;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.push(value)
    }
    fn end(self) -> Result<Value, Error> {
        Ok(self.finish())
    }
}

impl ser::SerializeTupleVariant for Seq {
    type Ok = Value;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.push(value)
    }
    fn end(self) -> Result<Value, Error> {
        Ok(self.finish())
    }
}

impl Struct {
    fn finish(self) -> Value {
        let object = Value::Object(self.fields);
        match self.tag {
            Some(tag) => tagged(tag, object),
            None => object,
        }
    }
}

impl ser::SerializeStruct for Struct {
    type Ok = Value;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Error> {
        self.fields.insert(key.to_string(), value.serialize(Shape)?);
        Ok(())
    }
    fn end(self) -> Result<Value, Error> {
        Ok(self.finish())
    }
}

impl ser::SerializeStructVariant for Struct {
    type Ok = Value;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Error> {
        self.fields.insert(key.to_string(), value.serialize(Shape)?);
        Ok(())
    }
    fn end(self) -> Result<Value, Error> {
        Ok(self.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    enum E {
        Unit,
        New(u8),
        Tup(u8, i16),
        Rec { a: Option<u32>, b: f32 },
    }

    #[test]
    fn shapes() {
        assert_eq!(to_js(&E::Unit).unwrap(), json!({"tag": "Unit", "value": null}));
        assert_eq!(to_js(&E::New(3)).unwrap(), json!({"tag": "New", "value": 3}));
        assert_eq!(to_js(&E::Tup(1, -2)).unwrap(), json!({"tag": "Tup", "value": [1, -2]}));
        assert_eq!(
            to_js(&E::Rec { a: None, b: 0.5 }).unwrap(),
            json!({"tag": "Rec", "value": {"a": null, "b": 0.5}})
        );
        assert_eq!(to_js(&[1u8, 2]).unwrap(), json!([1, 2]));
        assert_eq!(to_js(&()).unwrap(), Value::Null);
        // f32 exactly as a double
        assert_eq!(to_js(&6.4f32).unwrap(), json!(6.4f32 as f64));
        assert!(to_js(&u64::MAX).is_err());
        assert!(to_js(&Some(())).is_err());
    }
}
