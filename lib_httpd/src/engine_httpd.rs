use httparse::{Request, EMPTY_HEADER};

use engine::{EventEngineTrait, SerdeEngine, SerdeEngineTrait};
use race_client::lookup;

#[derive(PartialEq)]
pub enum Response {
    // response length (excludes additional), update length, additional response
    Complete(Option<usize>, Option<usize>, Option<&'static [u8]>),
    // Partial(usize),
    None,
}

pub trait EngineHttpdTrait {
    fn handle_event(
        &mut self,
        event: &[u8],
        result: &mut [u8],
        sleep: &dyn Fn(usize, usize),
    ) -> Result<Option<usize>, &'static str>;

    fn get_static_file(&self, key: &str) -> Option<&'static [u8]>;

    fn handle_request(
        &mut self,
        request: &[u8],
        response: &mut [u8],
        updates: &mut [u8],
        sleep: &dyn Fn(usize, usize),
    ) -> Result<Response, usize>;
}

const HTTP_VERSION: &[u8] = b"HTTP/1.1 ";
const OK: &[u8] = b"200 OK\r\n";
const BAD_REQUEST: &[u8] = b"400 Bad Request\r\n";
const NOT_FOUND: &[u8] = b"404 Not Found\r\n";
const SERVER_ERROR: &[u8] = b"500 Internal Server Error\r\n";
const CONTENT_TYPE: &[u8] = b"Content-Type: ";
const APP_JSON: &[u8] = b"application/json\r\n";
const CONTENT_LENGTH: &[u8] = b"Content-Length: ";

#[derive(Default)]
pub struct EngineHttpd<T: EventEngineTrait>(SerdeEngine<T>);

impl<T: EventEngineTrait> EngineHttpdTrait for EngineHttpd<T> {
    fn handle_event(
        &mut self,
        event: &[u8],
        result: &mut [u8],
        sleep: &dyn Fn(usize, usize),
    ) -> Result<Option<usize>, &'static str> {
        self.0.handle_event(event, result, sleep)
    }

    fn get_static_file(&self, key: &str) -> Option<&'static [u8]> {
        lookup(key)
    }

    fn handle_request(
        &mut self,
        request: &[u8],
        response: &mut [u8],
        updates: &mut [u8],
        sleep: &dyn Fn(usize, usize),
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

                // Assuming the body starts right after the headers
                let event_body = &request[offset..offset + content_length];

                let mut update_offs: usize = 0;
                updates[update_offs..update_offs + HTTP_VERSION.len()]
                    .copy_from_slice(HTTP_VERSION);
                update_offs += HTTP_VERSION.len();
                updates[update_offs..update_offs + 8].copy_from_slice(OK);
                update_offs += OK.len();
                updates[update_offs..update_offs + CONTENT_TYPE.len()]
                    .copy_from_slice(CONTENT_TYPE);
                update_offs += CONTENT_TYPE.len();
                updates[update_offs..update_offs + APP_JSON.len()].copy_from_slice(APP_JSON);
                update_offs += APP_JSON.len();
                updates[update_offs..update_offs + CONTENT_LENGTH.len()]
                    .copy_from_slice(CONTENT_LENGTH);
                update_offs += CONTENT_LENGTH.len();
                let content_len_offs = update_offs;
                updates[update_offs + 5..update_offs + 5 + 4].copy_from_slice(b"\r\n\r\n");
                update_offs += 5 + 4;

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
                    // Err(respond(response, BAD_REQUEST, Some(err)))
                }
                let (timestamp, cnt) = query_args.unwrap();

                match (timestamp, cnt) {
                    (Some(timestamp), Some(cnt)) => {
                        let mut response_offs: usize = 0;
                        response[response_offs..response_offs + HTTP_VERSION.len()]
                            .copy_from_slice(HTTP_VERSION);
                        response_offs += HTTP_VERSION.len();
                        response[response_offs..response_offs + 8].copy_from_slice(OK);
                        response_offs += OK.len();
                        response[response_offs..response_offs + CONTENT_TYPE.len()]
                            .copy_from_slice(CONTENT_TYPE);
                        response_offs += CONTENT_TYPE.len();
                        response[response_offs..response_offs + APP_JSON.len()]
                            .copy_from_slice(APP_JSON);
                        response_offs += APP_JSON.len();
                        response[response_offs..response_offs + CONTENT_LENGTH.len()]
                            .copy_from_slice(CONTENT_LENGTH);
                        response_offs += CONTENT_LENGTH.len();
                        let content_len_offs = response_offs;
                        response[response_offs + 5..response_offs + 5 + 4]
                            .copy_from_slice(b"\r\n\r\n");
                        response_offs += 5 + 4;

                        let update_len = self
                            .0
                            .get_state(cnt as usize, &mut response[response_offs..])
                            .map_err(|_| respond(response, BAD_REQUEST, Some(b"Invalid query2")))?;

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

            (Some("GET"), Some(path)) if path.starts_with("/") => {
                let path = &path[1..];
                if let Some(file) = self.get_static_file(path) {
                    let mut offs: usize = 0;
                    response[offs..offs + HTTP_VERSION.len()].copy_from_slice(HTTP_VERSION);
                    offs += HTTP_VERSION.len();
                    response[offs..offs + 8].copy_from_slice(OK);
                    offs += OK.len();
                    response[offs..offs + CONTENT_LENGTH.len()].copy_from_slice(CONTENT_LENGTH);
                    offs += CONTENT_LENGTH.len();
                    itoa(file.len(), &mut response[offs..offs + 5]);
                    offs += 5;
                    response[offs..offs + 4].copy_from_slice(b"\r\n\r\n");
                    offs += 4;

                    Ok(Response::Complete(Some(offs), None, Some(file)))
                } else {
                    Err(respond(response, NOT_FOUND, None))
                }
            }
            _ => Err(respond(response, NOT_FOUND, None)),
        }
    }
}

fn str_to_usize(s: &str) -> Result<(u64, &str), &'static [u8]> {
    let mut result = 0u64;
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() && (bytes[i] as char).is_digit(10) {
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

    while slice.len() > 0 {
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
    for j in 0..i {
        buf[j] = b' ';
    }
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
            response[offs..offs + 2].copy_from_slice(b"\r\n");

            offs + 2
        }
    }
}
