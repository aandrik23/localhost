use std::collections::HashMap;
use std::net::Ipv4Addr;

use localhost::config::{
    Config,
    RouteConfig,
    ServerConfig,
};

use localhost::http::{
    Header,
    HttpRequest,
    HttpVersion,
    Method,
    StatusCode,
};

use localhost::server::http_handler::{
    handle_request,
    RequestOutcome,
};
use localhost::server::session::SessionStore;

fn expect_response(
    outcome: RequestOutcome,
) -> localhost::http::HttpResponse {
    match outcome {
        RequestOutcome::Response(response) => response,

        RequestOutcome::StartCgi { .. } => {
            panic!("expected a direct response, got StartCgi")
        }
    }
}

fn request(
    path: &str,
    cookie: Option<&str>,
) -> HttpRequest {
    let mut headers = Vec::new();

    if let Some(cookie) = cookie {
        headers.push(Header {
            name: "Cookie".to_string(),
            value: cookie.to_string(),
        });
    }

    HttpRequest {
        method: Method::Get,
        target: path.to_string(),
        path: path.to_string(),
        query: None,
        version: HttpVersion::Http11,
        headers,
        body: Vec::new(),
    }
}

fn config() -> Config {
    Config {
        servers: vec![ServerConfig {
            server_address: "127.0.0.1".to_string(),
            ports: vec![8080],
            server_name: vec!["localhost".to_string()],
            error_pages: HashMap::new(),
            client_max_body_size: 1024,
            routes: vec![RouteConfig {
                path: "/".to_string(),
                methods: vec!["GET".to_string()],
                root: Some("./www".to_string()),
                index: None,
                directory_listing: false,
                redirect: None,
                redirect_status: None,
                cgi: HashMap::new(),
            }],
        }],
    }
}

fn set_cookie_header(
    response: &localhost::http::HttpResponse,
) -> Option<String> {
    response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Set-Cookie"))
        .map(|(_, value)| value.clone())
}

#[test]
fn first_request_without_cookie_gets_a_new_session() {
    let config = config();
    let mut sessions = SessionStore::new();

    let response = expect_response(handle_request(
        &config,
        &request("/session-info", None),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut sessions,
    ));

    assert_eq!(response.status, StatusCode::Ok);

    let set_cookie =
        set_cookie_header(&response).expect("Set-Cookie should be present");

    assert!(set_cookie.contains("session_id="));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("Path=/"));

    let body = String::from_utf8(response.body).unwrap();
    assert!(body.contains("\"visit_count\":1"));
}

#[test]
fn request_with_valid_cookie_reuses_session_and_increments_count() {
    let config = config();
    let mut sessions = SessionStore::new();

    let first = expect_response(handle_request(
        &config,
        &request("/session-info", None),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut sessions,
    ));

    let set_cookie = set_cookie_header(&first).unwrap();

    let session_id = set_cookie
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let second = expect_response(handle_request(
        &config,
        &request("/session-info", Some(&session_id)),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut sessions,
    ));

    // No new session was created, so no Set-Cookie is expected again.
    assert!(set_cookie_header(&second).is_none());

    let body = String::from_utf8(second.body).unwrap();
    assert!(body.contains("\"visit_count\":2"));
}

#[test]
fn request_with_unknown_cookie_gets_a_fresh_session() {
    let config = config();
    let mut sessions = SessionStore::new();

    let response = expect_response(handle_request(
        &config,
        &request(
            "/session-info",
            Some("session_id=00000000000000000000000000000000"),
        ),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut sessions,
    ));

    let set_cookie =
        set_cookie_header(&response).expect("Set-Cookie should be present");

    assert!(!set_cookie.contains("00000000000000000000000000000000"));

    let body = String::from_utf8(response.body).unwrap();
    assert!(body.contains("\"visit_count\":1"));
}

#[test]
fn malformed_cookie_value_is_ignored_and_a_new_session_is_created() {
    let config = config();
    let mut sessions = SessionStore::new();

    let response = expect_response(handle_request(
        &config,
        &request("/session-info", Some("session_id=not-hex!!")),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut sessions,
    ));

    assert_eq!(response.status, StatusCode::Ok);
    assert!(set_cookie_header(&response).is_some());
}

#[test]
fn sessions_are_independent_across_different_cookies() {
    let config = config();
    let mut sessions = SessionStore::new();

    let a = expect_response(handle_request(
        &config,
        &request("/session-info", None),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut sessions,
    ));

    let b = expect_response(handle_request(
        &config,
        &request("/session-info", None),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut sessions,
    ));

    let cookie_a = set_cookie_header(&a).unwrap();
    let cookie_b = set_cookie_header(&b).unwrap();

    assert_ne!(cookie_a, cookie_b);

    let body_a = String::from_utf8(a.body).unwrap();
    let body_b = String::from_utf8(b.body).unwrap();

    assert!(body_a.contains("\"visit_count\":1"));
    assert!(body_b.contains("\"visit_count\":1"));
}
