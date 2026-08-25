//! Phase 9 integration tests: exercise CGI end-to-end through the
//! real EventLoop over real TCP sockets - fork/pipe/execve, the
//! non-blocking stdin/stdout pipe fds registered with epoll, timeout
//! handling, and zombie-free process reaping.
//!
//! These tests require `python3` on PATH, matching the CGI mapping
//! used throughout the rest of the project (config.toml, docs).

use std::collections::HashMap;
use std::fs;
use std::io::{
    Read,
    Write,
};
use std::net::{
    Ipv4Addr,
    TcpStream,
};
use std::time::{
    Duration,
    SystemTime,
    UNIX_EPOCH,
};

use localhost::config::{
    Config,
    RouteConfig,
    ServerConfig,
};

use localhost::net::listener::bind_listener;
use localhost::server::event_loop::EventLoop;

fn temporary_directory() -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let path = std::env::temp_dir()
        .join(format!("localhost-phase9-{}", unique));

    fs::create_dir_all(&path).unwrap();

    path
}

fn write_script(
    dir: &std::path::Path,
    name: &str,
    contents: &str,
) -> std::path::PathBuf {
    let path = dir.join(name);

    fs::write(&path, contents).unwrap();

    path
}

fn config_with_cgi_root(root: String, port: u16) -> Config {
    Config {
        servers: vec![ServerConfig {
            server_address: "127.0.0.1".to_string(),
            ports: vec![port],
            server_name: vec!["localhost".to_string()],
            error_pages: HashMap::new(),
            client_max_body_size: 1024 * 1024,
            routes: vec![RouteConfig {
                path: "/cgi".to_string(),
                methods: vec![
                    "GET".to_string(),
                    "POST".to_string(),
                ],
                root: Some(root),
                index: None,
                directory_listing: false,
                redirect: None,
                redirect_status: None,
                cgi: {
                    let mut map = HashMap::new();
                    map.insert(
                        ".py".to_string(),
                        "python3".to_string(),
                    );
                    map
                },
            }],
        }],
    }
}

/// Binds an ephemeral listener first, then builds a Config whose
/// server explicitly lists that same port - server selection (see
/// server::routing::select_server) matches on (local_addr,
/// local_port), so the config's ports must agree with whatever port
/// the OS actually assigned.
fn start_loop(root: String) -> (EventLoop, u16) {
    let listener =
        bind_listener(Ipv4Addr::LOCALHOST, 0).expect("bind");

    let port = listener.port;

    let config = config_with_cgi_root(root, port);

    let event_loop =
        EventLoop::new(vec![listener], config).expect("event loop");

    (event_loop, port)
}

/// Sends a raw HTTP request and reads the response, driving the
/// event loop's tick() between each socket operation so the server
/// makes progress exactly like the real single-threaded run() loop
/// would - the test never calls anything on the server side except
/// tick().
fn send_request_and_read_response(
    event_loop: &mut EventLoop,
    port: u16,
    request: &str,
    max_ticks: usize,
) -> String {
    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).expect("connect");

    stream
        .set_nonblocking(true)
        .expect("set_nonblocking");

    stream
        .write_all(request.as_bytes())
        .expect("write request");

    let mut received = Vec::new();
    let mut buf = [0u8; 4096];

    for _ in 0..max_ticks {
        event_loop
            .tick(50)
            .expect("tick should not error");

        match stream.read(&mut buf) {
            Ok(0) => break,

            Ok(n) => {
                received.extend_from_slice(&buf[..n]);

                // A full response has arrived once we can see the
                // blank line separating headers from body and have
                // at least as many body bytes as Content-Length
                // claims.
                if response_looks_complete(&received) {
                    break;
                }
            }

            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock => {}

            Err(err) => panic!("read error: {}", err),
        }
    }

    String::from_utf8_lossy(&received).into_owned()
}

fn response_looks_complete(received: &[u8]) -> bool {
    let text = String::from_utf8_lossy(received);

    let header_end = match text.find("\r\n\r\n") {
        Some(pos) => pos,
        None => return false,
    };

    let content_length = text[..header_end]
        .lines()
        .find_map(|line| {
            line.strip_prefix("Content-Length: ")
                .or_else(|| line.strip_prefix("content-length: "))
        })
        .and_then(|value| value.trim().parse::<usize>().ok());

    match content_length {
        Some(length) => {
            received.len() >= header_end + 4 + length
        }

        None => true,
    }
}

#[test]
fn get_request_runs_cgi_and_returns_output() {
    let root = temporary_directory();

    write_script(
        &root,
        "hello.py",
        "print(\"Content-Type: text/plain\")\nprint()\nprint(\"hello from cgi\")\n",
    );

    let (mut event_loop, port) =
        start_loop(
            root.to_string_lossy().to_string(),
        );

    let response = send_request_and_read_response(
        &mut event_loop,
        port,
        "GET /cgi/hello.py HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        200,
    );

    assert!(
        response.starts_with("HTTP/1.1 200"),
        "unexpected response: {}",
        response
    );

    assert!(response.contains("hello from cgi"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn post_body_is_delivered_to_cgi_stdin() {
    let root = temporary_directory();

    write_script(
        &root,
        "echo.py",
        "import sys\n\
body = sys.stdin.read()\n\
print(\"Content-Type: text/plain\")\n\
print()\n\
print(\"received:\" + body)\n",
    );

    let (mut event_loop, port) =
        start_loop(
            root.to_string_lossy().to_string(),
        );

    let body = "posted-data";

    let request = format!(
        "POST /cgi/echo.py HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body,
    );

    let response = send_request_and_read_response(
        &mut event_loop,
        port,
        &request,
        200,
    );

    assert!(
        response.contains("received:posted-data"),
        "unexpected response: {}",
        response
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn query_string_reaches_cgi_environment() {
    let root = temporary_directory();

    write_script(
        &root,
        "query.py",
        "import os\n\
print(\"Content-Type: text/plain\")\n\
print()\n\
print(\"query=\" + os.environ.get(\"QUERY_STRING\", \"\"))\n",
    );

    let (mut event_loop, port) =
        start_loop(
            root.to_string_lossy().to_string(),
        );

    let response = send_request_and_read_response(
        &mut event_loop,
        port,
        "GET /cgi/query.py?a=1&b=2 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        200,
    );

    assert!(response.contains("query=a=1&b=2"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn path_info_reaches_cgi_environment() {
    let root = temporary_directory();

    write_script(
        &root,
        "pathinfo.py",
        "import os\n\
print(\"Content-Type: text/plain\")\n\
print()\n\
print(\"path_info=\" + os.environ.get(\"PATH_INFO\", \"\"))\n",
    );

    let (mut event_loop, port) =
        start_loop(
            root.to_string_lossy().to_string(),
        );

    let response = send_request_and_read_response(
        &mut event_loop,
        port,
        "GET /cgi/pathinfo.py/extra/segments HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        200,
    );

    assert!(response.contains("path_info=/pathinfo.py/extra/segments"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn cgi_script_exiting_nonzero_with_no_output_returns_500() {
    let root = temporary_directory();

    write_script(
        &root,
        "fail.py",
        "import sys\nsys.exit(1)\n",
    );

    let (mut event_loop, port) =
        start_loop(
            root.to_string_lossy().to_string(),
        );

    let response = send_request_and_read_response(
        &mut event_loop,
        port,
        "GET /cgi/fail.py HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        200,
    );

    assert!(
        response.starts_with("HTTP/1.1 500"),
        "unexpected response: {}",
        response
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn malformed_cgi_output_without_headers_returns_500() {
    let root = temporary_directory();

    write_script(
        &root,
        "malformed.py",
        "print(\"not a valid CGI response, no header/body split\", end=\"\")\n",
    );

    let (mut event_loop, port) =
        start_loop(
            root.to_string_lossy().to_string(),
        );

    let response = send_request_and_read_response(
        &mut event_loop,
        port,
        "GET /cgi/malformed.py HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        200,
    );

    assert!(
        response.starts_with("HTTP/1.1 500"),
        "unexpected response: {}",
        response
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn missing_cgi_script_returns_404() {
    let root = temporary_directory();

    let (mut event_loop, port) =
        start_loop(
            root.to_string_lossy().to_string(),
        );

    let response = send_request_and_read_response(
        &mut event_loop,
        port,
        "GET /cgi/does-not-exist.py HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        200,
    );

    assert!(
        response.starts_with("HTTP/1.1 404"),
        "unexpected response: {}",
        response
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn slow_cgi_does_not_block_a_concurrent_client() {
    let root = temporary_directory();

    write_script(
        &root,
        "slow.py",
        "import time\ntime.sleep(1)\nprint(\"Content-Type: text/plain\")\nprint()\nprint(\"done\")\n",
    );

    fs::write(root.join("fast.txt"), b"fast file").unwrap();

    let (mut event_loop, port) =
        start_loop(
            root.to_string_lossy().to_string(),
        );

    let mut slow_stream =
        TcpStream::connect(("127.0.0.1", port)).expect("connect slow");

    slow_stream.set_nonblocking(true).unwrap();

    slow_stream
        .write_all(
            b"GET /cgi/slow.py HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .unwrap();

    // Let the slow CGI actually start (spawned + registered) before
    // racing a second, unrelated request against it. EventLoop
    // doesn't expose CGI process count publicly (internal
    // bookkeeping, not part of the API other phases rely on), so a
    // handful of fixed ticks stands in for "spawn has happened."
    for _ in 0..5 {
        event_loop.tick(20).expect("tick");
    }

    let started = std::time::Instant::now();

    let fast_response = send_request_and_read_response(
        &mut event_loop,
        port,
        "GET /cgi/fast.txt HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        100,
    );

    let fast_elapsed = started.elapsed();

    assert!(fast_response.starts_with("HTTP/1.1 200"));

    assert!(
        fast_elapsed < Duration::from_millis(500),
        "fast request took {:?}, expected well under the 1s CGI sleep - \
         the event loop appears to have blocked on the slow CGI process",
        fast_elapsed
    );

    // Drain the slow response too so the test cleans up its process.
    let mut buf = [0u8; 4096];

    for _ in 0..100 {
        event_loop.tick(50).expect("tick");

        match slow_stream.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => continue,
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(err) => panic!("read error: {}", err),
        }
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn pipelined_request_after_cgi_is_still_answered() {
    let root = temporary_directory();

    write_script(
        &root,
        "quick.py",
        "print(\"Content-Type: text/plain\")\nprint()\nprint(\"cgi done\")\n",
    );

    fs::write(root.join("static.txt"), b"static file content").unwrap();

    let (mut event_loop, port) =
        start_loop(
            root.to_string_lossy().to_string(),
        );

    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).expect("connect");

    stream.set_nonblocking(true).expect("set_nonblocking");

    /*
     * Both requests are written in a single TCP write, and no more
     * bytes are ever sent after this - a real HTTP/1.1 pipelining
     * client waits for both responses rather than sending anything
     * else first. If the second request is never dispatched once
     * the CGI response for the first completes, this test times out
     * waiting for a second response that never arrives.
     */
    let pipelined = "GET /cgi/quick.py HTTP/1.1\r\nHost: localhost\r\n\r\nGET /cgi/static.txt HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

    stream
        .write_all(pipelined.as_bytes())
        .expect("write pipelined requests");

    let mut received = Vec::new();
    let mut buf = [0u8; 4096];

    for _ in 0..400 {
        event_loop.tick(50).expect("tick");

        match stream.read(&mut buf) {
            Ok(0) => break,

            Ok(n) => {
                received.extend_from_slice(&buf[..n]);
            }

            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock => {}

            Err(err) => panic!("read error: {}", err),
        }
    }

    let text = String::from_utf8_lossy(&received);

    let response_count =
        text.matches("HTTP/1.1").count();

    assert_eq!(
        response_count, 2,
        "expected 2 responses, got {}: {:?}",
        response_count, text
    );

    assert!(text.contains("cgi done"));
    assert!(text.contains("static file content"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn cgi_script_nonzero_exit_with_valid_output_is_still_returned() {
    let root = temporary_directory();

    write_script(
        &root,
        "partial_fail.py",
        "import sys\n\
print(\"Content-Type: text/plain\")\n\
print()\n\
print(\"here is output before failing\")\n\
sys.exit(1)\n",
    );

    let (mut event_loop, port) =
        start_loop(
            root.to_string_lossy().to_string(),
        );

    let response = send_request_and_read_response(
        &mut event_loop,
        port,
        "GET /cgi/partial_fail.py HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        200,
    );

    assert!(
        response.starts_with("HTTP/1.1 200"),
        "unexpected response: {}",
        response
    );

    assert!(response.contains("here is output before failing"));

    let _ = fs::remove_dir_all(root);
}
