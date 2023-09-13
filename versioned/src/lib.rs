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

/// A trait that types should implement to be considered "versioned".
///
/// Provides the necessary mechanisms to keep track of changes and derive 
/// versioned and delta representations of the data.
pub trait Versioned {
    /// Represents the versioned type.
    type Value;

    /// Represents the delta or difference type.
    type Delta;
   
    /// Create a new versioned value.
    ///
    /// # Arguments
    ///
    /// * `value` - The actual value to be versioned.
    /// * `version` - The version number for the value.
    fn new(value: Self, version: usize) -> VersionedValue<Self::Value>;

    /// Get the delta of a versioned value for a specific version.
    ///
    /// # Arguments
    ///
    /// * `value` - The versioned value to derive the delta from.
    /// * `version` - The version number to retrieve the delta for.    
    fn get(value: &VersionedValue<Self::Value>, version: usize) -> DeltaType<Self>;
}

/// Represents the versioned type of a given type `T`.
pub type VersionedType<T> = <T as Versioned>::Value;

/// Represents the delta or difference type of a given type `T`.
pub type DeltaType<T> = Option<<T as Versioned>::Delta>;

/// A structure that holds the actual versioned value and its version number.
pub struct VersionedValue<T> {
    /// The versioned value.
    pub value: T,


    /// The version number.
    pub version: usize,
}


/// Default implementation of the `Versioned` trait for types that implement `Atomic`.
impl<T> Versioned for T
where
    T: Atomic + Clone,
{
    type Value = T;
    type Delta = T;

    fn new(value: T, version: usize) -> VersionedValue<Self::Value> {
        VersionedValue{
            value,
            version: version,
        }
    }

    fn get(value: &VersionedValue<Self::Value>, version: usize) -> DeltaType<Self> {
        Some(value.value.clone()).filter(|_| value.version >= version)
    }
}


/// A macro to help update fields of versioned types - where the magic happens!
///
/// This macro aids in updating fields of versioned data structures 
/// and increments the version number appropriately.
#[macro_export]
macro_rules! update {
    ($outer:ident.$field:ident, $value:expr) => {
        {
            $outer.version += 1;
            $outer.value.$field = ::versioned::Versioned::new($value, $outer.version);
        }
    };

    ($outer:ident.$field:ident, $enum_type:ident::$variant:ident { $enum_field:ident: _ }, $value:expr) => {
        {
            type TmpType = ::versioned::VersionedType<$enum_type>;
            if let TmpType::$variant { ref mut $enum_field, .. } = &mut $outer.value.$field.value {
                $outer.version += 1;
                *$enum_field = ::versioned::Versioned::new($value, $outer.version);
            }
        }
    };
}
