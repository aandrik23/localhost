//! Phase 12 integration tests: two spec-called-out cases not covered
//! by earlier phase test files.
//!
//! - "multiple requests in one read" / persistent connections, tested
//!   through the real event loop for plain static-file GET requests
//!   (tests/cgi_phase9.rs already proves this for a CGI route, but
//!   nothing exercised it for the ordinary static-file path).
//! - Multiple Cookie header values / multiple cookies in one request,
//!   which src/http/parser.rs's cookie parsing supports but no test
//!   exercised end-to-end.

use std::collections::HashMap;
use std::io::{
    Read,
    Write,
};
use std::net::{
    Ipv4Addr,
    TcpStream,
};

use localhost::config::{
    Config,
    RouteConfig,
    ServerConfig,
};

use localhost::net::listener::bind_listener;
use localhost::server::event_loop::EventLoop;

fn test_config(root: String, port: u16) -> Config {
    Config {
        servers: vec![ServerConfig {
            server_address: "127.0.0.1".to_string(),
            ports: vec![port],
            server_name: vec!["localhost".to_string()],
            error_pages: HashMap::new(),
            client_max_body_size: 1024 * 1024,
            routes: vec![RouteConfig {
                path: "/".to_string(),
                methods: vec!["GET".to_string()],
                root: Some(root),
                index: Some("index.html".to_string()),
                directory_listing: false,
                redirect: None,
                redirect_status: None,
                cgi: HashMap::new(),
            }],
        }],
    }
}

fn start_loop(root: String) -> (EventLoop, u16) {
    let listener =
        bind_listener(Ipv4Addr::LOCALHOST, 0).expect("bind");

    let port = listener.port;

    let event_loop =
        EventLoop::new(vec![listener], test_config(root, port))
            .expect("event loop");

    (event_loop, port)
}

fn temporary_directory(label: &str) -> std::path::PathBuf {
    use std::time::{
        SystemTime,
        UNIX_EPOCH,
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let path = std::env::temp_dir()
        .join(format!("localhost-phase12-{}-{}", label, unique));

    std::fs::create_dir_all(&path).unwrap();

    path
}

fn read_available(
    stream: &mut TcpStream,
    event_loop: &mut EventLoop,
    max_ticks: usize,
) -> Vec<u8> {
    let mut received = Vec::new();
    let mut buf = [0u8; 4096];

    for _ in 0..max_ticks {
        event_loop.tick(50).expect("tick");

        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => received.extend_from_slice(&buf[..n]),

            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock => {}

            Err(err) => panic!("read error: {}", err),
        }
    }

    received
}

#[test]
fn two_plain_requests_in_one_read_are_both_answered() {
    let root = temporary_directory("multi-req");

    std::fs::write(root.join("index.html"), b"home page").unwrap();
    std::fs::write(root.join("second.txt"), b"second file").unwrap();

    let (mut event_loop, port) =
        start_loop(root.to_string_lossy().to_string());

    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).expect("connect");

    stream.set_nonblocking(true).expect("set_nonblocking");

    /*
     * Both requests are sent in a single TCP write. The parser must
     * not treat this one read as one HTTP message; it must recognize
     * two complete requests in the buffer and preserve the second
     * one for sequential processing rather than discarding it.
     */
    let pipelined = "GET / HTTP/1.1\r\nHost: localhost\r\n\r\nGET /second.txt HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

    stream
        .write_all(pipelined.as_bytes())
        .expect("write pipelined requests");

    let received = read_available(&mut stream, &mut event_loop, 200);
    let text = String::from_utf8_lossy(&received);

    let responses: Vec<&str> =
        text.split("HTTP/1.1 200 OK").collect();

    // split() on a 2-response body yields 3 pieces: before the first
    // match, between the two matches, and after the second - i.e. 2
    // occurrences of the status line.
    assert_eq!(
        responses.len(),
        3,
        "expected exactly two 200 OK responses, got: {:?}",
        text
    );

    assert!(
        text.contains("home page"),
        "missing first response body: {:?}",
        text
    );

    assert!(
        text.contains("second file"),
        "missing second response body: {:?}",
        text
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn multiple_cookies_in_one_request_are_all_parsed() {
    let root = temporary_directory("multi-cookie");

    std::fs::write(root.join("index.html"), b"home page").unwrap();

    let (mut event_loop, port) =
        start_loop(root.to_string_lossy().to_string());

    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).expect("connect");

    stream.set_nonblocking(true).expect("set_nonblocking");

    // A single Cookie header carrying multiple name=value pairs, as
    // real browsers send them, plus an unrelated existing session
    // cookie the server does not recognize.
    let request = "GET /session-info HTTP/1.1\r\nHost: localhost\r\nCookie: a=1; b=2; session_id=doesnotexist\r\nConnection: close\r\n\r\n";

    stream
        .write_all(request.as_bytes())
        .expect("write request");

    let received = read_available(&mut stream, &mut event_loop, 200);
    let text = String::from_utf8_lossy(&received);

    // The server must not crash or reject the request just because
    // multiple cookies (including one it doesn't recognize as a
    // session id) were sent in a single header; it should fall back
    // to creating a fresh session as usual.
    assert!(
        text.starts_with("HTTP/1.1 200 OK"),
        "expected 200 OK despite multiple cookies, got: {:?}",
        text
    );

    assert!(
        text.contains("\"session_id\""),
        "expected a session_id in the JSON body: {:?}",
        text
    );

    let _ = std::fs::remove_dir_all(root);
}
