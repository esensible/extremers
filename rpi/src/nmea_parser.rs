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
    pub utc_time: Option<u32>,
    pub status: Option<Status>,
    pub latitude: Option<f64>,
    pub ns_indicator: Option<char>,
    pub longitude: Option<f64>,
    pub ew_indicator: Option<char>,
    pub speed_over_ground: Option<f64>,
    pub course_over_ground: Option<f64>,
    pub date: Option<u32>,
    pub magnetic_variation: Option<f64>,
    pub ew_indicator_mag: Option<char>,
    pub mode: Option<Mode>,
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
                    // log::info!("RX error");
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

fn date_to_epoch(date_str: &str) -> Option<u32> {
    if date_str.len() != 6 {
        return None;
    }

    static MONTH_DAY: [u16; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

    let day: u32 = date_str[0..2].parse().ok()?;
    let month: u32 = date_str[2..4].parse().ok()?;
    let year: u32 = date_str[4..6].parse().ok()?;

    // Calculate the number of days for each month assuming all months have 30 days
    // This is a simplification and will not yield accurate results
    let days_in_months = if year % 4 == 0 && month > 2 {
        (MONTH_DAY[(month - 1) as usize] + 1) as u32
    } else {
        (MONTH_DAY[(month - 1) as usize]) as u32
    };

    // Calculate the number of days for each year assuming all years have 365 days
    let days_in_years = (year + 2000 - 1970) * 365;

    // Calculate the number of leap years since 1972
    let leap_years = ((year + 2000 - 1) - 1972) / 4 + 1;

    // Now add all the days together and convert to seconds
    let total_days = days_in_years + days_in_months + day + leap_years - 1;

    Some(total_days)
}

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
                return None;
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
                        0 => {
                            gnrmc.utc_time = if token.len() >= 9 {
                                let hours = token[0..2].parse::<u32>().ok()?;
                                let minutes = token[2..4].parse::<u32>().ok()?;
                                let seconds = token[4..6].parse::<u32>().ok()?;
                                let milliseconds = token[7..].parse::<u32>().ok()?;

                                Some(
                                    hours * 60 * 60_000
                                        + minutes * 60_000
                                        + seconds * 1_000
                                        + milliseconds,
                                )
                            } else {
                                None
                            }
                        }

                        1 => {
                            gnrmc.status = Some(match token {
                                "A" => Status::Active,
                                "V" => Status::Void,
                                _ => Status::Unknown,
                            })
                        }
                        2 => {
                            gnrmc.latitude = if token.len() >= 7 {
                                let degrees = token[0..2].parse::<f64>().ok()?;
                                let minutes = token[2..].parse::<f64>().ok()?;
                                Some(degrees + minutes / 60.0)
                            } else {
                                None
                            }
                        }
                        3 => gnrmc.ns_indicator = token.chars().next(),
                        4 => {
                            gnrmc.longitude = if token.len() >= 7 {
                                let degrees = token[0..3].parse::<f64>().ok()?;
                                let minutes = token[3..].parse::<f64>().ok()?;
                                Some(degrees + minutes / 60.0)
                            } else {
                                None
                            }
                        }
                        5 => gnrmc.ew_indicator = token.chars().next(),
                        6 => {
                            // log::info!("speed: {}", token);
                            gnrmc.speed_over_ground = token.parse::<f64>().ok()
                        }
                        7 => {
                            // log::info!("course: {}", token);
                            gnrmc.course_over_ground = token.parse::<f64>().ok()
                        }
                        8 => gnrmc.date = date_to_epoch(&token),
                        9 => gnrmc.magnetic_variation = token.parse::<f64>().ok(),
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
