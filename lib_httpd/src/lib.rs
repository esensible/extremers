#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
#[macro_use]
extern crate serde_json;

mod engine_httpd;

#[cfg(test)]
mod tests;

use crate::engine_httpd::EngineHttpd;
use engine::RaceEngine;

pub type RaceHttpd = EngineHttpd<RaceEngine>;
pub use engine_httpd::EngineHttpdTrait;
pub use engine_httpd::Response;
pub use engine_httpd::Response::Complete;

#[cfg(feature = "staticlib")]
mod ffi {
    use engine::Race;

    use crate::engine_httpd::{EngineHttpdTrait, Response};
    use crate::RaceHttpd;
    use core::ptr;

    use ::core::panic::PanicInfo;

    #[panic_handler]
    fn panic(_info: &PanicInfo) -> ! {
        loop {}
    }

    type SleepFn = extern "C" fn(usize, usize);

    fn engine() -> &'static mut RaceHttpd {
        static mut ENGINE: Option<RaceHttpd> = None;
        unsafe {
            if ENGINE.is_none() {
                ENGINE = Some(RaceHttpd::default());
            }
            ENGINE.as_mut().unwrap()
        }
    }

    #[no_mangle]
    pub extern "C" fn handle_request_ffi(
        request: *const u8,
        request_len: usize,
        response: *mut u8,
        response_len: &mut usize,
        update: *mut u8,
        update_len: &mut usize,
        extra: *mut *const u8,
        extra_len: &mut usize,
        sleep_fn: SleepFn,
    ) -> i32 {
        let request_slice = unsafe { ::core::slice::from_raw_parts(request, request_len) };
        let response_slice = unsafe { ::core::slice::from_raw_parts_mut(response, *response_len) };
        let update_slice = unsafe { ::core::slice::from_raw_parts_mut(update, *update_len) };

        let response = unsafe {
            let sleep_closure: &dyn Fn(usize, usize) = &|time, pos| sleep_fn(time, pos);
            let mut engine = engine();
            engine.handle_request(request_slice, response_slice, update_slice, &sleep_closure)
        };

        if response.is_err() {
            return -1;
        }

        let response = response.unwrap();

        match response {
            Response::Complete(r_len, up_len, ex) => {
                if let Some(r_len) = r_len {
                    *response_len = r_len;
                } else {
                    *response_len = 0;
                }
                if let Some(up_len) = up_len {
                    *update_len = up_len;
                } else {
                    *update_len = 0;
                }

                if let Some(ex) = ex {
                    if !extra.is_null() {
                        unsafe {
                            extra.write(ex.as_ptr());
                        }
                        *extra_len = ex.len();
                    } else {
                        *extra_len = 0;
                    }
                }
                0
            }
            Response::None => {
                *response_len = 0;
                *update_len = 0;
                *extra_len = 0;
                0
            }
            _ => -1,
        }
    }
}
