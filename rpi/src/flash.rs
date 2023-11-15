use littlefs2::io::Result;
use littlefs2::driver::Storage;
use embassy_rp::rom_data::{flash_range_erase, flash_range_program};
use core::ptr;
use generic_array;

pub struct PicoFlash;

impl Storage for PicoFlash {
    const READ_SIZE: usize = 1; // Adjust according to actual capabilities
    const WRITE_SIZE: usize = 256; // Typically 256 bytes for flash memory
    const BLOCK_SIZE: usize = 4096; // Typically 4KB for erase operations
    const BLOCK_COUNT: usize = 512; // Assuming 2MB of flash (2MB / 4KB per block)
    const BLOCK_CYCLES: isize = -1; // Disabling wear-leveling

    type CACHE_SIZE = generic_array::typenum::U256; // Example cache size
    type LOOKAHEAD_SIZE = generic_array::typenum::U256; // Example lookahead size

    fn read(&mut self, off: usize, buf: &mut [u8]) -> Result<usize> {
        // Implement the read operation. For the RP2040, this might be a direct memory read,
        // since the flash is memory-mapped. Ensure `off` and `buf.len()` are valid.
        // This is a simplified example and does not handle unaligned reads.
        unsafe {
            ptr::copy_nonoverlapping((0x10000000 + off) as *const u8, buf.as_mut_ptr(), buf.len());
        }
        Ok(buf.len())
    }

    fn write(&mut self, off: usize, data: &[u8]) -> Result<usize> {
        // Implement the write operation using flash_range_program
        unsafe {
            flash_range_program(off as u32, data.as_ptr(), data.len());
        }
        Ok(data.len())
    }

    fn erase(&mut self, off: usize, len: usize) -> Result<usize> {
        // Implement the erase operation using flash_range_erase
        // For simplicity, this example does not handle partial block erases.
        unsafe {
            flash_range_erase(off as u32, len, 4096, 0xD8); // Using 0xD8 as an example command
        }
        Ok(len)
    }
}


