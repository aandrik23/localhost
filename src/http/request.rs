use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Delete,
    Other(String),
}

impl Method {
    pub fn from_str(value: &str) -> Self {
        match value {
            "GET" => Method::Get,
            "POST" => Method::Post,
            "DELETE" => Method::Delete,
            other => Method::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Delete => "DELETE",
            Method::Other(value) => value,
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpVersion {
    Http11,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyFraming {
    None,
    ContentLength(usize),
    Chunked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHead {
    pub method: Method,

    /// Original request target.
    ///
    /// Example:
    /// /users?id=10
    pub target: String,

    /// Path without query string.
    ///
    /// Example:
    /// /users
    pub path: String,

    /// Query string without '?'.
    ///
    /// Example:
    /// id=10
    pub query: Option<String>,

    pub version: HttpVersion,

    pub headers: Vec<Header>,

    pub body_framing: BodyFraming,
}

impl RequestHead {
    /// Returns the first matching header.
    ///
    /// Header names are compared case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
    }

    pub fn header_values<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a str> {
        self.headers
            .iter()
            .filter(move |header| {
                header.name.eq_ignore_ascii_case(name)
            })
            .map(|header| header.value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: Method,

    pub target: String,

    pub path: String,

    pub query: Option<String>,

    pub version: HttpVersion,

    pub headers: Vec<Header>,

    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
    }

    pub fn header_values<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a str> {
        self.headers
            .iter()
            .filter(move |header| {
                header.name.eq_ignore_ascii_case(name)
            })
            .map(|header| header.value.as_str())
    }
}