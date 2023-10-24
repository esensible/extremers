use core::sync::atomic::AtomicU32;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::Duration;

pub const PORT: u16 = 80;
pub const TIMEOUT: Duration = Duration::from_secs(10);
pub const RX_BUF_SIZE: usize = 2048;
pub const TX_BUF_SIZE: usize = 2048;
pub const READ_BUF_SIZE: usize = 4096;
pub const RESPONSE_BUF_SIZE: usize = 4096;
pub const UPDATE_BUF_SIZE: usize = 4096;

pub const MAX_SOCKETS: usize = 4;

// offset between tick timer and GPS time in ms
// this isn't super safe, but realistically, the MSB will only change every 50 days
// and out of sync operations won't end the world
pub static mut OFFSET_MSB: AtomicU32 = AtomicU32::new(0);
pub static mut OFFSET_LSB: AtomicU32 = AtomicU32::new(0);

#[derive(Clone)]
pub struct UpdateMessage(pub [u8; UPDATE_BUF_SIZE], pub usize);

impl Default for UpdateMessage {
    fn default() -> Self {
        Self([0; UPDATE_BUF_SIZE], 0)
    }
}

// TODO: fix these args, given we're misusing them
pub static UPDATES_BUS: PubSubChannel<
    ThreadModeRawMutex,
    UpdateMessage,
    1,
    MAX_SOCKETS,
    { MAX_SOCKETS + 1 },
> = PubSubChannel::new();

#[derive(Default, Clone)]
pub struct SleepMessage {
    pub time: usize,
    pub callback: usize,
}

pub static SLEEP_BUS: PubSubChannel<ThreadModeRawMutex, SleepMessage, 1, MAX_SOCKETS, MAX_SOCKETS> =
    PubSubChannel::new();
