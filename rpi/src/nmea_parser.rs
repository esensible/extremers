use embassy_rp::peripherals::UART1;
use embassy_rp::uart::{Async, UartRx};

#[derive(Debug)]
pub enum Status {
    Active,
    Void,
    Unknown,
}

#[derive(Debug)]
pub enum Mode {
    Autonomous,
    Differential,
    Estimated,
    NotValid,
    Unknown,
}

#[derive(Default, Debug)]
pub struct GNRMC {
    utc_time: Option<f32>,
    status: Option<Status>,
    latitude: Option<f32>,
    ns_indicator: Option<char>,
    longitude: Option<f32>,
    ew_indicator: Option<char>,
    speed_over_ground: Option<f32>,
    course_over_ground: Option<u16>,
    date: Option<u64>,
    magnetic_variation: Option<f32>,
    ew_indicator_mag: Option<char>,
    mode: Option<Mode>,
}

pub enum NMEAMessage {
    GNRMC(GNRMC),
    Unknown,
}

pub struct RingBuffer<const N: usize> {
    reader: UartRx<'static, UART1, Async>,
    buf: [u8; N],
    read_ptr: usize,
}

impl<const N: usize> RingBuffer<N> {
    pub fn new(reader: UartRx<'static, UART1, Async>) -> Self {
        Self {
            reader,
            buf: [0; N],
            read_ptr: N,
        }
    }

    pub async fn next_token(&mut self) -> Option<&str> {
        let mut cursor = self.read_ptr;
        let old_ptr = self.read_ptr;

        let next_comma = loop {
            if cursor == N {
                let partial_len = N - self.read_ptr;
                if partial_len > 0 {
                    self.buf.copy_within(self.read_ptr.., 0);
                }
                cursor = partial_len;
                let result = self.reader.read(&mut self.buf[cursor..]).await;
                if result.is_err() {
                    log::info!("RX error");
                    return None;
                }
                self.read_ptr = 0;
            }
            if cursor > 0
                && (self.buf[cursor] == b','
                    || self.buf[cursor] == b'\n'
                    || self.buf[cursor] == b'*')
            {
                self.read_ptr = cursor + 1;
                break cursor;
            }
            cursor += 1;
        };

        let result = if next_comma >= old_ptr {
            core::str::from_utf8(&self.buf[old_ptr..next_comma])
        } else {
            core::str::from_utf8(&self.buf[..next_comma])
        };
        match result {
            Ok(s) => Some(s),
            Err(e) => {
                log::info!("UTF8 error: {:?}", e);
                None
            }
        }
    }
}

pub struct NMEAParser<const N: usize>(RingBuffer<N>);

impl<const N: usize> NMEAParser<N> {
    pub fn new(rx: UartRx<'static, UART1, Async>) -> Self {
        Self(RingBuffer::new(rx))
    }

    pub async fn next_token(&mut self) -> Option<NMEAMessage> {
        let mut message = NMEAMessage::Unknown;
        let mut field = -1;

        loop {
            let token = self.0.next_token().await;
            if token.is_none() {
                continue;
            }
            let token = token.unwrap();
            match &mut message {
                NMEAMessage::Unknown => {
                    if token == "$GNRMC" {
                        message = NMEAMessage::GNRMC(GNRMC::default());
                        field = -1;
                    } else if token.starts_with("$") {
                        // log::info!("{}", token);
                    }
                }

                NMEAMessage::GNRMC(gnrmc) => {
                    match field {
                        0 => gnrmc.utc_time = token.parse::<f32>().ok(),
                        1 => {
                            gnrmc.status = Some(match token {
                                "A" => Status::Active,
                                "V" => Status::Void,
                                _ => Status::Unknown,
                            })
                        }
                        2 => gnrmc.latitude = token.parse::<f32>().ok(),
                        3 => gnrmc.ns_indicator = token.chars().next(),
                        4 => gnrmc.longitude = token.parse::<f32>().ok(),
                        5 => gnrmc.ew_indicator = token.chars().next(),
                        6 => gnrmc.speed_over_ground = token.parse::<f32>().ok(),
                        7 => gnrmc.course_over_ground = token.parse::<u16>().ok(),
                        8 => gnrmc.date = token.parse::<u64>().ok(),
                        9 => gnrmc.magnetic_variation = token.parse::<f32>().ok(),
                        10 => gnrmc.ew_indicator_mag = token.chars().next(),
                        11 => {
                            gnrmc.mode = Some(match token {
                                "A" => Mode::Autonomous,
                                "D" => Mode::Differential,
                                "E" => Mode::Estimated,
                                "N" => Mode::NotValid,
                                _ => Mode::Unknown,
                            })
                        }
                        12 => {
                            // checksum
                        }
                        _ => {
                            return Some(message);
                        }
                    }
                }
            }
            field += 1;
        }
    }
}
