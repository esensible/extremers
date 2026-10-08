//! Minimal-copy NMEA (RMC) parser built on a small ring buffer.
//!
//! Bytes are read straight into the ring buffer and tokens are handed out as
//! `&str` slices of it; the only copying is moving a partial token to the
//! front of the buffer when more data is needed.
//!
//! Robustness rules:
//! - A run of bytes with no delimiter that fills the buffer (line noise,
//!   binary protocol traffic) is discarded and the parser resyncs on the next
//!   `$` sentence start.
//! - A sentence is only reported if its checksum matches.
//! - Only fixes with an "active" status and a valid mode are reported.

use embassy_time::{Duration, Timer};

use extreme_traits::{Fix, Velocity};

/// A valid GPS fix, as reported by [`next_update`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpsUpdate {
    /// Epoch milliseconds, if the receiver reported date and time.
    pub timestamp: Option<u64>,
    pub fix: Option<Fix>,
    pub velocity: Option<Velocity>,
}

#[derive(Debug, PartialEq)]
pub enum Status {
    Active,
    Void,
    Unknown,
}

#[derive(Debug, PartialEq)]
pub enum Mode {
    Autonomous,
    Differential,
    Estimated,
    NotValid,
    Unknown,
}

#[derive(Default, Debug)]
pub struct GNRMC {
    /// Milliseconds since midnight UTC
    pub utc_time: Option<u32>,
    pub status: Option<Status>,
    pub latitude: Option<f64>,
    pub ns_indicator: Option<char>,
    pub longitude: Option<f64>,
    pub ew_indicator: Option<char>,
    pub speed_over_ground: Option<f64>,
    pub course_over_ground: Option<f64>,
    /// Whole days since 1970-01-01
    pub date: Option<u32>,
    pub magnetic_variation: Option<f64>,
    pub ew_indicator_mag: Option<char>,
    pub mode: Option<Mode>,
}

impl GNRMC {
    /// True if the receiver reports a usable fix.
    pub fn is_valid_fix(&self) -> bool {
        self.status == Some(Status::Active)
            && !matches!(self.mode, Some(Mode::NotValid) | Some(Mode::Unknown))
    }
}

pub enum NMEAMessage {
    GNRMC(GNRMC),
}

#[allow(async_fn_in_trait)]
pub trait AsyncReader {
    /// Read at least one byte into `buf`, returning the number of bytes read.
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()>;
}

#[derive(Debug, PartialEq)]
pub enum TokenError {
    /// The underlying reader failed
    Read,
    /// The buffer filled up without a delimiter; its contents were discarded
    Overflow,
    /// The token was not valid UTF-8
    Utf8,
}

#[allow(async_fn_in_trait)]
pub trait Tokeniser {
    /// Returns the next token and the delimiter (`,`, `*`, `\n`, or `$` when a
    /// new sentence starts unexpectedly) that ended it.
    async fn next_token(&mut self) -> Result<(&str, u8), TokenError>;
}

pub struct RingBuffer<Reader, const N: usize>
where
    Reader: AsyncReader,
{
    reader: Reader,
    buf: [u8; N],
    /// Start of the next token
    read_ptr: usize,
    /// Position up to which the buffer has been searched for a delimiter
    scan_ptr: usize,
    /// End of valid data
    fill: usize,
}

impl<Reader, const N: usize> RingBuffer<Reader, N>
where
    Reader: AsyncReader,
{
    pub fn new(reader: Reader) -> Self {
        Self {
            reader,
            buf: [0; N],
            read_ptr: 0,
            scan_ptr: 0,
            fill: 0,
        }
    }
}

impl<Reader, const N: usize> Tokeniser for RingBuffer<Reader, N>
where
    Reader: AsyncReader,
{
    async fn next_token(&mut self) -> Result<(&str, u8), TokenError> {
        loop {
            // `$` also ends a token (a truncated sentence followed by a new
            // one), but it is kept as the start of the next token
            let read_ptr = self.read_ptr;
            if let Some(end) = (self.scan_ptr..self.fill).find(|&i| match self.buf[i] {
                b',' | b'*' | b'\n' => true,
                b'$' => i != read_ptr,
                _ => false,
            }) {
                let start = self.read_ptr;
                self.read_ptr = if self.buf[end] == b'$' { end } else { end + 1 };
                self.scan_ptr = end + 1;

                let token =
                    core::str::from_utf8(&self.buf[start..end]).map_err(|_| TokenError::Utf8)?;
                return Ok((token, self.buf[end]));
            }
            self.scan_ptr = self.fill;

            // Need more data. Move the partial token to the front first.
            if self.read_ptr > 0 {
                self.buf.copy_within(self.read_ptr..self.fill, 0);
                self.fill -= self.read_ptr;
                self.scan_ptr = self.fill;
                self.read_ptr = 0;
            }

            if self.fill == N {
                // No delimiter in a whole buffer: this isn't NMEA, drop it.
                self.fill = 0;
                self.scan_ptr = 0;
                return Err(TokenError::Overflow);
            }

            let n = self
                .reader
                .read(&mut self.buf[self.fill..])
                .await
                .map_err(|_| TokenError::Read)?;
            self.fill += n;
        }
    }
}

fn xor_bytes(s: &str) -> u8 {
    s.bytes().fold(0, |acc, b| acc ^ b)
}

fn parse_hex_u8(s: &str) -> Option<u8> {
    if s.len() != 2 {
        return None;
    }
    u8::from_str_radix(s, 16).ok()
}

/// `hhmmss.sss` -> milliseconds since midnight
fn parse_utc_time(token: &str) -> Option<u32> {
    let hours: u32 = token.get(0..2)?.parse().ok()?;
    let minutes: u32 = token.get(2..4)?.parse().ok()?;
    let seconds: u32 = token.get(4..6)?.parse().ok()?;
    if hours > 23 || minutes > 59 || seconds > 60 {
        return None;
    }

    // fractional seconds, any number of digits
    let mut milliseconds = 0;
    if let Some(fraction) = token.get(7..) {
        let mut scale = 100;
        for c in fraction.bytes().take(3) {
            if !c.is_ascii_digit() {
                return None;
            }
            milliseconds += (c - b'0') as u32 * scale;
            scale /= 10;
        }
    }

    Some(hours * 60 * 60_000 + minutes * 60_000 + seconds * 1_000 + milliseconds)
}

/// `ddmm.mmmm` (or `dddmm.mmmm` for longitude) -> decimal degrees
fn parse_angle(token: &str, degree_digits: usize) -> Option<f64> {
    if token.len() < degree_digits + 2 {
        return None;
    }
    let degrees = token.get(0..degree_digits)?.parse::<f64>().ok()?;
    let minutes = token.get(degree_digits..)?.parse::<f64>().ok()?;
    Some(degrees + minutes / 60.0)
}

/// GPS date (`ddmmyy`) -> whole days since 1970-01-01. Valid for 2000-2099.
fn date_to_epoch(date_str: &str) -> Option<u32> {
    if date_str.len() != 6 {
        return None;
    }

    static MONTH_DAY: [u16; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

    let day: u32 = date_str.get(0..2)?.parse().ok()?;
    let month: u32 = date_str.get(2..4)?.parse().ok()?;
    let year: u32 = date_str.get(4..6)?.parse().ok()?;
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) {
        return None;
    }

    let days_in_months = if year % 4 == 0 && month > 2 {
        (MONTH_DAY[(month - 1) as usize] + 1) as u32
    } else {
        (MONTH_DAY[(month - 1) as usize]) as u32
    };

    // Calculate the number of days for each year assuming all years have 365 days
    let days_in_years = (year + 2000 - 1970) * 365;
    // Calculate additional days from leap years since 1972
    let leap_days = ((year + 2000 - 1) - 1972) / 4 + 1;

    Some(days_in_years + leap_days + days_in_months + day - 1)
}

fn parse_field(gnrmc: &mut GNRMC, field: usize, token: &str) {
    match field {
        0 => gnrmc.utc_time = parse_utc_time(token),
        1 => {
            gnrmc.status = Some(match token {
                "A" => Status::Active,
                "V" => Status::Void,
                _ => Status::Unknown,
            })
        }
        2 => gnrmc.latitude = parse_angle(token, 2),
        3 => gnrmc.ns_indicator = token.chars().next(),
        4 => gnrmc.longitude = parse_angle(token, 3),
        5 => gnrmc.ew_indicator = token.chars().next(),
        6 => gnrmc.speed_over_ground = token.parse::<f64>().ok(),
        7 => gnrmc.course_over_ground = token.parse::<f64>().ok(),
        8 => gnrmc.date = date_to_epoch(token),
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
        // NMEA 4.1+ adds a navigational status field; ignore it and anything else
        _ => {}
    }
}

/// Returns the next RMC sentence with a valid checksum, or `Err` if the
/// underlying reader failed.
pub async fn next_message<T>(tokeniser: &mut T) -> Result<NMEAMessage, TokenError>
where
    T: Tokeniser,
{
    // The RMC sentence being parsed, if any
    let mut message: Option<GNRMC> = None;
    let mut field = 0;
    // XOR of everything between the `$` and the `*`
    let mut checksum = 0u8;
    // The previous token ended with `*`, so this one is the checksum
    let mut expect_checksum = false;

    loop {
        let (token, delimiter) = match tokeniser.next_token().await {
            Ok(t) => t,
            Err(TokenError::Read) => return Err(TokenError::Read),
            Err(e) => {
                debug!("NMEA resync: {:?}", crate::fmt::Dbg(&e));
                message = None;
                continue;
            }
        };

        // Any `$` starts a new sentence, even mid-sentence (dropped bytes)
        if token.starts_with('$') {
            message = None;
            if delimiter == b',' && (token == "$GPRMC" || token == "$GNRMC") {
                message = Some(GNRMC::default());
                field = 0;
                checksum = xor_bytes(&token[1..]);
                expect_checksum = false;
            }
            continue;
        }

        let Some(gnrmc) = message.as_mut() else {
            continue;
        };

        if delimiter == b'$' {
            // sentence was cut short by the start of another one
            message = None;
            continue;
        }

        if expect_checksum {
            let received = parse_hex_u8(token.trim_end_matches('\r'));
            let message = message.take();
            if delimiter == b'\n' && received == Some(checksum) {
                if let Some(gnrmc) = message {
                    return Ok(NMEAMessage::GNRMC(gnrmc));
                }
            } else {
                debug!("NMEA checksum mismatch");
            }
            continue;
        }

        checksum ^= b',' ^ xor_bytes(token);
        parse_field(gnrmc, field, token);
        field += 1;

        match delimiter {
            b'*' => expect_checksum = true,
            // a sentence without a checksum can't be trusted
            b'\n' => message = None,
            _ => {}
        }
    }
}

/// Waits for the next valid fix.
pub async fn next_update<T>(tokeniser: &mut T) -> GpsUpdate
where
    T: Tokeniser,
{
    loop {
        let gnrmc = match next_message(tokeniser).await {
            Ok(NMEAMessage::GNRMC(gnrmc)) => gnrmc,
            Err(_) => {
                // reader failed; back off rather than spin
                Timer::after(Duration::from_millis(100)).await;
                continue;
            }
        };

        if !gnrmc.is_valid_fix() {
            continue;
        }

        let timestamp = if let (Some(time), Some(date)) = (gnrmc.utc_time, gnrmc.date) {
            Some(time as u64 + date as u64 * 24 * 60 * 60_000)
        } else {
            None
        };

        let fix = if let (Some(latitude), Some(ns), Some(longitude), Some(ew)) = (
            gnrmc.latitude,
            gnrmc.ns_indicator,
            gnrmc.longitude,
            gnrmc.ew_indicator,
        ) {
            Some(Fix {
                lat: if ns == 'S' { -latitude } else { latitude },
                lon: if ew == 'W' { -longitude } else { longitude },
            })
        } else {
            None
        };

        let velocity = if let (Some(speed), Some(heading)) =
            (gnrmc.speed_over_ground, gnrmc.course_over_ground)
        {
            Some(Velocity { speed, heading })
        } else {
            None
        };

        return GpsUpdate {
            timestamp,
            fix,
            velocity,
        };
    }
}

/// Longest PMTK command [`pmtk_sentence`] accepts: room is needed for `$`,
/// `*`, two checksum digits and CR LF.
pub const MAX_PMTK_COMMAND: usize = 64 - 6;

/// Frames a PMTK (MediaTek GPS configuration) command, given without the
/// leading `$`, as a complete sentence: `$<command>*<checksum>\r\n`.
///
/// # Panics
/// If `command` is longer than [`MAX_PMTK_COMMAND`].
pub fn pmtk_sentence<'o>(command: &str, out: &'o mut [u8; 64]) -> &'o [u8] {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let command = command.as_bytes();
    assert!(command.len() <= MAX_PMTK_COMMAND, "PMTK command too long");
    let checksum = command.iter().fold(0u8, |acc, b| acc ^ b);

    let end = command.len() + 6;
    out[0] = b'$';
    out[1..=command.len()].copy_from_slice(command);
    out[end - 5..end].copy_from_slice(&[
        b'*',
        HEX[(checksum >> 4) as usize],
        HEX[(checksum & 0xF) as usize],
        b'\r',
        b'\n',
    ]);
    &out[..end]
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use embassy_futures::block_on;
    use std::vec::Vec;

    /// Hands out the input in fixed-size chunks, then fails.
    struct ChunkReader<'a> {
        data: &'a [u8],
        chunk: usize,
    }

    impl AsyncReader for ChunkReader<'_> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
            if self.data.is_empty() {
                return Err(());
            }
            let n = self.chunk.min(buf.len()).min(self.data.len());
            buf[..n].copy_from_slice(&self.data[..n]);
            self.data = &self.data[n..];
            Ok(n)
        }
    }

    const GOOD: &str = "$GNRMC,123519.50,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W,A*";

    fn sentence(body: &str) -> std::string::String {
        let cs = xor_bytes(&body[1..body.len() - 1]);
        std::format!("{body}{cs:02X}\r\n")
    }

    fn messages(input: &[u8], chunk: usize) -> Vec<GNRMC> {
        let mut rb = RingBuffer::<_, 32>::new(ChunkReader { data: input, chunk });
        let mut out = Vec::new();
        while let Ok(NMEAMessage::GNRMC(m)) = block_on(next_message(&mut rb)) {
            out.push(m);
        }
        out
    }

    #[test]
    fn parses_valid_sentence_any_chunking() {
        let input = sentence(GOOD);
        for chunk in [1, 3, 7, 32, 200] {
            let msgs = messages(input.as_bytes(), chunk);
            assert_eq!(msgs.len(), 1, "chunk {chunk}");
            let m = &msgs[0];
            assert_eq!(m.utc_time, Some(((12 * 60 + 35) * 60 + 19) * 1000 + 500));
            assert_eq!(m.status, Some(Status::Active));
            assert!((m.latitude.unwrap() - (48.0 + 7.038 / 60.0)).abs() < 1e-9);
            assert!((m.longitude.unwrap() - (11.0 + 31.0 / 60.0)).abs() < 1e-9);
            assert_eq!(m.speed_over_ground, Some(22.4));
            assert_eq!(m.course_over_ground, Some(84.4));
            assert!(m.is_valid_fix());
        }
    }

    #[test]
    fn consecutive_sentences_are_not_dropped() {
        let one = sentence(GOOD);
        let input = std::format!("{one}{one}{one}");
        assert_eq!(messages(input.as_bytes(), 5).len(), 3);
    }

    #[test]
    fn nmea_4_1_nav_status_field() {
        let input =
            sentence("$GNRMC,123519.00,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W,A,V*");
        assert_eq!(messages(input.as_bytes(), 4).len(), 1);
    }

    #[test]
    fn bad_checksum_is_rejected() {
        let mut input = sentence(GOOD).into_bytes();
        let n = input.len();
        input[n - 3] = if input[n - 3] == b'0' { b'1' } else { b'0' };
        let good = sentence(GOOD);
        input.extend_from_slice(good.as_bytes());
        // only the second, intact, sentence comes through
        assert_eq!(messages(&input, 6).len(), 1);
    }

    #[test]
    fn missing_checksum_is_rejected() {
        let input = "$GNRMC,123519.00,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W,A\r\n";
        assert_eq!(messages(input.as_bytes(), 6).len(), 0);
    }

    #[test]
    fn binary_noise_and_overflow_resync() {
        let mut input: Vec<u8> = Vec::new();
        // binary (UBX-like) traffic, including invalid UTF-8 and no delimiters
        input.extend((0..200u32).map(|i| (i * 37 % 251) as u8 | 0x80));
        // a printable run longer than the buffer
        input.extend(std::iter::repeat_n(b'x', 100));
        // a truncated sentence
        input.extend_from_slice(b"$GNRMC,1235");
        input.extend_from_slice(sentence(GOOD).as_bytes());
        assert_eq!(messages(&input, 9).len(), 1);
    }

    #[test]
    fn multibyte_utf8_does_not_panic() {
        let input =
            sentence("$GNRMC,é€é€é€,A,48€7.038,N,0€131.000,E,022.4,084.4,23€394,003.1,W,A*");
        let msgs = messages(input.as_bytes(), 5);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].utc_time, None);
        assert_eq!(msgs[0].date, None);
    }

    #[test]
    fn void_fix_is_not_valid() {
        let input = sentence("$GNRMC,123519.00,V,,,,,,,230394,,,N*");
        let msgs = messages(input.as_bytes(), 8);
        assert_eq!(msgs.len(), 1);
        assert!(!msgs[0].is_valid_fix());
    }

    #[test]
    fn next_update_signs_and_converts() {
        let input =
            sentence("$GNRMC,123519.50,A,4807.038,S,01131.000,W,022.4,084.4,230326,003.1,W,A*");
        let mut rb = RingBuffer::<_, 32>::new(ChunkReader {
            data: input.as_bytes(),
            chunk: 7,
        });
        let update = block_on(next_update(&mut rb));
        // 2026-03-23 12:35:19.5 UTC
        assert_eq!(update.timestamp, Some(1_774_269_319_500));
        let fix = update.fix.unwrap();
        assert!((fix.lat + (48.0 + 7.038 / 60.0)).abs() < 1e-9);
        assert!((fix.lon + (11.0 + 31.0 / 60.0)).abs() < 1e-9);
        assert_eq!(
            update.velocity,
            Some(Velocity {
                speed: 22.4,
                heading: 84.4
            })
        );
    }

    #[test]
    fn pmtk_sentence_checksum() {
        let mut out = [0u8; 64];
        // published sentences (MTK command reference, Adafruit GPS library)
        assert_eq!(
            pmtk_sentence("PMTK220,1000", &mut out),
            b"$PMTK220,1000*1F\r\n"
        );
        assert_eq!(
            pmtk_sentence("PMTK314,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0", &mut out),
            b"$PMTK314,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0*29\r\n"
        );
        // the commands tgt-pico sends
        assert_eq!(
            pmtk_sentence("PMTK314,0,1,0,0,0,0,0,0", &mut out),
            b"$PMTK314,0,1,0,0,0,0,0,0*35\r\n"
        );
        assert_eq!(pmtk_sentence("PMTK313,1", &mut out), b"$PMTK313,1*2E\r\n");
        assert_eq!(pmtk_sentence("PMTK319,1", &mut out), b"$PMTK319,1*24\r\n");
    }

    #[test]
    #[should_panic]
    fn pmtk_sentence_rejects_long_commands() {
        let mut out = [0u8; 64];
        let _ = pmtk_sentence(&"P".repeat(MAX_PMTK_COMMAND + 1), &mut out);
    }

    #[test]
    fn bad_dates_are_rejected() {
        assert_eq!(date_to_epoch("010170"), Some(36525)); // 2070-01-01
        assert_eq!(date_to_epoch("000126"), None);
        assert_eq!(date_to_epoch("010026"), None);
        assert_eq!(date_to_epoch("011326"), None);
        // 2026-10-01
        assert_eq!(date_to_epoch("011026"), Some(20727));
    }
}
