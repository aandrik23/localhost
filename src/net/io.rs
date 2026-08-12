//! The ONLY place in the codebase that calls `read(2)`/`recv(2)` or
//! `write(2)`/`send(2)` on a client socket.
//!
//! Every function here performs exactly one syscall. Neither function loops
//! internally. Callers (the event loop / connection state machine) invoke
//! `read_once` at most once per readable event and `write_once` at most
//! once per writable event, then return control to `epoll_wait`. This is
//! what makes the "one read/write per client per epoll event" audit
//! requirement mechanically true rather than just a coding convention.

use std::io;
use std::os::unix::io::RawFd;

/// Outcome of a single non-blocking read attempt.
pub enum ReadOutcome {
    /// Read `n` bytes (n > 0) into the caller's buffer.
    Read(usize),
    /// Peer closed the connection (`read` returned 0).
    Closed,
    /// No data available right now (EAGAIN/EWOULDBLOCK). Not an error --
    /// wait for the next readable event.
    WouldBlock,
    /// Interrupted by a signal (EINTR). Not fatal -- caller should treat
    /// this event as a no-op and wait for the next one.
    Interrupted,
}

/// Outcome of a single non-blocking write attempt.
pub enum WriteOutcome {
    /// Wrote `n` bytes (n > 0).
    Written(usize),
    /// The socket buffer is full right now (EAGAIN/EWOULDBLOCK). Not an
    /// error -- wait for the next writable event.
    WouldBlock,
    /// Interrupted by a signal (EINTR). Not fatal.
    Interrupted,
}

/// Performs exactly one `read()` call on `fd` into `buf`. Does not loop.
pub fn read_once(fd: RawFd, buf: &mut [u8]) -> io::Result<ReadOutcome> {
    if buf.is_empty() {
        // Nothing requested; avoid a zero-length read syscall whose
        // semantics are ambiguous to reason about at the call sites.
        return Ok(ReadOutcome::WouldBlock);
    }
    // SAFETY: `buf` is a valid, writable slice of at least `buf.len()`
    // bytes; `fd` is a valid, open, non-blocking socket owned by the
    // caller for the duration of this call.
    let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };

    if n > 0 {
        return Ok(ReadOutcome::Read(n as usize));
    }
    if n == 0 {
        return Ok(ReadOutcome::Closed);
    }

    let err = io::Error::last_os_error();
    match err.raw_os_error() {
        // On Linux, EWOULDBLOCK and EAGAIN are the same constant, so
        // matching both here would be an unreachable-pattern warning; this
        // arm covers both by definition.
        Some(libc::EAGAIN) => Ok(ReadOutcome::WouldBlock),
        Some(libc::EINTR) => Ok(ReadOutcome::Interrupted),
        _ => Err(err),
    }
}

/// Performs exactly one `write()` call on `fd` from `buf`. Does not loop.
pub fn write_once(fd: RawFd, buf: &[u8]) -> io::Result<WriteOutcome> {
    if buf.is_empty() {
        return Ok(WriteOutcome::Written(0));
    }
    // SAFETY: `buf` is a valid, readable slice of at least `buf.len()`
    // bytes; `fd` is a valid, open, non-blocking socket owned by the
    // caller for the duration of this call.
    let n = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len()) };

    if n >= 0 {
        return Ok(WriteOutcome::Written(n as usize));
    }

    let err = io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EAGAIN) => Ok(WriteOutcome::WouldBlock),
        Some(libc::EINTR) => Ok(WriteOutcome::Interrupted),
        _ => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::UnixStream;

    fn nonblocking_pair() -> (UnixStream, UnixStream) {
        let (a, b) = UnixStream::pair().expect("create socketpair");
        a.set_nonblocking(true).expect("set nonblocking");
        b.set_nonblocking(true).expect("set nonblocking");
        (a, b)
    }

    #[test]
    fn read_once_returns_would_block_when_no_data() {
        let (a, _b) = nonblocking_pair();
        let mut buf = [0u8; 16];
        match read_once(a.as_raw_fd(), &mut buf).expect("read_once should not error") {
            ReadOutcome::WouldBlock => {}
            _ => panic!("expected WouldBlock on empty non-blocking socket"),
        }
    }

    #[test]
    fn read_once_reads_exactly_available_bytes_in_one_call() {
        let (a, mut b) = nonblocking_pair();
        std::io::Write::write_all(&mut b, b"hi").unwrap();

        let mut buf = [0u8; 16];
        match read_once(a.as_raw_fd(), &mut buf).expect("read_once should not error") {
            ReadOutcome::Read(n) => assert_eq!(&buf[..n], b"hi"),
            _ => panic!("expected Read(2)"),
        }
    }

    #[test]
    fn read_once_detects_peer_close() {
        let (a, b) = nonblocking_pair();
        drop(b);
        let mut buf = [0u8; 16];
        match read_once(a.as_raw_fd(), &mut buf).expect("read_once should not error") {
            ReadOutcome::Closed => {}
            _ => panic!("expected Closed after peer drop"),
        }
    }

    #[test]
    fn write_once_writes_in_a_single_call() {
        let (a, b) = nonblocking_pair();
        match write_once(a.as_raw_fd(), b"hello").expect("write_once should not error") {
            WriteOutcome::Written(n) => assert_eq!(n, 5),
            _ => panic!("expected Written(5)"),
        }
        drop(a);
        drop(b);
    }
}
