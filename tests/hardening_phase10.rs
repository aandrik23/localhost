//! Phase 10 integration tests: idle connection timeouts and the
//! simultaneous-connection cap, driven through a real EventLoop over
//! real TCP sockets.

use std::collections::HashMap;
use std::io::{
    Read,
    Write,
};
use std::net::{
    Ipv4Addr,
    TcpStream,
};
use std::time::Duration;

use localhost::config::{
    Config,
    RouteConfig,
    ServerConfig,
};

use localhost::net::listener::bind_listener;
use localhost::server::event_loop::{
    EventLoop,
    CONNECTION_IDLE_TIMEOUT,
};

/// A small cap used by connection_cap_is_enforced so the test can
/// exercise the cap logic without opening a thousand-plus real
/// sockets, which is unreliable and slow. The mechanism under test
/// (EventLoop::with_max_connections / the accept-skip in
/// handle_listener_event) is identical regardless of the configured
/// number.
const TEST_MAX_CONNECTIONS: usize = 5;

fn test_config() -> Config {
    Config {
        servers: vec![ServerConfig {
            server_address: "127.0.0.1".to_string(),
            ports: vec![8080],
            server_name: vec!["localhost".to_string()],
            error_pages: HashMap::new(),
            client_max_body_size: 1024 * 1024,
            routes: vec![RouteConfig {
                path: "/".to_string(),
                methods: vec!["GET".to_string()],
                root: Some("./www".to_string()),
                index: Some("index.html".to_string()),
                directory_listing: false,
                redirect: None,
                redirect_status: None,
                cgi: HashMap::new(),
            }],
        }],
    }
}

fn start_loop() -> (EventLoop, u16) {
    let listener =
        bind_listener(Ipv4Addr::LOCALHOST, 0).expect("bind");

    let port = listener.port;

    let event_loop =
        EventLoop::new(vec![listener], test_config())
            .expect("event loop");

    (event_loop, port)
}

fn start_loop_with_cap(max_connections: usize) -> (EventLoop, u16) {
    let listener =
        bind_listener(Ipv4Addr::LOCALHOST, 0).expect("bind");

    let port = listener.port;

    let event_loop = EventLoop::with_max_connections(
        vec![listener],
        test_config(),
        max_connections,
    )
    .expect("event loop");

    (event_loop, port)
}

fn drive_for(
    event_loop: &mut EventLoop,
    duration: Duration,
    tick_timeout_ms: i32,
) {
    let deadline = std::time::Instant::now() + duration;

    while std::time::Instant::now() < deadline {
        event_loop
            .tick(tick_timeout_ms)
            .expect("tick should not error");
    }
}

fn drive_until(
    event_loop: &mut EventLoop,
    max_ticks: usize,
    tick_timeout_ms: i32,
    mut pred: impl FnMut(&EventLoop) -> bool,
) {
    for _ in 0..max_ticks {
        if pred(event_loop) {
            return;
        }

        event_loop
            .tick(tick_timeout_ms)
            .expect("tick should not error");
    }

    assert!(pred(event_loop), "condition not met within max_ticks");
}

#[test]
fn fully_idle_connection_is_closed_after_timeout() {
    let (mut event_loop, port) = start_loop();

    let stream =
        TcpStream::connect(("127.0.0.1", port)).expect("connect");

    drive_until(
        &mut event_loop,
        50,
        50,
        |ev| ev.connection_count() == 1,
    );

    // Send nothing at all; just wait past the idle timeout.
    drive_for(
        &mut event_loop,
        CONNECTION_IDLE_TIMEOUT + Duration::from_secs(2),
        200,
    );

    assert_eq!(
        event_loop.connection_count(),
        0,
        "idle connection should have been closed after the timeout"
    );

    drop(stream);
}

#[test]
fn incomplete_request_gets_408_after_timeout() {
    let (mut event_loop, port) = start_loop();

    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).expect("connect");

    stream.set_nonblocking(true).expect("set_nonblocking");

    // Send only a partial request line/headers, never completing it.
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n")
        .expect("write partial request");

    drive_for(
        &mut event_loop,
        CONNECTION_IDLE_TIMEOUT + Duration::from_secs(2),
        200,
    );

    let mut received = Vec::new();
    let mut buf = [0u8; 4096];

    match stream.read(&mut buf) {
        Ok(0) => {}
        Ok(n) => received.extend_from_slice(&buf[..n]),
        Err(_) => {}
    }

    let text = String::from_utf8_lossy(&received);

    assert!(
        text.starts_with("HTTP/1.1 408"),
        "expected a 408 Request Timeout response, got: {:?}",
        text
    );

    assert!(
        text.contains("Connection: close"),
        "expected the timeout response to close the connection: {:?}",
        text
    );
}

#[test]
fn active_connection_is_not_closed_by_idle_sweep() {
    let (mut event_loop, port) = start_loop();

    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).expect("connect");

    stream.set_nonblocking(true).expect("set_nonblocking");

    let deadline = std::time::Instant::now()
        + CONNECTION_IDLE_TIMEOUT
        + Duration::from_secs(2);

    // Keep sending small amounts of activity (invalid requests are
    // fine - the point is only to keep last_activity moving) well
    // past what would otherwise be the idle timeout.
    while std::time::Instant::now() < deadline {
        let _ = stream.write_all(b" ");

        event_loop.tick(200).expect("tick");

        std::thread::sleep(Duration::from_millis(300));
    }

    assert!(
        event_loop.connection_count() >= 1,
        "a connection that keeps producing activity should not be \
         closed by the idle sweep"
    );
}

#[test]
fn connection_cap_is_enforced() {
    let (mut event_loop, port) =
        start_loop_with_cap(TEST_MAX_CONNECTIONS);

    // Fill up to the cap. Keep every stream alive so its connection
    // stays counted.
    let mut streams = Vec::new();

    for _ in 0..TEST_MAX_CONNECTIONS {
        streams.push(
            TcpStream::connect(("127.0.0.1", port))
                .expect("connect"),
        );
    }

    drive_until(
        &mut event_loop,
        200,
        50,
        |ev| ev.connection_count() == TEST_MAX_CONNECTIONS,
    );

    assert_eq!(
        event_loop.connection_count(),
        TEST_MAX_CONNECTIONS
    );

    // One more connection attempt should not be accepted while at
    // the cap.
    let extra =
        TcpStream::connect(("127.0.0.1", port)).expect("connect");

    for _ in 0..20 {
        event_loop.tick(20).expect("tick");
    }

    assert_eq!(
        event_loop.connection_count(),
        TEST_MAX_CONNECTIONS,
        "connection count should not exceed the configured cap"
    );

    // Freeing one connection should allow the extra one in.
    drop(streams.pop());

    drive_until(
        &mut event_loop,
        100,
        50,
        |ev| ev.connection_count() == TEST_MAX_CONNECTIONS,
    );

    assert_eq!(event_loop.connection_count(), TEST_MAX_CONNECTIONS);

    drop(extra);
    drop(streams);
}
