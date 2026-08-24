use std::fmt;
use std::str;

use super::request::{
    BodyFraming,
    Header,
    HttpRequest,
    HttpVersion,
    Method,
    RequestHead,
};

/// Prevents a client from growing the header buffer forever.
///
/// The upload/body limit is handled separately using the configuration.
/// This limit applies only to the HTTP request line + headers.
pub const MAX_HEADER_SIZE: usize = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseResult<T> {
    Complete {
        value: T,
        consumed: usize,
    },

    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    HeaderTooLarge,

    InvalidUtf8,

    InvalidRequestLine,

    InvalidMethod,

    InvalidTarget,

    UnsupportedVersion(String),

    InvalidHeader,

    MissingHost,

    DuplicateHost,

    InvalidContentLength,

    ConflictingContentLength,

    ConflictingBodyFraming,

    UnsupportedTransferEncoding(String),

    InvalidChunkSize,

    BodyTooLarge,
}

impl fmt::Display for ParseError {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            ParseError::HeaderTooLarge => {
                write!(f, "HTTP headers are too large")
            }

            ParseError::InvalidUtf8 => {
                write!(f, "HTTP headers contain invalid UTF-8")
            }

            ParseError::InvalidRequestLine => {
                write!(f, "invalid HTTP request line")
            }

            ParseError::InvalidMethod => {
                write!(f, "invalid HTTP method")
            }

            ParseError::InvalidTarget => {
                write!(f, "invalid HTTP request target")
            }

            ParseError::UnsupportedVersion(version) => {
                write!(
                    f,
                    "unsupported HTTP version '{}'",
                    version
                )
            }

            ParseError::InvalidHeader => {
                write!(f, "invalid HTTP header")
            }

            ParseError::MissingHost => {
                write!(
                    f,
                    "HTTP/1.1 request is missing Host header"
                )
            }

            ParseError::DuplicateHost => {
                write!(
                    f,
                    "HTTP/1.1 request contains multiple Host headers"
                )
            }

            ParseError::InvalidContentLength => {
                write!(f, "invalid Content-Length header")
            }

            ParseError::ConflictingContentLength => {
                write!(
                    f,
                    "conflicting Content-Length headers"
                )
            }

            ParseError::ConflictingBodyFraming => {
                write!(
                    f,
                    "request contains both Content-Length and Transfer-Encoding"
                )
            }

            ParseError::UnsupportedTransferEncoding(value) => {
                write!(
                    f,
                    "unsupported Transfer-Encoding '{}'",
                    value
                )
            }

            ParseError::InvalidChunkSize => {
                write!(
                    f,
                    "invalid chunk size in chunked request body"
                )
            }

            ParseError::BodyTooLarge => {
                write!(
                    f,
                    "request body exceeds the configured limit"
                )
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Parses only the request line and headers.
///
/// This function is incremental:
///
/// - If "\r\n\r\n" has not arrived yet -> Incomplete
/// - If the header section is valid -> Complete
/// - Malformed HTTP -> ParseError
///
/// This function only recognizes chunked framing (via the
/// Transfer-Encoding header); chunk decoding happens in
/// parse_request.
pub fn parse_request_head(
    buffer: &[u8],
) -> Result<ParseResult<RequestHead>, ParseError> {
    let header_end = match find_header_end(buffer) {
        Some(position) => position,

        None => {
            if buffer.len() > MAX_HEADER_SIZE {
                return Err(ParseError::HeaderTooLarge);
            }

            return Ok(ParseResult::Incomplete);
        }
    };

    if header_end > MAX_HEADER_SIZE {
        return Err(ParseError::HeaderTooLarge);
    }

    // header_end points to the first byte of "\r\n\r\n".
    let header_bytes = &buffer[..header_end];

    let header_text =
        str::from_utf8(header_bytes)
            .map_err(|_| ParseError::InvalidUtf8)?;

    let mut lines = header_text.split("\r\n");

    let request_line =
        lines
            .next()
            .ok_or(ParseError::InvalidRequestLine)?;

    let (
        method,
        target,
        version,
    ) = parse_request_line(request_line)?;

    let mut headers = Vec::new();

    for line in lines {
        if line.is_empty() {
            continue;
        }

        let header = parse_header(line)?;

        headers.push(header);
    }

    validate_host(&headers)?;

    let body_framing =
        determine_body_framing(&headers)?;

    let (path, query) =
        split_target(&target);

    Ok(ParseResult::Complete {
        value: RequestHead {
            method,
            target,
            path,
            query,
            version,
            headers,
            body_framing,
        },

        // +4 includes "\r\n\r\n".
        consumed: header_end + 4,
    })
}

/// Parses one complete HTTP request, including Content-Length and
/// chunked bodies.
///
/// `max_body_size` bounds how large a decoded body may grow while
/// parsing. This is a connection-level, not yet route-aware, cap: it
/// exists purely to stop a client from making the server buffer an
/// unbounded amount of memory before routing/config-specific limits
/// can be checked. Callers should still re-check the final body
/// length against the selected server's configured
/// client_max_body_size after routing.
pub fn parse_request(
    buffer: &[u8],
    max_body_size: usize,
) -> Result<ParseResult<HttpRequest>, ParseError> {
    let (
        head,
        header_consumed,
    ) = match parse_request_head(buffer)? {
        ParseResult::Incomplete => {
            return Ok(ParseResult::Incomplete);
        }

        ParseResult::Complete {
            value,
            consumed,
        } => {
            (value, consumed)
        }
    };

    let (body, total_length) =
        match head.body_framing {
            BodyFraming::None => {
                (Vec::new(), header_consumed)
            }

            BodyFraming::ContentLength(length) => {
                if length > max_body_size {
                    return Err(ParseError::BodyTooLarge);
                }

                let total_length =
                    header_consumed
                        .checked_add(length)
                        .ok_or(ParseError::InvalidContentLength)?;

                if buffer.len() < total_length {
                    return Ok(ParseResult::Incomplete);
                }

                let body =
                    buffer[header_consumed..total_length]
                        .to_vec();

                (body, total_length)
            }

            BodyFraming::Chunked => {
                let remaining = &buffer[header_consumed..];

                match decode_chunked_body(
                    remaining,
                    max_body_size,
                )? {
                    ParseResult::Incomplete => {
                        return Ok(ParseResult::Incomplete);
                    }

                    ParseResult::Complete {
                        value: body,
                        consumed: body_consumed,
                    } => {
                        (
                            body,
                            header_consumed + body_consumed,
                        )
                    }
                }
            }
        };

    Ok(ParseResult::Complete {
        value: HttpRequest {
            method: head.method,
            target: head.target,
            path: head.path,
            query: head.query,
            version: head.version,
            headers: head.headers,
            body,
        },

        consumed: total_length,
    })
}

/// Decodes a chunked-transfer-encoding message body.
///
/// `buffer` starts right after the request headers, at the first
/// chunk-size line. Handles chunk boundaries, chunk sizes, and
/// trailers split across separate reads by returning
/// `ParseResult::Incomplete` whenever the buffer runs out before a
/// full element (chunk-size line, chunk data + CRLF, or trailer
/// section) has arrived.
///
/// Format (RFC 7230 section 4.1):
///
/// chunk-size [ ";" chunk-ext ] CRLF
/// chunk-data CRLF
/// ...
/// "0" [ ";" chunk-ext ] CRLF
/// trailer-part
/// CRLF
fn decode_chunked_body(
    buffer: &[u8],
    max_body_size: usize,
) -> Result<ParseResult<Vec<u8>>, ParseError> {
    let mut body = Vec::new();

    let mut offset = 0;

    loop {
        let line_end =
            match find_crlf(&buffer[offset..]) {
                Some(position) => offset + position,

                None => {
                    return Ok(ParseResult::Incomplete);
                }
            };

        let size_line = &buffer[offset..line_end];

        let chunk_size =
            parse_chunk_size(size_line)?;

        offset = line_end + 2;

        if chunk_size == 0 {
            /*
             * Final chunk. What follows is an optional trailer
             * section (zero or more header-like lines) terminated
             * by a blank line.
             */
            let trailer_end =
                match find_crlf_crlf_or_bare_crlf(
                    &buffer[offset..]
                ) {
                    Some(position) => offset + position,

                    None => {
                        return Ok(ParseResult::Incomplete);
                    }
                };

            let consumed = trailer_end + 2;

            return Ok(ParseResult::Complete {
                value: body,
                consumed,
            });
        }

        let new_len =
            body.len()
                .checked_add(chunk_size)
                .ok_or(ParseError::BodyTooLarge)?;

        if new_len > max_body_size {
            return Err(ParseError::BodyTooLarge);
        }

        let data_end =
            match offset.checked_add(chunk_size) {
                Some(end) => end,

                None => {
                    return Err(ParseError::InvalidChunkSize);
                }
            };

        // Chunk data must be followed by CRLF.
        let terminator_end =
            match data_end.checked_add(2) {
                Some(end) => end,

                None => {
                    return Err(ParseError::InvalidChunkSize);
                }
            };

        if buffer.len() < terminator_end {
            return Ok(ParseResult::Incomplete);
        }

        if &buffer[data_end..terminator_end] != b"\r\n" {
            return Err(ParseError::InvalidChunkSize);
        }

        body.extend_from_slice(
            &buffer[offset..data_end],
        );

        offset = terminator_end;
    }
}

/// Parses a chunk-size line, ignoring any chunk-extension after ';'.
fn parse_chunk_size(
    line: &[u8],
) -> Result<usize, ParseError> {
    let hex_part =
        match line.iter().position(|&byte| byte == b';') {
            Some(index) => &line[..index],

            None => line,
        };

    if hex_part.is_empty() {
        return Err(ParseError::InvalidChunkSize);
    }

    let text =
        str::from_utf8(hex_part)
            .map_err(|_| ParseError::InvalidChunkSize)?;

    usize::from_str_radix(text.trim(), 16)
        .map_err(|_| ParseError::InvalidChunkSize)
}

/// Finds the offset of the next "\r\n" in `buffer`.
fn find_crlf(
    buffer: &[u8],
) -> Option<usize> {
    buffer
        .windows(2)
        .position(|window| window == b"\r\n")
}

/// Finds the end of the trailer section following the final ("0")
/// chunk: either an immediate "\r\n" (no trailers) or a "\r\n\r\n"
/// terminating one or more trailer header lines.
///
/// Returns the offset of the start of the terminating "\r\n".
fn find_crlf_crlf_or_bare_crlf(
    buffer: &[u8],
) -> Option<usize> {
    if buffer.len() >= 2 && &buffer[..2] == b"\r\n" {
        return Some(0);
    }

    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 2)
}

fn parse_request_line(
    line: &str,
) -> Result<
    (Method, String, HttpVersion),
    ParseError,
> {
    /*
     * HTTP request-line:
     *
     * method SP request-target SP HTTP-version
     */

    let mut parts = line.split(' ');

    let method =
        parts
            .next()
            .ok_or(ParseError::InvalidRequestLine)?;

    let target =
        parts
            .next()
            .ok_or(ParseError::InvalidRequestLine)?;

    let version =
        parts
            .next()
            .ok_or(ParseError::InvalidRequestLine)?;

    // More than three components means malformed request-line.
    if parts.next().is_some() {
        return Err(ParseError::InvalidRequestLine);
    }

    if method.is_empty()
        || target.is_empty()
        || version.is_empty()
    {
        return Err(ParseError::InvalidRequestLine);
    }

    if !method.bytes().all(is_token_char) {
        return Err(ParseError::InvalidMethod);
    }

    if target.bytes().any(|byte| {
        byte <= 0x20 || byte == 0x7f
    }) {
        return Err(ParseError::InvalidTarget);
    }

    let http_version =
        match version {
            "HTTP/1.1" => HttpVersion::Http11,

            other => {
                return Err(
                    ParseError::UnsupportedVersion(
                        other.to_string()
                    )
                );
            }
        };

    Ok((
        Method::from_str(method),
        target.to_string(),
        http_version,
    ))
}

fn parse_header(
    line: &str,
) -> Result<Header, ParseError> {
    // Obsolete line folding is not accepted.
    if line.starts_with(' ')
        || line.starts_with('\t')
    {
        return Err(ParseError::InvalidHeader);
    }

    let (
        name,
        value,
    ) = line
        .split_once(':')
        .ok_or(ParseError::InvalidHeader)?;

    if name.is_empty()
        || !name.bytes().all(is_token_char)
    {
        return Err(ParseError::InvalidHeader);
    }

    let value =
        value.trim_matches(|c| {
            c == ' ' || c == '\t'
        });

    // Header value may not contain control characters,
    // except horizontal tab.
    if value.bytes().any(|byte| {
        (byte < 0x20 && byte != b'\t')
            || byte == 0x7f
    }) {
        return Err(ParseError::InvalidHeader);
    }

    Ok(Header {
        // Header field names are case-insensitive.
        // Storing them lowercase makes future routing easier.
        name: name.to_ascii_lowercase(),

        value: value.to_string(),
    })
}

fn validate_host(
    headers: &[Header],
) -> Result<(), ParseError> {
    let hosts: Vec<&Header> =
        headers
            .iter()
            .filter(|header| {
                header
                    .name
                    .eq_ignore_ascii_case("host")
            })
            .collect();

    if hosts.is_empty() {
        return Err(ParseError::MissingHost);
    }

    if hosts.len() > 1 {
        return Err(ParseError::DuplicateHost);
    }

    if hosts[0].value.trim().is_empty() {
        return Err(ParseError::MissingHost);
    }

    Ok(())
}

fn determine_body_framing(
    headers: &[Header],
) -> Result<BodyFraming, ParseError> {
    let content_lengths:
        Vec<&str> =
        headers
            .iter()
            .filter(|header| {
                header
                    .name
                    .eq_ignore_ascii_case(
                        "content-length"
                    )
            })
            .map(|header| header.value.as_str())
            .collect();

    let transfer_encodings:
        Vec<&str> =
        headers
            .iter()
            .filter(|header| {
                header
                    .name
                    .eq_ignore_ascii_case(
                        "transfer-encoding"
                    )
            })
            .map(|header| header.value.as_str())
            .collect();

    /*
     * Having both Transfer-Encoding and Content-Length is
     * dangerous because different intermediaries can interpret
     * the request differently.
     */
    if !content_lengths.is_empty()
        && !transfer_encodings.is_empty()
    {
        return Err(
            ParseError::ConflictingBodyFraming
        );
    }

    if !transfer_encodings.is_empty() {
        let combined =
            transfer_encodings.join(",");

        let encodings:
            Vec<String> =
            combined
                .split(',')
                .map(|item| {
                    item
                        .trim()
                        .to_ascii_lowercase()
                })
                .filter(|item| {
                    !item.is_empty()
                })
                .collect();

        if encodings.len() == 1
            && encodings[0] == "chunked"
        {
            return Ok(BodyFraming::Chunked);
        }

        return Err(
            ParseError::UnsupportedTransferEncoding(
                combined
            )
        );
    }

    if content_lengths.is_empty() {
        return Ok(BodyFraming::None);
    }

    let mut parsed_length:
        Option<usize> = None;

    for value in content_lengths {
        /*
         * Content-Length values can sometimes arrive as:
         *
         * Content-Length: 5
         *
         * or as a combined duplicate:
         *
         * Content-Length: 5, 5
         */

        for part in value.split(',') {
            let part = part.trim();

            if part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| {
                        byte.is_ascii_digit()
                    })
            {
                return Err(
                    ParseError::InvalidContentLength
                );
            }

            let value =
                part
                    .parse::<usize>()
                    .map_err(|_| {
                        ParseError::InvalidContentLength
                    })?;

            match parsed_length {
                None => {
                    parsed_length = Some(value);
                }

                Some(previous)
                    if previous == value =>
                {
                    // Identical duplicate values are safe.
                }

                Some(_) => {
                    return Err(
                        ParseError::ConflictingContentLength
                    );
                }
            }
        }
    }

    Ok(
        BodyFraming::ContentLength(
            parsed_length
                .ok_or(
                    ParseError::InvalidContentLength
                )?
        )
    )
}

fn split_target(
    target: &str,
) -> (String, Option<String>) {
    match target.split_once('?') {
        Some((path, query)) => {
            (
                path.to_string(),
                Some(query.to_string()),
            )
        }

        None => {
            (
                target.to_string(),
                None,
            )
        }
    }
}

fn find_header_end(
    buffer: &[u8],
) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| {
            window == b"\r\n\r\n"
        })
}

/// RFC-style HTTP token character check.
///
/// Used for methods and header field names.
fn is_token_char(
    byte: u8,
) -> bool {
    matches!(
        byte,

        b'!'
            | b'#'
            | b'$'
            | b'%'
            | b'&'
            | b'\''
            | b'*'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~'
            | b'0'..=b'9'
            | b'A'..=b'Z'
            | b'a'..=b'z'
    )
}