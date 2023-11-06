#[cfg(test)]
mod tests {
    use crate::engine_httpd::{EngineHttpdTrait, Response};
    use crate::RaceHttpd;

    macro_rules! lines {
        ($($element:expr),+ $(,)? ) => {
            concat!($( $element, "\r\n", )+).as_bytes()
        };
    }
    fn sleep(_: u64, _: usize) -> Result<(), &'static str> {
        Ok(())
    }

    #[test]
    fn test_nano_fail1() {
        let mut httpd = RaceHttpd::default();
        let mut result = [0u8; 2048];
        let mut update = [0u8; 2048];

        let event = concat!(
            "GET /updates?cnt=0&timestamp=1696710546420 HTTP/1.1\r\n",
            "Host: 192.168.4.1\r\n",
            "Connection: keep-alive\r\n",
            "Pragma: no-cache\r\n",
            "Cache-Control: no-cache\r\n",
            "User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/116.0.0.0 Safari/537.36\r\n",
            "DNT: 1\r\n",
            "Accept: */*\r\n",
            "Referer: http://192.168.4.1/index.html\r\n",
            "Accept-Encoding: gzip, deflate\r\n",
            "Accept-Language: en-GB,en-US;q=0.9,en;q=0.8\r\n",
            "\r\n"
        );
        let response =
            httpd.handle_request(0, event.as_bytes(), &mut result, &mut update, &mut sleep);

        let response = response.unwrap();
        if let Response::Complete(response, updates, extra) = response {
            let response = response.unwrap();
            assert_eq!(extra, None);
            assert_eq!(updates, None);

            // println!(
            //     "response[{}], {:?}",
            //     response,
            //     String::from_utf8_lossy(&result[..response])
            // );
        } else {
            panic!("Unexpected response: {:?}", response);
        }
    }

    #[test]
    fn test_static() {
        let mut httpd = RaceHttpd::default();
        let mut result = [0u8; 2048];
        let mut update = [0u8; 2048];

        let event = b"GET /index.html HTTP/1.1\r\n\r\n";

        let response = httpd.handle_request(0, event, &mut result, &mut update, &sleep);

        let response = response.unwrap();
        if let Response::Complete(response, updates, extra) = response {
            assert!(response.unwrap() > 0);
            assert!(extra.is_some());
            assert_eq!(updates, None);
        } else {
            panic!("Unexpected response: {:?}", response);
        }
    }

    #[test]
    fn test_partial() {
        let mut httpd = RaceHttpd::default();
        let mut result = [0u8; 2048];
        let mut update = [0u8; 2048];

        let event = lines!(
            "POST /events HTTP/1.1",
            "Host: 169.254.1.1",
            "Origin: http://169.254.1.1",
            "Referer: http://169.254.1.1/index.html",
            "Content-Type: application/json",
            "Accept: */*",
            "Accept-Encoding: gzip",
            "Accept-Language: en-GB",
            "Connection: Keep-Alive",
            "User-Agent: Mozilla/5.0 (X11; U; Linux armv7l like Android; en-us) AppleWebKit/531.2+ (KHTML, like Gecko) Version/5.0 Safari/533.2+ Kindle/3.0+",
            "Content-Length: 20",
        );

        let response = httpd.handle_request(0, event, &mut result, &mut update, &sleep);

        assert!(response.is_err());
        let response = response.expect_err("Expected error");
        // println!("Response: {:?}", core::str::from_utf8(&result[..response]));
    }

    #[test]
    fn test_engine() {
        let mut httpd = RaceHttpd::default();
        let mut result = [0u8; 2048];
        let mut update = [0u8; 2048];

        let event = b"GET /updates?timestamp=23&cnt=1000 HTTP/1.1\r\n\r\n";
        let response = httpd.handle_request(0, event, &mut result, &mut update, &sleep);

        let response = response.unwrap();
        assert_eq!(response, Response::None);

        let event = b"GET /updates?timestamp=23&cnt=0 HTTP/1.1\r\n\r\n";
        let response = httpd.handle_request(0, event, &mut result, &mut update, &sleep);

        let response = response.unwrap();

        if let Response::Complete(response, updates, extra) = response {
            assert_eq!(updates, None);
            assert_eq!(extra, None);
            if let Some(response) = response {
                // println!(
                //     "response[{}], {:?}",
                //     response,
                //     String::from_utf8_lossy(&result[..response])
                // );
            } else {
                panic!("Unexpected response: {:?}", response);
            }
        } else {
            panic!("Unexpected response: {:?}", response);
        }

        let payload = json!({
            "event": "Activate"
        })
        .to_string();

        let event = format!(
            concat!(
                "POST /events HTTP/1.1\r\n",
                "Content-Type: application/json\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}",
            ),
            payload.len(),
            payload,
        );

        let response = httpd.handle_request(0, event.as_bytes(), &mut result, &mut update, &sleep);
        let response = response.unwrap();

        if let Response::Complete(response, updates, extra) = response {
            let updates = updates.unwrap();
            assert_eq!(updates, 123);
            assert_eq!(extra, None);
            let response = response.unwrap();
            // println!(
            //     "response[{}], {:?}",
            //     response,
            //     String::from_utf8_lossy(&result[..response])
            // );
            // println!(
            //     "updates[{}], {:?}",
            //     updates,
            //     String::from_utf8_lossy(&update[..updates])
            // );
        } else {
            panic!("Unexpected response: {:?}", response);
        }
    }
}
