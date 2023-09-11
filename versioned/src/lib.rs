//! This crate provides types and traits to handle versioning of data structures.
//! It allows for keeping track of changes in data over time.

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
    fn get(value: VersionedValue<Self::Value>, version: usize) -> DeltaType<Self>;
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


/// Default implementation of the `Versioned` trait for types that implement `Default`.
impl<T> Versioned for T
where
    T: Default,
{
    type Value = T;
    type Delta = T;

    fn new(value: T, version: usize) -> VersionedValue<Self::Value> {
        VersionedValue{
            value,
            version: version,
        }
    }

    fn get(value: VersionedValue<Self::Value>, version: usize) -> DeltaType<Self> {
        Some(value.value).filter(|_| value.version >= version)
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
            $outer.value.$field = Versioned::new($value, $outer.version);
        }
    };

    ($outer:ident.$field:ident, $enum_type:ident::$variant:ident { $enum_field:ident: _ }, $value:expr) => {
        {
            type TmpType = VersionedType<$enum_type>;
            if let TmpType::$variant { ref mut $enum_field, .. } = &mut $outer.value.$field.value {
                $outer.version += 1;
                *$enum_field = Versioned::new($value, $outer.version);
            }
        }
    };
}
