//! This crate provides types and traits to handle versioning of data structures.
//! It allows for keeping track of changes in data over time.
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

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
        let mut state = serializer.serialize_struct("Flat", num_fields + 1)?;
        self.0.flatten::<S>(core::stringify!(self), &mut state)?;
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
        let mut state = serializer.serialize_struct("FlatDiff", num_fields)?;
        self.0.diff::<S>(self.1, core::stringify!(self), &mut state)?;
        state.end()
    }
}


