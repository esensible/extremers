#[derive(Copy, Clone)]
pub struct Callback<M, T: Copy> {
    f: fn(&mut M, &T),
    value: T,
}

impl<M, T: Copy> Callback<M, T> {
    pub fn new(f: fn(&mut M, &T), value: T) -> Self {
        Self { f, value }
    }

    pub fn invoke(&self, mut_val: &mut M) {
        (self.f)(mut_val, &self.value);
    }
}

pub trait CallbackTrait {
    type T;

    fn invoke(&self, mut_val: &mut Self::T);
}

#[macro_export]
macro_rules! callbacks {
    ($mut_ty:ident, $vis:vis $enum_name:ident { $($variant:ident($ty:ty)),* $(,)? }) => {
        #[derive(Copy, Clone)]
        $vis enum $enum_name {
            $(
                $variant(crate::core::Callback<$mut_ty, $ty>),
            )*
        }

        impl crate::core::CallbackTrait for $enum_name {
            type T = $mut_ty;

            fn invoke(&self, mut_val: &mut Self::T) {
                match self {
                    $(
                        $enum_name::$variant(closure) => closure.invoke(mut_val),
                    )*
                }
            }
        }

        paste::paste! {
            $vis trait [<_NewClosure $enum_name>] {
                fn new(f: fn(&mut $mut_ty, &Self), value: Self) -> $enum_name;
            }


            $(
                impl [<_NewClosure $enum_name>] for $ty {
                    fn new(f: fn(&mut $mut_ty, &Self), value: Self) -> $enum_name {
                        $enum_name::$variant(crate::core::Callback::new(f, value))
                    }
                }
            )*
        }
    };
}
