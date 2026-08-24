use localhost::http::{
    parse_request,
    parse_request_head,
    BodyFraming,
    Method,
    ParseError,
    ParseResult,
};

#[test]
fn parses_simple_get_request() {
    let raw =
        b"GET / HTTP/1.1\r\n\
Host: localhost\r\n\
\r\n";

    let result =
        parse_request(raw, 1024 * 1024).unwrap();

    match result {
        ParseResult::Complete {
            value,
            consumed,
        } => {
            assert_eq!(
                value.method,
                Method::Get
            );

            assert_eq!(
                value.path,
                "/"
            );

            assert_eq!(
                value.query,
                None
            );

            assert_eq!(
                value.header("host"),
                Some("localhost")
            );

            assert_eq!(
                value.body,
                Vec::<u8>::new()
            );

            assert_eq!(
                consumed,
                raw.len()
            );
        }

        ParseResult::Incomplete => {
            panic!(
                "request should be complete"
            );
        }
    }
}

#[test]
fn parses_query_string() {
    let raw =
        b"GET /users?id=42&name=andreas HTTP/1.1\r\n\
Host: localhost\r\n\
\r\n";

    let result =
        parse_request(raw, 1024 * 1024).unwrap();

    match result {
        ParseResult::Complete {
            value,
            ..
        } => {
            assert_eq!(
                value.path,
                "/users"
            );

            assert_eq!(
                value.query.as_deref(),
                Some(
                    "id=42&name=andreas"
                )
            );
        }

        _ => {
            panic!(
                "request should be complete"
            );
        }
    }
}

#[test]
fn parses_post_with_body() {
    let raw =
        b"POST /submit HTTP/1.1\r\n\
Host: localhost\r\n\
Content-Length: 5\r\n\
\r\n\
hello";

    let result =
        parse_request(raw, 1024 * 1024).unwrap();

    match result {
        ParseResult::Complete {
            value,
            ..
        } => {
            assert_eq!(
                value.method,
                Method::Post
            );

            assert_eq!(
                value.path,
                "/submit"
            );

            assert_eq!(
                value.body,
                b"hello"
            );
        }

        _ => {
            panic!(
                "request should be complete"
            );
        }
    }
}

#[test]
fn partial_request_is_incomplete() {
    let raw =
        b"GET / HTTP/1.1\r\n\
Host: local";

    let result =
        parse_request(raw, 1024 * 1024).unwrap();

    assert_eq!(
        result,
        ParseResult::Incomplete
    );
}

#[test]
fn partial_body_is_incomplete() {
    let raw =
        b"POST /submit HTTP/1.1\r\n\
Host: localhost\r\n\
Content-Length: 5\r\n\
\r\n\
hel";

    let result =
        parse_request(raw, 1024 * 1024).unwrap();

    assert_eq!(
        result,
        ParseResult::Incomplete
    );
}

#[test]
fn missing_host_is_rejected() {
    let raw =
        b"GET / HTTP/1.1\r\n\
User-Agent: test\r\n\
\r\n";

    let result =
        parse_request(raw, 1024 * 1024);

    assert!(matches!(
        result,
        Err(ParseError::MissingHost)
    ));
}

#[test]
fn duplicate_host_is_rejected() {
    let raw =
        b"GET / HTTP/1.1\r\n\
Host: one.local\r\n\
Host: two.local\r\n\
\r\n";

    let result =
        parse_request(raw, 1024 * 1024);

    assert!(matches!(
        result,
        Err(ParseError::DuplicateHost)
    ));
}

#[test]
fn unsupported_http_version_is_rejected() {
    let raw =
        b"GET / HTTP/1.0\r\n\
Host: localhost\r\n\
\r\n";

    let result =
        parse_request(raw, 1024 * 1024);

    assert!(matches!(
        result,
        Err(
            ParseError::UnsupportedVersion(_)
        )
    ));
}

#[test]
fn malformed_header_is_rejected() {
    let raw =
        b"GET / HTTP/1.1\r\n\
Host localhost\r\n\
\r\n";

    let result =
        parse_request(raw, 1024 * 1024);

    assert!(matches!(
        result,
        Err(ParseError::InvalidHeader)
    ));
}

#[test]
fn unknown_method_is_parsed() {
    let raw =
        b"PATCH /users HTTP/1.1\r\n\
Host: localhost\r\n\
\r\n";

    let result =
        parse_request(raw, 1024 * 1024).unwrap();

    match result {
        ParseResult::Complete {
            value,
            ..
        } => {
            assert_eq!(
                value.method,
                Method::Other(
                    "PATCH".to_string()
                )
            );
        }

        _ => {
            panic!(
                "request should be complete"
            );
        }
    }
}

#[test]
fn conflicting_content_length_is_rejected() {
    let raw =
        b"POST / HTTP/1.1\r\n\
Host: localhost\r\n\
Content-Length: 5\r\n\
Content-Length: 10\r\n\
\r\n";

    let result =
        parse_request(raw, 1024 * 1024);

    assert!(matches!(
        result,
        Err(
            ParseError::ConflictingContentLength
        )
    ));
}

#[test]
fn content_length_and_transfer_encoding_are_rejected_together() {
    let raw =
        b"POST / HTTP/1.1\r\n\
Host: localhost\r\n\
Content-Length: 5\r\n\
Transfer-Encoding: chunked\r\n\
\r\n";

    let result =
        parse_request_head(raw);

    assert!(matches!(
        result,
        Err(
            ParseError::ConflictingBodyFraming
        )
    ));
}

#[test]
fn chunked_request_is_recognized() {
    let raw =
        b"POST /upload HTTP/1.1\r\n\
Host: localhost\r\n\
Transfer-Encoding: chunked\r\n\
\r\n";

    let result =
        parse_request_head(raw)
            .unwrap();

    match result {
        ParseResult::Complete {
            value,
            ..
        } => {
            assert_eq!(
                value.body_framing,
                BodyFraming::Chunked
            );
        }

        _ => {
            panic!(
                "headers should be complete"
            );
        }
    }
}

#[test]
fn headers_are_case_insensitive() {
    let raw =
        b"GET / HTTP/1.1\r\n\
hOsT: localhost\r\n\
USER-AGENT: test\r\n\
\r\n";

    let result =
        parse_request(raw, 1024 * 1024).unwrap();

    match result {
        ParseResult::Complete {
            value,
            ..
        } => {
            assert_eq!(
                value.header("HOST"),
                Some("localhost")
            );

            assert_eq!(
                value.header("user-agent"),
                Some("test")
            );
        }

        _ => {
            panic!(
                "request should be complete"
            );
        }
    }
}

#[test]
fn identical_content_lengths_are_allowed() {
    let raw =
        b"POST / HTTP/1.1\r\n\
Host: localhost\r\n\
Content-Length: 5\r\n\
Content-Length: 5\r\n\
\r\n\
hello";

    let result =
        parse_request(raw, 1024 * 1024);

    assert!(matches!(
        result,
        Ok(
            ParseResult::Complete {
                ..
            }
        )
    ));
}

#[test]
fn parses_delete_request() {
    let raw =
        b"DELETE /files/test.txt HTTP/1.1\r\n\
Host: localhost\r\n\
\r\n";

    let result =
        parse_request(raw, 1024 * 1024).unwrap();

    match result {
        ParseResult::Complete {
            value,
            ..
        } => {
            assert_eq!(
                value.method,
                Method::Delete
            );

            assert_eq!(
                value.path,
                "/files/test.txt"
            );
        }

        _ => {
            panic!(
                "request should be complete"
            );
        }
    }
}