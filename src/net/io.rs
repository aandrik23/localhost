//! Single non-blocking client I/O operations.
//!
//! The event loop calls read_once or write_once at most once for each
//! readiness event. Neither function contains an internal retry loop.

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