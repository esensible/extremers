#![feature(const_generics)]
#![feature(const_evaluatable_checked)]
#![allow(incomplete_features)]
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use versioned::DeltaTrait;
// use paste::paste;
use serde_json_core::{from_slice, to_slice};

use httparse::{Request, EMPTY_HEADER};

pub trait EventHandler where Self::Event: Deserialize<'static> + DeserializeOwned  {

    type Event;
    type Callbacks;

    fn handle_event(&mut self, event: Self::Event, sleep: &dyn FnMut(u32, Self::Callbacks)) -> Result<(), &'static str>;
}

impl<T: EventHandler + DeltaTrait + Default, const N: usize> Default for EngineWrapper<T, N> 
where
    <T as DeltaTrait>::Type: Serialize,
    <T as EventHandler>::Callbacks: Copy,
{
    fn default() -> Self {
        EngineWrapper(T::default(), [None; N])
    }
}

#[derive(Copy, Clone)]
pub struct EngineWrapper<T: EventHandler + DeltaTrait, const N: usize>(T, [Option<T::Callbacks>; N]) where
    <T as DeltaTrait>::Type: Serialize;

impl<T, const N: usize> EngineWrapper<T, N> 
where
    T: EventHandler + DeltaTrait + Clone,
    <T as DeltaTrait>::Type: Serialize {

    pub fn handle_event(&mut self, event: &[u8], result: &mut [u8], sleep: &dyn Fn(usize, usize)) -> Result<(), &'static str> {

        let (event, _): (T::Event, usize) = from_slice(event).map_err(|_| {"Invalid JSON event"})?;

        let transformed_sleep = |time: u32, callback: T::Callbacks| {
            if let Some(pos) = self.1.iter_mut().position(|x| {x.is_none()}) {
                self.1[pos] = Some(callback);
                sleep(time as usize, pos);
            } else {
                panic!();
            }
        };

        let old_value = self.0.clone();
        let update = self.0.handle_event(event, &transformed_sleep)?;
        let delta = T::delta(&old_value, &self.0);
        if let Some(delta) = delta {
            to_slice(&delta, result).map_err(|_| "Failed to serialize delta")?;
        }
        Ok(())

    }

    pub fn handle_request(&mut self, body: &[u8], response: &mut [u8], sleep: &dyn Fn(usize, usize)) -> Result<(), &'static str> {
        // Buffer to hold HTTP request headers
        let mut headers = [EMPTY_HEADER; 16];
    
        // Parsing the request
        let mut req = Request::new(&mut headers);
        let status = req.parse(body).map_err(|_| "Invalid HTTP request")?;
    
        // Check if the headers were fully parsed
        if let httparse::Status::Complete(offset) = status {
            if req.method == Some("POST") && req.path == Some("/events") {
                let content_length: usize = req.headers.iter()
                    .filter(|header| header.name.eq_ignore_ascii_case("Content-Length"))
                    .filter_map(|header| core::str::from_utf8(header.value).ok()?.parse().ok())
                    .next()
                    .ok_or("Content-Length not found or invalid")?;
    
                // Assuming the body starts right after the headers
                let event_body = &body[offset..offset + content_length];
    

                // Process the event
                self.handle_event(event_body, response, sleep)?;
    
                // Set response
                let response_str = "HTTP/1.1 200 OK\r\n\r\n";
                response[0..response_str.len()].copy_from_slice(response_str.as_bytes());
                Ok(())
            } else {
                // Unsupported HTTP method or path
                Err("Unsupported HTTP method or path")
            }
        } else {
            Err("Incomplete HTTP request")
        }
    }
     

}

#[derive(Copy, Clone)]
pub struct Closure<M, T: Copy> {
    f: fn(&mut M, &T),
    value: T,
}

impl<M, T: Copy> Closure<M, T> {
    pub fn new(f: fn(&mut M, &T), value: T) -> Self {
        Self { f, value }
    }

    pub fn invoke(&self, mut_val: &mut M) {
        (self.f)(mut_val, &self.value);
    }
}


#[macro_export]
macro_rules! closure {
    ($mut_ty:ident, $vis:vis $enum_name:ident { $($variant:ident($ty:ty)),* $(,)? }) => {
        #[derive(Copy, Clone)]
        $vis enum $enum_name {
            $(
                $variant(crate::engine_traits::Closure<$mut_ty, $ty>),
            )*
        }

        impl $enum_name {
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
                        $enum_name::$variant(crate::engine_traits::Closure::new(f, value))
                    }
                }
            )*
        }
    };
}
