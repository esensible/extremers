// #![no_std]

static STATIC_FILES: [(&'static str, &'static [u8]); 5] = [
    ("a", &[0u8, 1, 2, 3, 4, 5]),
    ("a_file2", &[6u8, 7, 8, 9, 10, 11, 12, 13]),
    ("asdfdsf_file3", &[14u8, 15, 16, 17, 18, 19, 20, 21, 22]),
    ("hmm", &[23u8, 24, 25, 26, 27, 28, 29, 30, 31, 32]),
    ("file5", &[33u8, 34, 35, 36, 37]),
];

pub fn lookup(key: &str) -> Option<&'static [u8]> {
    for &(k, v) in STATIC_FILES.iter() {
        println!("{} {}", k, v.len());
        if k == key {
            println!("yay!");
            return Some(v);
        }
    }
    None
}
