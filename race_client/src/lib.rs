#![no_std]

include!(concat!(env!("OUT_DIR"), "/static_files.rs"));

pub fn lookup(key: &str) -> Option<&'static [u8]> {
    for &(k, v) in STATIC_FILES.iter() {
        if k == key {
            return Some(v);
        }
    }
    None
}

// use ::core::panic::PanicInfo;

// #[panic_handler]
// fn panic(_info: &PanicInfo) -> ! {
//     loop {}
// }
