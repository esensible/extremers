use httparse::{Request, EMPTY_HEADER};

use engine::{EventEngineTrait, SerdeEngine, SerdeEngineTrait, SleepFn};
use race_client::lookup;
use serde::Serialize;
use serde_json_core::to_slice;

const TIMESTAMP_TOLERANCE_MS: i64 = 50;
const TIMEZONE_OFFSET: i64 = (10 * 60 + 30) * 60; // ACDT (s)

#[derive(PartialEq)]
#[cfg_attr(feature = "std", derive(Debug))]
pub enum Response {
    // response length (excludes additional), update length, additional response
    Complete(Option<usize>, Option<usize>, Option<&'static [u8]>),
    Partial(usize),
    None,
}

pub trait EngineHttpdTrait {
    fn handle_request(
        &mut self,
        timestamp: u64,
        request: &[u8],
        response: &mut [u8],
        updates: &mut [u8],
        sleep: &SleepFn,
    ) -> Result<Response, usize>;

    fn update_location(
        &mut self,
        location: Option<(f32, f32)>,
        heading: Option<(f32, f32)>,
        updates: &mut [u8],
    ) -> Option<usize>;

    fn handle_sleep(&mut self, updates: &mut [u8], callback: usize) -> Option<usize>;
}

const HTTP_VERSION: &[u8] = b"HTTP/1.1 ";
const OK: &[u8] = b"200 OK\r\n";
const BAD_REQUEST: &[u8] = b"400 Bad Request\r\n";
const NOT_FOUND: &[u8] = b"404 Not Found\r\n";
const SERVER_ERROR: &[u8] = b"500 Internal Server Error\r\n";
const CONTENT_TYPE: &[u8] = b"Content-Type: ";
const APP_JSON: &[u8] = b"application/json\r\n";
// const TEXT_HTML: &[u8] = b"text/html\r\n";

const CONTENT_LENGTH: &[u8] = b"Content-Length: ";

#[derive(Default)]
pub struct EngineHttpd<T: EventEngineTrait>(SerdeEngine<T>);

impl<T: EventEngineTrait> EngineHttpdTrait for EngineHttpd<T> {
    fn handle_request(
        &mut self,
        timestamp: u64,
        request: &[u8],
        response: &mut [u8],
        updates: &mut [u8],
        sleep: &SleepFn,
    ) -> Result<Response, usize> {
        // Buffer to hold HTTP request headers
        let mut headers = [EMPTY_HEADER; 16];

        // Parsing the request
        let mut req = Request::new(&mut headers);
        let status = req.parse(request);

        // Check if the headers were fully parsed
        let offset = match status {
            Err(_) => {
                return Err(respond(
                    response,
                    BAD_REQUEST,
                    Some(b"Error parsing request"),
                ));
            }
            Ok(status) => {
                if let httparse::Status::Complete(offset) = status {
                    offset
                } else {
                    return Err(respond(response, SERVER_ERROR, Some(b"Incomplete request")));
                }
            }
        };

        match (req.method, req.path) {
            (Some("POST"), Some("/events")) => {
                let content_length: usize = req
                    .headers
                    .iter()
                    .filter(|header| header.name.eq_ignore_ascii_case("Content-Length"))
                    .filter_map(|header| core::str::from_utf8(header.value).ok()?.parse().ok())
                    .next()
                    .ok_or(respond(
                        response,
                        BAD_REQUEST,
                        Some(b"Content-Length not found or invalid"),
                    ))?;

                if offset + content_length > request.len() {
                    return Ok(Response::Partial(request.len() - (offset + content_length)));
                }

                // Assuming the body starts right after the headers
                let event_body = &request[offset..offset + content_length];

                let (update_offs, content_len_offs) = fill_header(updates, OK, Some(APP_JSON));

                let update_len = self
                    .0
                    .handle_event(event_body, &mut updates[update_offs..], sleep)
                    .map_err(|msg| respond(response, BAD_REQUEST, Some(msg.as_bytes())))?;

                if let Some(update_len) = update_len {
                    itoa(
                        update_len,
                        &mut updates[content_len_offs..content_len_offs + 5],
                    );
                    Ok(Response::Complete(
                        Some(respond(response, OK, None)),
                        Some(update_offs + update_len),
                        None,
                    ))
                } else {
                    Ok(Response::Complete(
                        Some(respond(response, OK, None)),
                        None,
                        None,
                    ))
                }
            }
            (Some("GET"), Some(path)) if path.starts_with("/updates?") => {
                let query_args = parse_query(&path["/updates?".len()..]);

                if let Err(err) = query_args {
                    return Err(respond(response, BAD_REQUEST, Some(err)));
                }
                let (query_timestamp, cnt) = query_args.unwrap();

                match (query_timestamp, cnt) {
                    (Some(query_timestamp), Some(cnt)) => {
                        let (response_offs, content_len_offs) =
                            fill_header(response, OK, Some(APP_JSON));

                        let time_offset: i64 = if timestamp >= query_timestamp {
                            (timestamp.wrapping_sub(query_timestamp)) as i64
                        } else {
                            -(query_timestamp.wrapping_sub(timestamp) as i64)
                        };

                        let update_len = if query_timestamp != 0
                            && time_offset.abs() > TIMESTAMP_TOLERANCE_MS
                        {
                            #[derive(Serialize)]
                            struct OffsetResponse {
                                #[serde(rename = "tzOffset")]
                                tz_offset: i64,
                                offset: i64,
                                cnt: i8,
                            }
                            let offset_response = OffsetResponse {
                                offset: time_offset,
                                tz_offset: TIMEZONE_OFFSET, // seconds
                                cnt: -1,
                            };
                            let len = to_slice(&offset_response, &mut response[response_offs..])
                                .map_err(|_| {
                                    respond(response, SERVER_ERROR, Some(b"Offset update failed"))
                                })?;

                            Some(len)
                        } else {
                            self.0
                                .get_state(cnt as usize, &mut response[response_offs..])
                                .map_err(|_| {
                                    respond(response, BAD_REQUEST, Some(b"Invalid query2"))
                                })?
                        };

                        if let Some(update_len) = update_len {
                            itoa(
                                update_len,
                                &mut response[content_len_offs..content_len_offs + 5],
                            );

                            Ok(Response::Complete(
                                Some(response_offs + update_len),
                                None,
                                None,
                            ))
                        } else {
                            Ok(Response::None)
                        }
                    }
                    _ => Err(respond(response, BAD_REQUEST, Some(b"Invalid query3"))),
                }
            }

            (Some("GET"), Some(path)) if path.starts_with('/') => {
                let path = &path[1..];
                if let Some(file) = lookup(path) {
                    let ext = path.split('.').last();
                    let content_type: Option<&[u8]> = if let Some(ext) = ext {
                        match ext {
                            "html" => Some(b"text/html\r\n"),
                            "css" => Some(b"text/css\r\n"),
                            "js" => Some(b"application/javascript\r\n"),
                            "png" => Some(b"image/png\r\n"),
                            "jpg" | "jpeg" => Some(b"image/jpeg\r\n"),
                            _ => None,
                        }
                    } else {
                        None
                    };

                    let (header_len, content_len_offs) = fill_header(response, OK, content_type);

                    itoa(
                        file.len(),
                        &mut response[content_len_offs..content_len_offs + 5],
                    );

                    Ok(Response::Complete(Some(header_len), None, Some(file)))
                } else {
                    Err(respond(response, NOT_FOUND, Some(b"bummer")))
                }
            }
            _ => Err(respond(response, NOT_FOUND, Some(b"Ooops"))),
        }
    }

    fn update_location(
        &mut self,
        location: Option<(f32, f32)>,
        speed: Option<(f32, f32)>,
        updates: &mut [u8],
    ) -> Option<usize> {
        let (header_len, content_len_offs) = fill_header(updates, OK, Some(APP_JSON));

        let len = self
            .0
            .update_location(location, speed, &mut updates[header_len..]);

        if let Some(len) = len {
            itoa(len, &mut updates[content_len_offs..content_len_offs + 5]);
            Some(header_len + len)
        } else {
            None
        }
    }

    fn handle_sleep(&mut self, updates: &mut [u8], callback: usize) -> Option<usize> {
        let (header_len, content_len_offs) = fill_header(updates, OK, Some(APP_JSON));

        let len = self.0.handle_sleep(&mut updates[header_len..], callback);

        if let Some(len) = len {
            itoa(len, &mut updates[content_len_offs..content_len_offs + 5]);
            Some(header_len + len)
        } else {
            None
        }
    }
}

fn str_to_usize(s: &str) -> Result<(u64, &str), &'static [u8]> {
    let mut result = 0u64;
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
        match (bytes[i] as char).to_digit(10) {
            Some(digit) => {
                result = match result
                    .checked_mul(10)
                    .and_then(|res| res.checked_add(digit as u64))
                {
                    Some(tmp) => tmp,
                    None => return Err(b"str_to_usize>2"),
                }
            }
            None => break,
        }
        i += 1;
    }

    // Skip '&' if present
    if i < bytes.len() && (bytes[i] as char) == '&' {
        i += 1;
    }

    Ok((result, &s[i..]))
}

fn parse_query(query: &str) -> Result<(Option<u64>, Option<u64>), &'static [u8]> {
    let mut slice = query;
    let mut result: (Option<u64>, Option<u64>) = (None, None);

    while !slice.is_empty() {
        if slice.starts_with("timestamp=") {
            let (value, new_slice) = str_to_usize(&slice["timestamp=".len()..])?;
            result.0 = Some(value);
            slice = new_slice;
        } else if slice.starts_with("cnt=") {
            let (value, new_slice) = str_to_usize(&slice["cnt=".len()..])?;
            result.1 = Some(value);
            slice = new_slice;
        } else {
            return Err(b"parse_query>2");
        }
    }

    Ok(result)
}

fn itoa(n: usize, buf: &mut [u8]) {
    let mut n = n;
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = (n % 10) as u8 + b'0';
        n /= 10;
    }
    for byte in buf[0..i].iter_mut().take(i) {
        *byte = b' ';
    }
}

fn fill_header(buffer: &mut [u8], status: &[u8], content_type: Option<&[u8]>) -> (usize, usize) {
    let mut offset: usize = 0;

    buffer[offset..offset + HTTP_VERSION.len()].copy_from_slice(HTTP_VERSION);
    offset += HTTP_VERSION.len();
    buffer[offset..offset + status.len()].copy_from_slice(status);
    offset += status.len();
    if let Some(content_type) = content_type {
        buffer[offset..offset + CONTENT_TYPE.len()].copy_from_slice(CONTENT_TYPE);
        offset += CONTENT_TYPE.len();
        buffer[offset..offset + content_type.len()].copy_from_slice(content_type);
        offset += content_type.len();
    }
    buffer[offset..offset + CONTENT_LENGTH.len()].copy_from_slice(CONTENT_LENGTH);
    offset += CONTENT_LENGTH.len();

    let content_length_offset = offset;

    buffer[offset + 5..offset + 5 + 4].copy_from_slice(b"\r\n\r\n");
    offset += 5 + 4;

    (offset, content_length_offset)
}

fn respond(response: &mut [u8], status: &[u8], body: Option<&[u8]>) -> usize {
    let mut offs: usize = 0;
    response[offs..offs + HTTP_VERSION.len()].copy_from_slice(HTTP_VERSION);
    offs += HTTP_VERSION.len();
    response[offs..offs + status.len()].copy_from_slice(status);
    offs += status.len();

    match body {
        Some(body) => {
            response[offs..offs + CONTENT_LENGTH.len()].copy_from_slice(CONTENT_LENGTH);
            offs += CONTENT_LENGTH.len();

            itoa(body.len(), &mut response[offs..offs + 5]);
            offs += 5;
            response[offs..offs + 4].copy_from_slice(b"\r\n\r\n");
            offs += 4;

            response[offs..offs + body.len()].copy_from_slice(body);

            offs + body.len()
        }
        None => {
            response[offs..offs + CONTENT_LENGTH.len()].copy_from_slice(CONTENT_LENGTH);
            offs += CONTENT_LENGTH.len();

            response[offs..offs + 6].copy_from_slice(b"0 \r\n\r\n");

            offs + 6
        }
    }
}
