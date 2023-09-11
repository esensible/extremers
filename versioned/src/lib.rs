pub trait Versioned {
    type Value;
    type Delta;
   
    fn new(value: Self, version: usize) -> VersionedValue<Self::Value>;
    fn get(value: VersionedValue<Self::Value>, version: usize) -> Self::Delta;
}

pub type VersionedType<T> = <T as Versioned>::Value;
pub type DeltaType<T> = <T as Versioned>::Delta;

pub struct VersionedValue<T> {
    pub value: T,
    pub version: usize,
}


impl<T> Versioned for T
where
    T: Default,
{
    type Value = T;
    type Delta = Option<T>;

    fn new(value: T, version: usize) -> VersionedValue<Self::Value> {
        VersionedValue{
            value,
            version: version,
        }
    }

    fn get(value: VersionedValue<Self::Value>, version: usize) -> Self::Delta {
        Some(value.value).filter(|_| value.version >= version)
    }
}


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
