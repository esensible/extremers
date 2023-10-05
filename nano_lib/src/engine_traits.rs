#![allow(incomplete_features)]
use serde::Deserialize;
use serde::de::DeserializeOwned;
use versioned::FlatDiffSer;
use serde_json_core::{from_slice, to_slice};

use httparse::{Request, EMPTY_HEADER};

pub trait EventHandler 
where Self::Event: Deserialize<'static> + DeserializeOwned  
{

    type Event;
    type Callbacks;

    fn handle_event(&mut self, event: Self::Event, sleep: &dyn FnMut(u32, Self::Callbacks)) -> Result<(), &'static str>;
}

impl<T: EventHandler + ::versioned::FlatDiffSer + Default, const N: usize> Default for EngineWrapper<T, N> 
where
    <T as EventHandler>::Callbacks: Copy,
{
    fn default() -> Self {
        EngineWrapper(T::default(), [None; N])
    }
}

#[derive(Copy, Clone)]
pub struct EngineWrapper<T: EventHandler + FlatDiffSer, const N: usize>(T, [Option<T::Callbacks>; N]);


impl<T, const N: usize> EngineWrapper<T, N> 
where
    T: EventHandler + FlatDiffSer + Clone,
    <T as EventHandler>::Callbacks: CallbackTrait    
{

    pub fn handle_event(&mut self, event: &[u8], result: &mut [u8], sleep: &dyn Fn(usize, usize)) -> Result<usize, &'static str> {
        let (event, _): (T::Event, usize) = from_slice(event).expect( "zzInvalid JSON event");

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
        let delta = ::versioned::FlatDiff(&self.0, &old_value);
        let len = to_slice(&delta, result).map_err(|_| "Failed to serialize delta")?;
        Ok(len)
    }

    // fn wakeup(&mut self, pos: usize) {
    //     if let Some(callback) = self.1[pos] {
    //         self.1[pos] = None;
    //         let mut args = &self.0;
    //         CallbackTrait::invoke(&callback, &mut args);
    //     }
    // }

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
    
                 // Manually constructing the HTTP response headers with a placeholder for Content-Length
                let header = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length:     \r\n\r\n";  // 5 spaces as placeholder
                response[..header.len()].copy_from_slice(header);

                // Process the event
                let response_len = self.handle_event(event_body, &mut response[header.len()..], sleep)?;

                // Update the Content-Length placeholder with the actual length of the response body
                let content_length_offset = header.len() - 8;
                itoa(response_len, &mut response[content_length_offset..content_length_offset + 5]);              
                
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

pub trait CallbackTrait {
    type T;

    fn invoke(&self, mut_val: &mut Self::T);
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

        impl crate::engine_traits::CallbackTrait for $enum_name {
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
                        $enum_name::$variant(crate::engine_traits::Closure::new(f, value))
                    }
                }
            )*
        }
    };
}


fn itoa(n: usize, buf: &mut [u8]) {
    let mut n = n;
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = (n % 10) as u8 + b'0';
        n /= 10;
    }
}