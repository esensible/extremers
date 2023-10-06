#![no_std]

mod race;
use race::Race;
mod engine;
use engine::Engine;

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

type SleepFn = extern "C" fn(usize, usize);

static mut ENGINE: Option<Engine<Race, 1>> = None;

#[no_mangle]
pub extern "C" fn init_engine() {
    unsafe {
        ENGINE = Some(Engine::default());
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
    let request_slice = unsafe { core::slice::from_raw_parts(request, request_len) };
    let response_slice = unsafe { core::slice::from_raw_parts_mut(response, response_len) };

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

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use serde_json_core::{from_slice, to_slice};

//     #[test]
//     fn test_handle_request() {
//         let mut engine = EngineWrapper::<Race, 1>::default();

//         let request_payload = b"POST /events HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: 39\r\n\r\n{\"timestamp\":32.4,\"event\":\"RaceFinish\"}\r\n";

//         let mut response = [0u8; 1024];  // Assuming this is sufficient space

//         let sleep_closure: &dyn Fn(usize, usize) = &|_time, _pos| {};  // No-op sleep function

//         match engine.handle_request(request_payload, &mut response, sleep_closure) {
//             Ok(()) => {
//                 // Parse and validate the response
//                 let response_str = core::str::from_utf8(&response).expect("Valid UTF-8");
//                 assert!(response_str.starts_with("HTTP/1.1 200 OK"));
//                 assert!(response_str.contains("Content-Type: application/json"));
//                 println!("response: {}", response_str);
//             },
//             Err(e) => panic!("Request handling failed: {}", e),
//         }
//     }

// }
