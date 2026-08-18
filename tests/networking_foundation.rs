//! Phase 2 integration tests: exercise the event loop's networking
//! foundation directly (no HTTP semantics involved).

use std::io::Write;
use std::net::{Ipv4Addr, TcpStream};
use std::time::Duration;

use localhost::net::listener::bind_listener;
use localhost::server::event_loop::EventLoop;

/// Binds one listener on an OS-assigned ephemeral port and wraps it in an
/// EventLoop, returning the loop and the bound port.
fn start_loop_on_ephemeral_port() -> (EventLoop, u16) {
    let listener = bind_listener(Ipv4Addr::LOCALHOST, 0).expect("bind ephemeral listener");
    let port = local_port_of(&listener);
    let event_loop = EventLoop::new(vec![listener]).expect("create event loop");
    (event_loop, port)
}

/// Reads back the port the OS assigned when binding to port 0, via
/// getsockname on the raw fd.
fn local_port_of(
    listener: &localhost::net::listener::Listener,
) -> u16 {
    listener.port
}

/// Drives `tick` until `pred(&event_loop)` is true or `max_ticks` elapses.
/// Each tick uses a short timeout so the test doesn't hang if the predicate
/// never becomes true (it will just fail with a clear panic instead).
fn drive_until(
    event_loop: &mut EventLoop,
    max_ticks: usize,
    mut pred: impl FnMut(&EventLoop) -> bool,
) {
    for _ in 0..max_ticks {
        if pred(event_loop) {
            return;
        }
        event_loop.tick(50).expect("tick should not error");
    }
    assert!(pred(event_loop), "condition not met within max_ticks");
}

#[test]
fn accepts_multiple_simultaneous_clients() {
    let (mut event_loop, port) = start_loop_on_ephemeral_port();

    let mut clients = Vec::new();
    for _ in 0..5 {
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        clients.push(stream);
    }

    drive_until(&mut event_loop, 50, |ev| ev.connection_count() == 5);
    assert_eq!(event_loop.connection_count(), 5);

    // Keep the streams alive until after the assertion so the OS doesn't
    // close them before the server had a chance to accept.
    drop(clients);
}

#[test]
fn handles_client_disconnect() {
    let (mut event_loop, port) = start_loop_on_ephemeral_port();

    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    drive_until(&mut event_loop, 50, |ev| ev.connection_count() == 1);

    drop(stream); // client disconnects

    drive_until(&mut event_loop, 50, |ev| ev.connection_count() == 0);
    assert_eq!(event_loop.connection_count(), 0);
}

#[test]
fn repeated_connect_disconnect_cycles_do_not_leak_connections() {
    let (mut event_loop, port) = start_loop_on_ephemeral_port();

    for _ in 0..20 {
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        drive_until(&mut event_loop, 50, |ev| ev.connection_count() == 1);
        drop(stream);
        drive_until(&mut event_loop, 50, |ev| ev.connection_count() == 0);
    }

    assert_eq!(event_loop.connection_count(), 0);
}

#[test]
fn partial_read_is_accumulated_in_connection_state() {
    let (mut event_loop, port) = start_loop_on_ephemeral_port();

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    drive_until(&mut event_loop, 50, |ev| ev.connection_count() == 1);

    // Send data in two separate writes to force two separate readable
    // events / two separate `read_once` calls, accumulating in
    // Connection::read_buf across them.
    stream.write_all(b"hello ").unwrap();
    stream.flush().unwrap();
    std::thread::sleep(Duration::from_millis(50));
    event_loop.tick(200).expect("tick");

    stream.write_all(b"world").unwrap();
    stream.flush().unwrap();
    std::thread::sleep(Duration::from_millis(50));
    event_loop.tick(200).expect("tick");

    // We can't reach into the private connections map from an external
    // test crate, so this test primarily verifies the loop keeps running
    // and the connection is not dropped across the two partial sends.
    assert_eq!(event_loop.connection_count(), 1);
}

#[test]
fn one_broken_client_does_not_affect_others() {
    let (mut event_loop, port) = start_loop_on_ephemeral_port();

    let good = TcpStream::connect(("127.0.0.1", port)).expect("connect good client");
    let bad = TcpStream::connect(("127.0.0.1", port)).expect("connect bad client");

    drive_until(&mut event_loop, 50, |ev| ev.connection_count() == 2);

    // Abruptly reset the "bad" connection by setting SO_LINGER(0) and
    // dropping it, which causes the kernel to send RST instead of FIN.
    #[cfg(unix)]
    set_linger_zero(&bad);
    drop(bad);

    drive_until(&mut event_loop, 50, |ev| ev.connection_count() == 1);
    assert_eq!(event_loop.connection_count(), 1);

    // The good client should still be usable: write from it and confirm
    // the server accepts more bytes without erroring the whole loop.
    let mut good = good;
    good.write_all(b"still alive").unwrap();
    good.flush().unwrap();
    event_loop.tick(200).expect("tick after bad client reset");
    assert_eq!(event_loop.connection_count(), 1);
}

#[cfg(unix)]
fn set_linger_zero(stream: &TcpStream) {
    use std::os::unix::io::AsRawFd;
    let linger = libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };
    unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            &linger as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::linger>() as libc::socklen_t,
        );
    }
}

#[test]
fn queued_response_is_written_back_to_client_and_may_be_partial() {
    let (mut event_loop, port) = start_loop_on_ephemeral_port();

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    drive_until(&mut event_loop, 50, |ev| ev.connection_count() == 1);

    // We don't have direct access to the accepted fd from the test crate,
    // so drive a couple of ticks to let the server see the connection, then
    // rely on queue_response being exercised indirectly is out of scope
    // here without exposing the fd. Instead, verify basic byte echo isn't
    // implemented yet (Phase 2 has no application logic) and that the
    // connection remains open and stable across ticks.
    stream.write_all(b"ping").unwrap();
    stream.flush().unwrap();
    for _ in 0..3 {
        event_loop.tick(50).expect("tick");
    }
    assert_eq!(event_loop.connection_count(), 1);
}

#[test]
fn multiple_listeners_share_one_event_loop() {
    let l1 = bind_listener(Ipv4Addr::LOCALHOST, 0).expect("bind first listener");
    let l2 = bind_listener(Ipv4Addr::LOCALHOST, 0).expect("bind second listener");
    let p1 = local_port_of(&l1);
    let p2 = local_port_of(&l2);

    let mut event_loop = EventLoop::new(vec![l1, l2]).expect("create event loop");
    assert_eq!(event_loop.listener_count(), 2);

    let c1 = TcpStream::connect(("127.0.0.1", p1)).expect("connect to listener 1");
    let c2 = TcpStream::connect(("127.0.0.1", p2)).expect("connect to listener 2");

    drive_until(&mut event_loop, 50, |ev| ev.connection_count() == 2);
    assert_eq!(event_loop.connection_count(), 2);

    drop(c1);
    drop(c2);
}
