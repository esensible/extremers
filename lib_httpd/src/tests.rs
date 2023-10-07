#[cfg(test)]
#[cfg(feature = "std")]
mod tests {
    use super::*;
    use crate::engine_httpd::{EngineHttpdTrait, Response};
    use crate::RaceHttpd;

    fn sleep(_: usize, _: usize) {}

    #[test]
    fn test_static() {
        let mut httpd = RaceHttpd::default();
        let mut result = [0u8; 2048];
        let mut update = [0u8; 2048];

        let event = b"GET /index.html HTTP/1.1\r\n\r\n";

        println!("hwllo");
        let response = httpd.handle_request(event, &mut result, &mut update, &sleep);

        let response = response.unwrap();
        if let Response::Complete(response, updates, extra) = response {
            let response = response.unwrap();
            let extra = extra.unwrap();
            assert_eq!(updates, None);
            println!(
                "response[{}], {:?}",
                response,
                String::from_utf8_lossy(&result[..response])
            );
            println!(
                "updates[{}], {:?}",
                extra.len(),
                String::from_utf8_lossy(&extra[..extra.len()])
            );
        } else {
            panic!("Unexpected response: {:?}", response);
        }
    }

    #[test]
    fn test_engine() {
        let mut httpd = RaceHttpd::default();
        let mut result = [0u8; 2048];
        let mut update = [0u8; 2048];

        let event = b"GET /updates?timestamp=23&cnt=1000 HTTP/1.1\r\n\r\n";
        let response = httpd.handle_request(event, &mut result, &mut update, &sleep);

        let response = response.unwrap();
        assert_eq!(response, Response::None);

        let event = b"GET /updates?timestamp=23&cnt=0 HTTP/1.1\r\n\r\n";
        let response = httpd.handle_request(event, &mut result, &mut update, &sleep);

        let response = response.unwrap();

        if let Response::Complete(response, updates, extra) = response {
            assert_eq!(updates, None);
            assert_eq!(extra, None);
            if let Some(response) = response {
                println!(
                    "response[{}], {:?}",
                    response,
                    String::from_utf8_lossy(&result[..response])
                );
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

        let response = httpd.handle_request(event.as_bytes(), &mut result, &mut update, &sleep);
        let response = response.unwrap();

        if let Response::Complete(response, updates, extra) = response {
            let updates = updates.unwrap();
            assert_eq!(updates, 123);
            assert_eq!(extra, None);
            let response = response.unwrap();
            println!(
                "response[{}], {:?}",
                response,
                String::from_utf8_lossy(&result[..response])
            );
            println!(
                "updates[{}], {:?}",
                updates,
                String::from_utf8_lossy(&update[..updates])
            );
        } else {
            panic!("Unexpected response: {:?}", response);
        }
    }
}
