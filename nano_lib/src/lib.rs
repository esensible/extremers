#![no_std]
// extern crate alloc;

use cty::{c_char, c_void};
use core::slice;
use core::str;
use core::ptr::copy_nonoverlapping;
use httparse::{Request, EMPTY_HEADER};

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn parse_http_request(
    request: *const c_char,
    response_buf: *mut c_char,
    buf_size: usize,
) -> i32 {
    if request.is_null() || response_buf.is_null() {
        return -1;
    }

    let c_str_len = unsafe {
        let mut len = 0;
        while *request.add(len) != 0 {
            len += 1;
        }
        len
    };

    let c_str = unsafe { slice::from_raw_parts(request as *const u8, c_str_len) };
    let request_str = match str::from_utf8(c_str) {
        Ok(s) => s,
        Err(_) => return -2,
    };

    let mut headers = [EMPTY_HEADER; 16];
    let mut req = Request::new(&mut headers);

    if let Ok(_) = req.parse(request_str.as_bytes()) {
        if let Some(path) = req.path {
            let response_prefix = b"HTTP/1.1 200 OK\r\n\r\nYou requested ";
            let response_len = response_prefix.len() + path.len();
            
            if response_len > buf_size {
                return -3;
            }

            unsafe {
                copy_nonoverlapping(
                    response_prefix.as_ptr(),
                    response_buf as *mut u8,
                    response_prefix.len(),
                );
                copy_nonoverlapping(
                    path.as_ptr(),
                    response_buf.add(response_prefix.len()) as *mut u8,
                    path.len(),
                );
            }

            return response_len as i32;
        }
    }

    0
}
