//! This crate provides types and traits to handle versioning of data structures.
//! It allows for keeping track of changes in data over time.
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

pub use flatdiff_derive::FlatDiffSer;

pub trait Atomic {}
impl Atomic for i8 {}
impl Atomic for i16 {}
impl Atomic for i32 {}
impl Atomic for i64 {}
impl Atomic for i128 {}

impl Atomic for u8 {}
impl Atomic for u16 {}
impl Atomic for u32 {}
impl Atomic for u64 {}
impl Atomic for u128 {}

impl Atomic for f32 {}
impl Atomic for f64 {}

impl Atomic for isize {}
impl Atomic for usize {}

impl Atomic for bool {}
impl Atomic for char {}
impl Atomic for str {}

pub trait FlatDiffSer {
    fn flatten<S>(
        &self,
        label: &'static str,
        state: &mut S::SerializeStruct,
    ) -> Result<(), S::Error>
    where
        S: Serializer;

    fn diff<S>(
        &self,
        rhs: &Self,
        label: &'static str,
        state: &mut S::SerializeStruct,
    ) -> Result<(), S::Error>
    where
        S: Serializer;

    fn count() -> usize;
}

impl<T: Atomic + PartialEq + Serialize> FlatDiffSer for T {
    fn flatten<S>(
        &self,
        label: &'static str,
        state: &mut S::SerializeStruct,
    ) -> Result<(), S::Error>
    where
        S: Serializer,
    {
        state.serialize_field(label, self)?;

        Ok(())
    }

    fn diff<S>(
        &self,
        rhs: &Self,
        label: &'static str,
        state: &mut S::SerializeStruct,
    ) -> Result<(), S::Error>
    where
        S: Serializer,
    {
        if self != rhs {
            self.flatten::<S>(label, state)?;
        }

        Ok(())
    }

    fn count() -> usize {
        1
    }
}

pub struct Flat<'a, T: FlatDiffSer>(pub &'a T);

impl<'a, T: FlatDiffSer> Serialize for Flat<'a, T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let num_fields = T::count();
        let mut state = serializer.serialize_struct("Race", num_fields + 1)?;
        self.0.flatten::<S>(stringify!(self), &mut state)?;
        state.end()
    }
}

pub struct FlatDiff<'a, T: FlatDiffSer>(pub &'a T, pub &'a T);

impl<'a, T: FlatDiffSer> Serialize for FlatDiff<'a, T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let num_fields = T::count();
        let mut state = serializer.serialize_struct("Race", num_fields)?;
        self.0.diff::<S>(self.1, stringify!(self), &mut state)?;
        state.end()
    }
}


#[cfg(test)]
mod tests {
    #[warn(unused_imports)]
    use super::*;
    use ::serde::{Deserialize, Serialize};
    use serde_json::json;

    #[test]
    fn test_simple() {
        let s1 = InnerStruct::default();
        let mut s2 = InnerStruct::default();

        s2.f1 = 23;
        expect(&s2, &s1, json!({
            "f1": 23,
        }));
    }

    #[test]
    fn test_enum() {
            // variant
        let s1 = EnumStruct {
            enum_: Enum::Var1,
        };
        let s2 = EnumStruct {
            enum_: Enum::Var2 { f1: 23, f2: 42 },
        };
        expect(&s2, &s1, json!({
            "f1": 23,
            "f2": 42,
            "enum_": "Var2",
        }));

        // field within current variant
        let s3 = EnumStruct {
            enum_: Enum::Var2 { f1: 23, f2: 45 },
        };
        expect(&s3, &s2, json!({
            "f2": 45,
        }));

    }

    #[test]
    fn test_nested_atomic() {
        #[derive(FlatDiffSer, Copy, Clone, PartialEq, Default)]
        pub struct InnerStruct {
            f1: u32,
            atomic: AtomicStruct,
        }
        let s1 = InnerStruct::default();

        let s2 = InnerStruct {
            f1: 23,
            atomic: AtomicStruct {
                f1: 23,
                f2: 42,
            },
        };
        expect(&s2, &s1, json!({
            "f1": 23,
            "atomic": {
                "f1": 23,
                "f2": 42,
            },
        }));

        // field within outer
        let mut s2 = InnerStruct::default();
        s2.f1 = 45;
        expect(&s2, &s1, json!({
            "f1": 45,
        }));

        // field within nested atomic
        let mut s2 = InnerStruct::default();
        s2.atomic.f2 = 45;
        expect(&s2, &s1, json!({
            "atomic": {
                "f1": 0,
                "f2": 45,
            },
        }));
    }

    #[derive(Serialize, Deserialize, Copy, Clone, PartialEq, Default)]
    pub struct AtomicStruct {
        f1: u32,
        f2: u32,    
    }

    impl Atomic for AtomicStruct {}

    #[derive(FlatDiffSer, Copy, Clone, PartialEq, Default)]
    pub struct InnerStruct {
        // #[delta(skip)]
        f1: u32,
        f2: u32,
    }

    // #[derive(FlatDiffSer, Copy, Clone, PartialEq)]
    // pub struct OuterStruct {
    //     // #[delta(skip)]
    //     // location: Location,
    //     enum_: Enum,
    //     inner_struct: InnerStruct,
    // }
    
    #[derive(FlatDiffSer, Copy, Clone, PartialEq)]
    pub struct EnumStruct {
        enum_: Enum,
    }

    #[derive(FlatDiffSer, Copy, Clone, PartialEq)]
    enum Enum {
        Var1,
        Var2 {
            f1: u32,
            f2: u32,
        },
        Var3 {
            inner_struct: InnerStruct,
        },
    }

    fn expect<T: FlatDiffSer>(new_val: &T, old_val: &T, expected: serde_json::Value) {
        let diff = FlatDiff(new_val, old_val);
        let json = serde_json::to_string(&diff).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value, expected);
    }

}