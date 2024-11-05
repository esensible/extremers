
use embassy_rp::flash::{Flash, Async};
use embassy_rp::peripherals::FLASH;
use bytemuck::{Zeroable, Pod};


#[repr(C)]
#[derive(Copy, Clone, Default, Zeroable, Pod)]
struct GpsRecord {
    timestamp: u64,
    latitude: f64,
    longitude: f64,
    speed: f64,
    heading: f64,
}

// Calculate the number of GpsRecords that can fit in the segment
const SEGMENT_SIZE: u32 = 256;
const SEGMENT_RECORDS: u32 = (SEGMENT_SIZE - core::mem::size_of::<[u8; 16]>() as u32) / core::mem::size_of::<GpsRecord>() as u32;

const FLASH_OFFSET: u32 = 0x100000; // Offset from flash start
const FLASH_SIZE: usize = 0x100000;


#[repr(C)]
#[derive(Copy, Clone, Default, Pod, Zeroable)]
struct SegmentBuffer {
    hash: [u8; 16],
    records: [GpsRecord; SEGMENT_RECORDS as usize],
}


pub struct FlashLogger<'a> {
    flash: Flash<'a, FLASH, Async, FLASH_SIZE>,
    record_index: usize,
    segment_index: u32,
    segment_buffer: SegmentBuffer,
}

impl<'a> FlashLogger<'a> {
    pub fn new(flash: Flash<'a, FLASH, Async, FLASH_SIZE>) -> Self {
        Self {
            flash,
            record_index: 0,
            segment_index: 0,
            segment_buffer: SegmentBuffer::default(),
        }
    }

    pub fn write_record(
        &mut self,
        timestamp: Option<u64>,
        location: Option<(f64, f64)>,
        speed: Option<(f64, f64)>,
    )  {
        if let (Some(timestamp), Some(location), Some(speed)) = (timestamp, location, speed) {
            let gps_record = GpsRecord {
                timestamp,
                latitude: location.0,
                longitude: location.1,
                speed: speed.0,
                heading: speed.1,
            };

            // Populate the GpsRecord directly into the segment buffer
            self.segment_buffer.records[self.record_index] = gps_record;
            self.record_index += 1;

            if self.record_index == SEGMENT_RECORDS as usize {
                // We have collected enough records to write a segment to flash

                // Compute the MD5 hash of the GpsRecords in the segment buffer
                let records_bytes = bytemuck::cast_slice(&self.segment_buffer.records);
                let hash_result = md5::compute(records_bytes);
                self.segment_buffer.hash = hash_result.into();

                // Get a byte slice of the entire segment buffer
                let segment_bytes = bytemuck::bytes_of(&self.segment_buffer);
                let flash_address = FLASH_OFFSET + self.segment_index * SEGMENT_SIZE;

                let _ = self.flash.blocking_write(flash_address, segment_bytes);

                // Increment segment_index and reset record_index
                self.segment_index += 1;
                self.record_index = 0;
            }
        }
    }
}
