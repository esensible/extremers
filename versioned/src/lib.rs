//! This crate provides types and traits to handle versioning of data structures.
//! It allows for keeping track of changes in data over time.


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
impl Atomic for String {}
impl Atomic for str {}


pub trait DeltaTrait {
    type Type;

    fn delta(lhs: &Self, rhs: &Self) -> Option<Self::Type>;
}

impl<T> DeltaTrait for T
where
    T: Atomic + PartialEq + Copy,
{
    type Type = T;

    fn delta(lhs: &T, rhs: &T) -> Option<Self::Type> {
        if lhs == rhs {
            return None;
        }

        Some(*rhs)
    }
}
