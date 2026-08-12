//! Listening socket creation and client `accept()`.
//!
//! All sockets created here (listening and accepted client sockets) are set
//! non-blocking before being handed back to the caller.

use std::io;
use std::mem;
use std::net::Ipv4Addr;
use std::os::unix::io::RawFd;

/// A bound, listening, non-blocking TCP socket.
pub struct Listener {
    pub fd: RawFd,
    pub addr: Ipv4Addr,
    pub port: u16,
}

/// Sets the `O_NONBLOCK` flag on `fd`. Applied to every listening and
/// client socket in the server -- this is the single place that toggles
/// non-blocking mode, so it is easy to point to during an audit.
fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: `fd` is a valid, open file descriptor for the duration of
    // this call (guaranteed by callers).
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: same as above.
    let ret = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Creates, binds, and listens on `addr:port`, returning a non-blocking
/// listening socket ready to be registered with epoll.
pub fn bind_listener(addr: Ipv4Addr, port: u16) -> io::Result<Listener> {
    // SAFETY: standard socket() call with valid, constant arguments.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    // Allow immediate re-bind after restart (avoids "Address already in
    // use" from sockets lingering in TIME_WAIT).
    let optval: libc::c_int = 1;
    // SAFETY: `fd` was just created and is valid; `optval` lives for the
    // duration of the call and matches the expected size.
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &optval as *const _ as *const libc::c_void,
            mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if ret < 0 {
        let err = io::Error::last_os_error();
        // SAFETY: `fd` is a valid fd owned by us; closing on the error path
        // avoids leaking it.
        unsafe { libc::close(fd) };
        return Err(err);
    }

    if let Err(err) = set_nonblocking(fd) {
        // SAFETY: see above.
        unsafe { libc::close(fd) };
        return Err(err);
    }

    let sockaddr = libc::sockaddr_in {
        sin_family: libc::AF_INET as libc::sa_family_t,
        sin_port: port.to_be(),
        sin_addr: libc::in_addr {
            s_addr: u32::from(addr).to_be(),
        },
        sin_zero: [0; 8],
    };

    // SAFETY: `sockaddr` is a valid, fully-initialized sockaddr_in on the
    // stack; its size matches the third argument.
    let ret = unsafe {
        libc::bind(
            fd,
            &sockaddr as *const libc::sockaddr_in as *const libc::sockaddr,
            mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    if ret < 0 {
        let err = io::Error::last_os_error();
        // SAFETY: see above.
        unsafe { libc::close(fd) };
        return Err(err);
    }

    // Backlog: kernel default-caps this; 1024 is a reasonable ceiling for
    // pending connections awaiting accept().
    // SAFETY: `fd` is a valid, bound socket.
    let ret = unsafe { libc::listen(fd, 1024) };
    if ret < 0 {
        let err = io::Error::last_os_error();
        // SAFETY: see above.
        unsafe { libc::close(fd) };
        return Err(err);
    }

    Ok(Listener { fd, addr, port })
}

/// Result of a single `accept()` call.
pub enum AcceptResult {
    /// A new client connection was accepted.
    Accepted { fd: RawFd, peer: (Ipv4Addr, u16) },
    /// No pending connection right now (EAGAIN/EWOULDBLOCK). Not an error.
    WouldBlock,
    /// The accept call was interrupted by a signal (EINTR). Caller should
    /// simply wait for the next epoll event; not treated as fatal.
    Interrupted,
}

/// Performs exactly one `accept()` call on the listening socket `fd`. The
/// returned client socket is set non-blocking before being handed back.
pub fn accept_one(fd: RawFd) -> io::Result<AcceptResult> {
    let mut peer_addr: libc::sockaddr_in = unsafe { mem::zeroed() };
    let mut addr_len = mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;

    // SAFETY: `peer_addr`/`addr_len` are valid, correctly-sized out
    // parameters for accept(); `fd` is a valid listening socket owned by
    // the caller for the duration of this call.
    let client_fd = unsafe {
        libc::accept(
            fd,
            &mut peer_addr as *mut libc::sockaddr_in as *mut libc::sockaddr,
            &mut addr_len,
        )
    };

    if client_fd < 0 {
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EAGAIN) => return Ok(AcceptResult::WouldBlock),
            Some(libc::EINTR) => return Ok(AcceptResult::Interrupted),
            _ => return Err(err),
        }
    }

    if let Err(err) = set_nonblocking(client_fd) {
        // SAFETY: `client_fd` was just accepted and is owned by us; must
        // close it on this error path to avoid leaking the fd.
        unsafe { libc::close(client_fd) };
        return Err(err);
    }

    let ip = Ipv4Addr::from(u32::from_be(peer_addr.sin_addr.s_addr));
    let port = u16::from_be(peer_addr.sin_port);

    Ok(AcceptResult::Accepted {
        fd: client_fd,
        peer: (ip, port),
    })
}

/// Closes a raw socket fd. Centralized here so every close site is easy to
/// locate; used for both listener and client cleanup.
pub fn close_fd(fd: RawFd) {
    // SAFETY: caller guarantees `fd` is a valid, open fd that it owns and
    // will not use again after this call.
    unsafe {
        libc::close(fd);
    }
}
