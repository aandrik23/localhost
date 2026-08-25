//! Single non-blocking I/O operations.
//!
//! The event loop calls read_once or write_once at most once for each
//! readiness event. Neither function contains an internal retry loop.
//!
//! read_once/write_once operate on TcpStream (client sockets).
//! read_fd_once/write_fd_once operate on a raw file descriptor and
//! exist for CGI pipes, which are not sockets and have no Rust
//! standard-library wrapper with the same Read/Write ergonomics.

use std::io::{self, Read, Write};
use std::net::TcpStream;

pub enum ReadOutcome {
    Read(usize),
    Closed,
    WouldBlock,
    Interrupted,
}

pub enum WriteOutcome {
    Written(usize),
    WouldBlock,
    Interrupted,
}

/// Performs one read attempt only.
pub fn read_once(
    stream: &mut TcpStream,
    buf: &mut [u8],
) -> io::Result<ReadOutcome> {
    if buf.is_empty() {
        return Ok(ReadOutcome::WouldBlock);
    }

    match stream.read(buf) {
        Ok(0) => Ok(ReadOutcome::Closed),

        Ok(n) => Ok(ReadOutcome::Read(n)),

        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
            Ok(ReadOutcome::WouldBlock)
        }

        Err(err) if err.kind() == io::ErrorKind::Interrupted => {
            Ok(ReadOutcome::Interrupted)
        }

        Err(err) => Err(err),
    }
}

/// Performs one write attempt only.
pub fn write_once(
    stream: &mut TcpStream,
    buf: &[u8],
) -> io::Result<WriteOutcome> {
    if buf.is_empty() {
        return Ok(WriteOutcome::Written(0));
    }

    match stream.write(buf) {
        Ok(0) => Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "socket write returned zero",
        )),

        Ok(n) => Ok(WriteOutcome::Written(n)),

        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
            Ok(WriteOutcome::WouldBlock)
        }

        Err(err) if err.kind() == io::ErrorKind::Interrupted => {
            Ok(WriteOutcome::Interrupted)
        }

        Err(err) => Err(err),
    }
}

/// Performs one read attempt only, on a raw non-blocking file
/// descriptor (used for CGI stdout pipes).
///
/// The fd must already be set O_NONBLOCK by the caller; this
/// function does not set it. The single unsafe block wraps the
/// read(2) syscall itself - its safety invariant is that `fd` is a
/// valid, open, readable file descriptor for the duration of the
/// call, and `buf` is a valid buffer of at least `buf.len()` bytes,
/// both guaranteed by the caller owning the fd and passing a real
/// Rust slice.
#[cfg(unix)]
pub fn read_fd_once(
    fd: std::os::fd::RawFd,
    buf: &mut [u8],
) -> io::Result<ReadOutcome> {
    if buf.is_empty() {
        return Ok(ReadOutcome::WouldBlock);
    }

    let result = unsafe {
        libc::read(
            fd,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
        )
    };

    if result == 0 {
        return Ok(ReadOutcome::Closed);
    }

    if result > 0 {
        return Ok(ReadOutcome::Read(result as usize));
    }

    let err = io::Error::last_os_error();

    match err.kind() {
        io::ErrorKind::WouldBlock => Ok(ReadOutcome::WouldBlock),
        io::ErrorKind::Interrupted => Ok(ReadOutcome::Interrupted),
        _ => Err(err),
    }
}

/// Performs one write attempt only, on a raw non-blocking file
/// descriptor (used for CGI stdin pipes). Same safety invariant as
/// read_fd_once.
#[cfg(unix)]
pub fn write_fd_once(
    fd: std::os::fd::RawFd,
    buf: &[u8],
) -> io::Result<WriteOutcome> {
    if buf.is_empty() {
        return Ok(WriteOutcome::Written(0));
    }

    let result = unsafe {
        libc::write(
            fd,
            buf.as_ptr() as *const libc::c_void,
            buf.len(),
        )
    };

    if result > 0 {
        return Ok(WriteOutcome::Written(result as usize));
    }

    let err = io::Error::last_os_error();

    match err.kind() {
        io::ErrorKind::WouldBlock => Ok(WriteOutcome::WouldBlock),
        io::ErrorKind::Interrupted => Ok(WriteOutcome::Interrupted),
        _ => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener =
            TcpListener::bind(("127.0.0.1", 0)).expect("bind");

        let addr = listener.local_addr().unwrap();

        let client = TcpStream::connect(addr).expect("connect");

        let (server, _) = listener.accept().expect("accept");

        client.set_nonblocking(true).unwrap();
        server.set_nonblocking(true).unwrap();

        (server, client)
    }

    #[test]
    fn read_once_returns_would_block_when_no_data() {
        let (mut server, _client) = tcp_pair();

        let mut buf = [0u8; 16];

        match read_once(&mut server, &mut buf).unwrap() {
            ReadOutcome::WouldBlock => {}

            _ => panic!("expected WouldBlock"),
        }
    }

    #[test]
    fn read_once_reads_available_bytes() {
        let (mut server, mut client) = tcp_pair();

        client.write_all(b"hi").unwrap();

        let mut buf = [0u8; 16];

        loop {
            match read_once(&mut server, &mut buf).unwrap() {
                ReadOutcome::Read(n) => {
                    assert_eq!(&buf[..n], b"hi");
                    break;
                }

                ReadOutcome::WouldBlock => {
                    std::thread::yield_now();
                }

                ReadOutcome::Interrupted => {
                    continue;
                }

                ReadOutcome::Closed => {
                    panic!("connection closed before data was read");
                }
            }
        }
    }

    #[test]
    fn read_once_detects_peer_close() {
        let (mut server, client) = tcp_pair();

        drop(client);

        let mut buf = [0u8; 16];

        loop {
            match read_once(&mut server, &mut buf).unwrap() {
                ReadOutcome::Closed => break,

                ReadOutcome::WouldBlock => {
                    std::thread::yield_now();
                }

                _ => {}
            }
        }
    }

    #[test]
    fn write_once_writes_data() {
        let (mut server, _client) = tcp_pair();

        match write_once(&mut server, b"hello").unwrap() {
            WriteOutcome::Written(n) => {
                assert_eq!(n, 5);
            }

            _ => panic!("expected Written"),
        }
    }
}