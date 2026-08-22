use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusCode {
    Ok,
    BadRequest,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    PayloadTooLarge,
    InternalServerError,
}

impl StatusCode {
    pub fn code(self) -> u16 {
        match self {
            StatusCode::Ok => 200,
            StatusCode::BadRequest => 400,
            StatusCode::Forbidden => 403,
            StatusCode::NotFound => 404,
            StatusCode::MethodNotAllowed => 405,
            StatusCode::PayloadTooLarge => 413,
            StatusCode::InternalServerError => 500,
        }
    }

    pub fn reason_phrase(self) -> &'static str {
        match self {
            StatusCode::Ok => "OK",
            StatusCode::BadRequest => "Bad Request",
            StatusCode::Forbidden => "Forbidden",
            StatusCode::NotFound => "Not Found",
            StatusCode::MethodNotAllowed => "Method Not Allowed",
            StatusCode::PayloadTooLarge => "Payload Too Large",
            StatusCode::InternalServerError => "Internal Server Error",
        }
    }
}

impl fmt::Display for StatusCode {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        write!(
            f,
            "{} {}",
            self.code(),
            self.reason_phrase()
        )
    }
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: StatusCode,

    pub headers: Vec<(String, String)>,

    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(
        status: StatusCode,
        body: Vec<u8>,
    ) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body,
        }
    }

    pub fn html(
        status: StatusCode,
        body: impl Into<String>,
    ) -> Self {
        Self::new(
            status,
            body.into().into_bytes(),
        )
        .with_header(
            "Content-Type",
            "text/html; charset=utf-8",
        )
    }

    pub fn with_header(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.headers.push((
            name.into(),
            value.into(),
        ));

        self
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut response = Vec::new();

        let status_line = format!(
            "HTTP/1.1 {} {}\r\n",
            self.status.code(),
            self.status.reason_phrase()
        );

        response.extend_from_slice(
            status_line.as_bytes(),
        );

        let has_content_length =
            self.headers.iter().any(
                |(name, _)| {
                    name.eq_ignore_ascii_case(
                        "content-length"
                    )
                },
            );

        let has_server =
            self.headers.iter().any(
                |(name, _)| {
                    name.eq_ignore_ascii_case(
                        "server"
                    )
                },
            );

        for (name, value) in &self.headers {
            let header = format!(
                "{}: {}\r\n",
                name,
                value
            );

            response.extend_from_slice(
                header.as_bytes(),
            );
        }

        if !has_content_length {
            let header = format!(
                "Content-Length: {}\r\n",
                self.body.len()
            );

            response.extend_from_slice(
                header.as_bytes(),
            );
        }

        if !has_server {
            response.extend_from_slice(
                b"Server: localhost-rust\r\n",
            );
        }

        response.extend_from_slice(
            b"\r\n",
        );

        response.extend_from_slice(
            &self.body,
        );

        response
    }
}