use localhost::http::{
    parse_request,
    ParseError,
    ParseResult,
};

const MAX: usize = 1024 * 1024;

#[test]
fn decodes_single_chunk() {
    let raw =
        b"POST /upload HTTP/1.1\r\n\
Host: localhost\r\n\
Transfer-Encoding: chunked\r\n\
\r\n\
5\r\n\
hello\r\n\
0\r\n\
\r\n";

    let result =
        parse_request(raw, MAX).unwrap();

    match result {
        ParseResult::Complete {
            value,
            consumed,
        } => {
            assert_eq!(value.body, b"hello");
            assert_eq!(consumed, raw.len());
        }

        ParseResult::Incomplete => {
            panic!("request should be complete");
        }
    }
}

#[test]
fn decodes_multiple_chunks() {
    let raw =
        b"POST /upload HTTP/1.1\r\n\
Host: localhost\r\n\
Transfer-Encoding: chunked\r\n\
\r\n\
4\r\n\
Wiki\r\n\
6\r\n\
pedia \r\n\
D\r\n\
in\r\n\
\r\nchunks.\r\n\
0\r\n\
\r\n";

    let result =
        parse_request(raw, MAX).unwrap();

    match result {
        ParseResult::Complete { value, .. } => {
            assert_eq!(
                value.body,
                b"Wikipedia in\r\n\r\nchunks."
            );
        }

        ParseResult::Incomplete => {
            panic!("request should be complete");
        }
    }
}

#[test]
fn empty_chunked_body_is_accepted() {
    let raw =
        b"POST /upload HTTP/1.1\r\n\
Host: localhost\r\n\
Transfer-Encoding: chunked\r\n\
\r\n\
0\r\n\
\r\n";

    let result =
        parse_request(raw, MAX).unwrap();

    match result {
        ParseResult::Complete { value, .. } => {
            assert_eq!(value.body, Vec::<u8>::new());
        }

        ParseResult::Incomplete => {
            panic!("request should be complete");
        }
    }
}

#[test]
fn trailers_after_final_chunk_are_consumed() {
    let raw =
        b"POST /upload HTTP/1.1\r\n\
Host: localhost\r\n\
Transfer-Encoding: chunked\r\n\
\r\n\
5\r\n\
hello\r\n\
0\r\n\
X-Checksum: abc123\r\n\
\r\n";

    let result =
        parse_request(raw, MAX).unwrap();

    match result {
        ParseResult::Complete {
            value,
            consumed,
        } => {
            assert_eq!(value.body, b"hello");
            assert_eq!(consumed, raw.len());
        }

        ParseResult::Incomplete => {
            panic!("request should be complete");
        }
    }
}

#[test]
fn chunk_size_split_across_reads_is_incomplete_then_completes() {
    let head =
        b"POST /upload HTTP/1.1\r\n\
Host: localhost\r\n\
Transfer-Encoding: chunked\r\n\
\r\n";

    // Split right in the middle of the hex chunk-size line.
    let mut partial = head.to_vec();
    partial.extend_from_slice(b"5");

    assert_eq!(
        parse_request(&partial, MAX).unwrap(),
        ParseResult::Incomplete
    );

    let mut complete = partial.clone();
    complete.extend_from_slice(b"\r\nhello\r\n0\r\n\r\n");

    match parse_request(&complete, MAX).unwrap() {
        ParseResult::Complete { value, .. } => {
            assert_eq!(value.body, b"hello");
        }

        ParseResult::Incomplete => {
            panic!("request should be complete");
        }
    }
}

#[test]
fn chunk_data_split_across_reads_is_incomplete_then_completes() {
    let head =
        b"POST /upload HTTP/1.1\r\n\
Host: localhost\r\n\
Transfer-Encoding: chunked\r\n\
\r\n\
5\r\n\
hel";

    assert_eq!(
        parse_request(head, MAX).unwrap(),
        ParseResult::Incomplete
    );

    let mut complete = head.to_vec();
    complete.extend_from_slice(b"lo\r\n0\r\n\r\n");

    match parse_request(&complete, MAX).unwrap() {
        ParseResult::Complete { value, .. } => {
            assert_eq!(value.body, b"hello");
        }

        ParseResult::Incomplete => {
            panic!("request should be complete");
        }
    }
}

#[test]
fn missing_final_chunk_is_incomplete() {
    let raw =
        b"POST /upload HTTP/1.1\r\n\
Host: localhost\r\n\
Transfer-Encoding: chunked\r\n\
\r\n\
5\r\n\
hello\r\n";

    assert_eq!(
        parse_request(raw, MAX).unwrap(),
        ParseResult::Incomplete
    );
}

#[test]
fn malformed_chunk_size_is_rejected() {
    let raw =
        b"POST /upload HTTP/1.1\r\n\
Host: localhost\r\n\
Transfer-Encoding: chunked\r\n\
\r\n\
zz\r\n\
hello\r\n\
0\r\n\
\r\n";

    let result = parse_request(raw, MAX);

    assert!(matches!(
        result,
        Err(ParseError::InvalidChunkSize)
    ));
}

#[test]
fn missing_chunk_terminator_is_rejected() {
    let raw =
        b"POST /upload HTTP/1.1\r\n\
Host: localhost\r\n\
Transfer-Encoding: chunked\r\n\
\r\n\
5\r\n\
helloXX\r\n\
0\r\n\
\r\n";

    let result = parse_request(raw, MAX);

    assert!(matches!(
        result,
        Err(ParseError::InvalidChunkSize)
    ));
}

#[test]
fn chunk_extension_is_ignored() {
    let raw =
        b"POST /upload HTTP/1.1\r\n\
Host: localhost\r\n\
Transfer-Encoding: chunked\r\n\
\r\n\
5;ext=value\r\n\
hello\r\n\
0\r\n\
\r\n";

    let result =
        parse_request(raw, MAX).unwrap();

    match result {
        ParseResult::Complete { value, .. } => {
            assert_eq!(value.body, b"hello");
        }

        ParseResult::Incomplete => {
            panic!("request should be complete");
        }
    }
}

#[test]
fn chunked_body_over_limit_is_rejected() {
    let raw =
        b"POST /upload HTTP/1.1\r\n\
Host: localhost\r\n\
Transfer-Encoding: chunked\r\n\
\r\n\
5\r\n\
hello\r\n\
0\r\n\
\r\n";

    let result = parse_request(raw, 3);

    assert!(matches!(
        result,
        Err(ParseError::BodyTooLarge)
    ));
}

#[test]
fn content_length_body_over_limit_is_rejected() {
    let raw =
        b"POST /upload HTTP/1.1\r\n\
Host: localhost\r\n\
Content-Length: 5\r\n\
\r\n\
hello";

    let result = parse_request(raw, 3);

    assert!(matches!(
        result,
        Err(ParseError::BodyTooLarge)
    ));
}
