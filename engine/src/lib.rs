#![no_std]

mod race;
use race::Race;
mod core;
use core::RequestWrapper;

use ::core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

type SleepFn = extern "C" fn(usize, usize);

static mut ENGINE: Option<RequestWrapper<Race, 1>> = None;

#[no_mangle]
pub extern "C" fn init_engine() {
    unsafe {
        ENGINE = Some(RequestWrapper::default());
    }
}

#[no_mangle]
pub extern "C" fn handle_request_ffi(
    request: *const u8,
    request_len: usize,
    response: *mut u8,
    response_len: usize,
    sleep_fn: SleepFn,
) -> i32 {
    let request_slice = unsafe { ::core::slice::from_raw_parts(request, request_len) };
    let response_slice = unsafe { ::core::slice::from_raw_parts_mut(response, response_len) };

    let result = unsafe {
        if let Some(engine) = ENGINE.as_mut() {
            let sleep_closure: &dyn Fn(usize, usize) = &|time, pos| sleep_fn(time, pos);

            engine.handle_request(request_slice, response_slice, &sleep_closure)
        } else {
            Err("Engine not initialized")
        }
    };

    match result {
        Ok(_) => 0,
        Err(_) => -1,
    }
}
