//! Thin wrapper around the raw epoll syscalls.
//!
//! This is the ONLY file that calls `epoll_create1`, `epoll_ctl`, and
//! `epoll_wait`. Auditors should be able to find epoll initialization,
//! registration, and the wait call entirely within this file.

use std::io;
use std::os::unix::io::RawFd;

/// Interest flags for a registered fd. Kept as a thin newtype over the raw
/// bitmask so callers don't need to reach for `libc::EPOLLIN` directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interest(u32);

impl Interest {
    pub const READABLE: Interest = Interest(libc::EPOLLIN as u32);
    pub const WRITABLE: Interest = Interest(libc::EPOLLOUT as u32);

    pub fn bits(self) -> u32 {
        self.0
    }
}

impl std::ops::BitOr for Interest {
    type Output = Interest;
    fn bitor(self, rhs: Interest) -> Interest {
        Interest(self.0 | rhs.0)
    }
}

/// One event returned by `epoll_wait`, decoded into plain fields.
#[derive(Debug, Clone, Copy)]
pub struct EpollEvent {
    pub fd: RawFd,
    pub readable: bool,
    pub writable: bool,
    pub error: bool,
    pub hup: bool,
}

/// Owns the single epoll instance for the whole server.
///
/// Exactly one `Epoll` is created for the lifetime of the process (see
/// `main.rs`); this is the "one central multiplexing mechanism" required by
/// the audit checklist.
pub struct Epoll {
    epfd: RawFd,
}

impl Epoll {
    /// Creates the one epoll instance used by the entire server.
    pub fn new() -> io::Result<Epoll> {
        // SAFETY: epoll_create1 has no preconditions beyond a valid flags
        // argument; 0 means no special flags. We check the return value for
        // an error fd (-1) below.
        let epfd = unsafe { libc::epoll_create1(0) };
        if epfd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Epoll { epfd })
    }

    /// Registers `fd` with the given interest set. Called once per fd when
    /// it is first placed under multiplexing (listening sockets at startup,
    /// client sockets on accept).
    pub fn register(&self, fd: RawFd, interest: Interest) -> io::Result<()> {
        let mut ev = libc::epoll_event {
            events: interest.bits(),
            u64: fd as u64,
        };
        // SAFETY: `epfd` is a valid epoll fd owned by `self`; `ev` is a
        // valid, fully-initialized epoll_event on the stack for the
        // duration of the call.
        let ret = unsafe { libc::epoll_ctl(self.epfd, libc::EPOLL_CTL_ADD, fd, &mut ev) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Changes the interest set for an already-registered fd. Used to
    /// switch a client socket between EPOLLIN (waiting for a request) and
    /// EPOLLOUT (waiting to send a response).
    pub fn modify(&self, fd: RawFd, interest: Interest) -> io::Result<()> {
        let mut ev = libc::epoll_event {
            events: interest.bits(),
            u64: fd as u64,
        };
        // SAFETY: same as `register`; `fd` must already be registered,
        // which is a precondition documented on this method.
        let ret = unsafe { libc::epoll_ctl(self.epfd, libc::EPOLL_CTL_MOD, fd, &mut ev) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Deregisters `fd`. Must be called before the fd is closed, as part of
    /// client/listener cleanup.
    pub fn deregister(&self, fd: RawFd) -> io::Result<()> {
        // SAFETY: the kernel ignores the event pointer for EPOLL_CTL_DEL on
        // modern Linux, but older kernels require a non-null pointer; pass a
        // dummy zeroed event to stay portable.
        let mut ev = libc::epoll_event { events: 0, u64: 0 };
        let ret = unsafe { libc::epoll_ctl(self.epfd, libc::EPOLL_CTL_DEL, fd, &mut ev) };
        if ret < 0 {
            let err = io::Error::last_os_error();
            // ENOENT/EBADF mean the fd is already gone from the epoll set
            // (e.g. the kernel auto-removed it when the fd was closed
            // elsewhere) -- not fatal for cleanup purposes.
            if err.raw_os_error() == Some(libc::ENOENT) || err.raw_os_error() == Some(libc::EBADF)
            {
                return Ok(());
            }
            return Err(err);
        }
        Ok(())
    }

    /// The single central `epoll_wait` call. Blocks (with `timeout_ms`,
    /// or indefinitely if `None`) until at least one registered fd is
    /// ready, then returns the ready events. This is the only place
    /// `epoll_wait` is invoked.
    pub fn wait(&self, buf: &mut Vec<libc::epoll_event>, timeout_ms: i32) -> io::Result<usize> {
        loop {
            // SAFETY: `buf` is a valid, appropriately-sized buffer of
            // `epoll_event`; its capacity is passed as `maxevents`.
            let n = unsafe {
                libc::epoll_wait(
                    self.epfd,
                    buf.as_mut_ptr(),
                    buf.capacity() as i32,
                    timeout_ms,
                )
            };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    // EINTR: a signal interrupted the wait. Not a fatal
                    // error -- retry the wait rather than propagating.
                    continue;
                }
                return Err(err);
            }
            // SAFETY: the kernel guarantees the first `n` slots were
            // written by epoll_wait.
            unsafe { buf.set_len(n as usize) };
            return Ok(n as usize);
        }
    }
}

impl Drop for Epoll {
    fn drop(&mut self) {
        // SAFETY: `epfd` is a valid fd owned exclusively by this struct.
        unsafe {
            libc::close(self.epfd);
        }
    }
}

/// Decodes raw `libc::epoll_event`s from a wait call into `EpollEvent`s.
pub fn decode_events(raw: &[libc::epoll_event]) -> impl Iterator<Item = EpollEvent> + '_ {
    raw.iter().map(|ev| EpollEvent {
        fd: ev.u64 as RawFd,
        readable: ev.events & (libc::EPOLLIN as u32) != 0,
        writable: ev.events & (libc::EPOLLOUT as u32) != 0,
        error: ev.events & (libc::EPOLLERR as u32) != 0,
        hup: ev.events & (libc::EPOLLHUP as u32) != 0 || ev.events & (libc::EPOLLRDHUP as u32) != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::UnixStream;

    #[test]
    fn register_and_wait_reports_readable_event() {
        let epoll = Epoll::new().expect("create epoll instance");
        let (a, mut b) = UnixStream::pair().expect("create socketpair");
        a.set_nonblocking(true).unwrap();

        epoll
            .register(a.as_raw_fd(), Interest::READABLE)
            .expect("register fd");

        std::io::Write::write_all(&mut b, b"x").unwrap();

        let mut buf = Vec::with_capacity(8);
        let n = epoll.wait(&mut buf, 1000).expect("wait");
        assert_eq!(n, 1);

        let events: Vec<_> = decode_events(&buf).collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fd, a.as_raw_fd());
        assert!(events[0].readable);
    }

    #[test]
    fn deregister_stops_further_events() {
        let epoll = Epoll::new().expect("create epoll instance");
        let (a, mut b) = UnixStream::pair().expect("create socketpair");
        a.set_nonblocking(true).unwrap();

        epoll
            .register(a.as_raw_fd(), Interest::READABLE)
            .expect("register fd");
        epoll
            .deregister(a.as_raw_fd())
            .expect("deregister fd");

        std::io::Write::write_all(&mut b, b"x").unwrap();

        let mut buf = Vec::with_capacity(8);
        let n = epoll.wait(&mut buf, 100).expect("wait");
        assert_eq!(n, 0, "no events should be reported after deregister");
    }

    #[test]
    fn wait_times_out_with_no_ready_fds() {
        let epoll = Epoll::new().expect("create epoll instance");
        let (a, _b) = UnixStream::pair().expect("create socketpair");
        a.set_nonblocking(true).unwrap();
        epoll
            .register(a.as_raw_fd(), Interest::READABLE)
            .expect("register fd");

        let mut buf = Vec::with_capacity(8);
        let n = epoll.wait(&mut buf, 50).expect("wait");
        assert_eq!(n, 0);
    }
}
