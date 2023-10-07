use crate::core::SerdeEngineTrait;
use httparse::{Request, EMPTY_HEADER};

pub struct RequestWrapper<T: SerdeEngineTrait>(T);

impl<T> RequestWrapper<T>
where
    T: SerdeEngineTrait,
{
    pub fn handle_request(
        &mut self,
        body: &[u8],
        response: &mut [u8],
        sleep: &dyn Fn(usize, usize),
    ) -> Result<(), &'static str> {
        // Buffer to hold HTTP request headers
        let mut headers = [EMPTY_HEADER; 16];

        // Parsing the request
        let mut req = Request::new(&mut headers);
        let status = req.parse(body).map_err(|_| "Invalid HTTP request")?;

        // Check if the headers were fully parsed
        if let httparse::Status::Complete(offset) = status {
            if req.method == Some("POST") && req.path == Some("/events") {
                let content_length: usize = req
                    .headers
                    .iter()
                    .filter(|header| header.name.eq_ignore_ascii_case("Content-Length"))
                    .filter_map(|header| core::str::from_utf8(header.value).ok()?.parse().ok())
                    .next()
                    .ok_or("Content-Length not found or invalid")?;

                // Assuming the body starts right after the headers
                let event_body = &body[offset..offset + content_length];

                // Manually constructing the HTTP response headers with a placeholder for Content-Length
                let header = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length:     \r\n\r\n"; // 5 spaces as placeholder
                response[..header.len()].copy_from_slice(header);

                // Process the event
                let response_len =
                    self.0
                        .handle_event(event_body, &mut response[header.len()..], sleep)?;

                // Update the Content-Length placeholder with the actual length of the response body
                let content_length_offset = header.len() - 8;
                itoa(
                    response_len,
                    &mut response[content_length_offset..content_length_offset + 5],
                );

                Ok(())
            } else {
                // Unsupported HTTP method or path
                Err("Unsupported HTTP method or path")
            }
        } else {
            Err("Incomplete HTTP request")
        }
    }
}

// impl<T: CallbackEngine> Default for RequestWrapper<T>
// where
//     <T as EngineCore>::Callbacks: Copy,
// {
//     fn default() -> Self {
//         RequestWrapper(Engine::default())
//     }
// }

fn itoa(n: usize, buf: &mut [u8]) {
    let mut n = n;
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = (n % 10) as u8 + b'0';
        n /= 10;
    }
}
