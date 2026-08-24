use std::fmt;
use std::time::{
    SystemTime,
    UNIX_EPOCH,
};

/*
 * Formats a SystemTime as an RFC 7231 IMF-fixdate, e.g.:
 *
 * Sun, 06 Nov 1994 08:49:37 GMT
 *
 * Computed with plain civil-calendar arithmetic (Howard Hinnant's
 * days_from_civil algorithm) since no time/chrono crate is available
 * and epoll-driven server code should not depend on one.
 */
fn http_date(
    time: SystemTime,
) -> String {
    let secs =
        time.duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

    let days =
        secs.div_euclid(86_400);

    let day_secs =
        secs.rem_euclid(86_400);

    let hour = day_secs / 3600;
    let minute = (day_secs % 3600) / 60;
    let second = day_secs % 60;

    let weekday =
        (days.rem_euclid(7) + 4) % 7;

    let weekday_name = [
        "Sun", "Mon", "Tue", "Wed",
        "Thu", "Fri", "Sat",
    ][weekday as usize];

    let (year, month, day) =
        civil_from_days(days);

    let month_name = [
        "Jan", "Feb", "Mar", "Apr",
        "May", "Jun", "Jul", "Aug",
        "Sep", "Oct", "Nov", "Dec",
    ][(month - 1) as usize];

    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        weekday_name,
        day,
        month_name,
        year,
        hour,
        minute,
        second,
    )
}

/*
 * Converts a day count since the Unix epoch (1970-01-01) into a
 * (year, month, day) civil date. Proleptic Gregorian calendar.
 */
fn civil_from_days(
    z: i64,
) -> (i64, i64, i64) {
    let z = z + 719_468;

    let era =
        if z >= 0 { z } else { z - 146_096 }
            / 146_097;

    let doe = (z - era * 146_097) as u64;

    let yoe = (doe
        - doe / 1460
        + doe / 36524
        - doe / 146_096)
        / 365;

    let y = yoe as i64 + era * 400;

    let doy =
        doe - (365 * yoe + yoe / 4 - yoe / 100);

    let mp = (5 * doy + 2) / 153;

    let d = (doy - (153 * mp + 2) / 5 + 1) as i64;

    let m =
        if mp < 10 { mp + 3 } else { mp - 9 }
            as i64;

    let year =
        if m <= 2 { y + 1 } else { y };

    (year, m, d)
}

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

        let has_date =
            self.headers.iter().any(
                |(name, _)| {
                    name.eq_ignore_ascii_case(
                        "date"
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

        if !has_date {
            let header = format!(
                "Date: {}\r\n",
                http_date(SystemTime::now())
            );

            response.extend_from_slice(
                header.as_bytes(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_date_matches_rfc7231_example() {
        let time =
            UNIX_EPOCH
                + std::time::Duration::from_secs(
                    784_111_777,
                );

        assert_eq!(
            http_date(time),
            "Sun, 06 Nov 1994 08:49:37 GMT",
        );
    }

    #[test]
    fn http_date_matches_epoch() {
        assert_eq!(
            http_date(UNIX_EPOCH),
            "Thu, 01 Jan 1970 00:00:00 GMT",
        );
    }
}