use crate::event_core::EngineCore;

#[derive(Copy, Clone)]
pub struct Callback<M: EngineCore, T: Copy> {
    f: fn(&mut M, &T),
    value: T,
}

impl<M: EngineCore, T: Copy> Callback<M, T> {
    pub fn new(f: fn(&mut M, &T), value: T) -> Self {
        Self { f, value }
    }

    pub fn invoke(&self, mut_val: &mut M) {
        (self.f)(mut_val, &self.value);
    }
}

pub trait CallbackTrait<T: EngineCore> {
    fn invoke(&self, mut_val: &mut T);
}

#[macro_export]
macro_rules! callbacks {
    ($mut_ty:ident, $vis:vis $enum_name:ident { $($variant:ident($ty:ty)),* $(,)? }) => {
        #[derive(Copy, Clone)]
        $vis enum $enum_name {
            $(
                $variant($crate::callbacks::Callback<$mut_ty, $ty>),
            )*
        }

        impl $crate::callbacks::CallbackTrait<$mut_ty> for $enum_name {
            fn invoke(&self, mut_val: &mut $mut_ty) {
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
                        $enum_name::$variant($crate::callbacks::Callback::new(f, value))
                    }
                }
            )*
        }
    };
}
