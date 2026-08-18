//! Cross-platform event multiplexer.
//!
//! Linux   -> epoll
//! macOS   -> kqueue
//! Windows -> WSAPoll
//!
//! The rest of the server uses the exact same interface regardless
//! of operating system.

use std::io;

use crate::net::socket::SocketId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interest(u8);

impl Interest {
    pub const READABLE: Interest = Interest(0b01);
    pub const WRITABLE: Interest = Interest(0b10);

    pub fn contains(self, other: Interest) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOr for Interest {
    type Output = Interest;

    fn bitor(self, rhs: Interest) -> Interest {
        Interest(self.0 | rhs.0)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EpollEvent {
    pub fd: SocketId,
    pub readable: bool,
    pub writable: bool,
    pub error: bool,
    pub hup: bool,
}

/* ============================================================
   LINUX
   ============================================================ */

#[cfg(target_os = "linux")]
pub struct Epoll {
    epfd: std::os::fd::RawFd,
}

#[cfg(target_os = "linux")]
impl Epoll {
    pub fn new() -> io::Result<Self> {
        let epfd = unsafe { libc::epoll_create1(0) };

        if epfd < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self { epfd })
    }

    fn flags(interest: Interest) -> u32 {
        let mut flags = libc::EPOLLRDHUP as u32;

        if interest.contains(Interest::READABLE) {
            flags |= libc::EPOLLIN as u32;
        }

        if interest.contains(Interest::WRITABLE) {
            flags |= libc::EPOLLOUT as u32;
        }

        flags
    }

    pub fn register(
        &mut self,
        fd: SocketId,
        interest: Interest,
    ) -> io::Result<()> {
        let mut event = libc::epoll_event {
            events: Self::flags(interest),
            u64: fd as u64,
        };

        let result = unsafe {
            libc::epoll_ctl(
                self.epfd,
                libc::EPOLL_CTL_ADD,
                fd,
                &mut event,
            )
        };

        if result < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    pub fn modify(
        &mut self,
        fd: SocketId,
        interest: Interest,
    ) -> io::Result<()> {
        let mut event = libc::epoll_event {
            events: Self::flags(interest),
            u64: fd as u64,
        };

        let result = unsafe {
            libc::epoll_ctl(
                self.epfd,
                libc::EPOLL_CTL_MOD,
                fd,
                &mut event,
            )
        };

        if result < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    pub fn deregister(
        &mut self,
        fd: SocketId,
    ) -> io::Result<()> {
        let mut event = libc::epoll_event {
            events: 0,
            u64: 0,
        };

        let result = unsafe {
            libc::epoll_ctl(
                self.epfd,
                libc::EPOLL_CTL_DEL,
                fd,
                &mut event,
            )
        };

        if result < 0 {
            let err = io::Error::last_os_error();

            if matches!(
                err.raw_os_error(),
                Some(libc::ENOENT) | Some(libc::EBADF)
            ) {
                return Ok(());
            }

            return Err(err);
        }

        Ok(())
    }

    pub fn wait(
        &mut self,
        events: &mut Vec<EpollEvent>,
        timeout_ms: i32,
    ) -> io::Result<usize> {
        events.clear();

        let capacity = events.capacity().max(1);

        let mut raw =
            Vec::<libc::epoll_event>::with_capacity(capacity);

        let count = unsafe {
            libc::epoll_wait(
                self.epfd,
                raw.as_mut_ptr(),
                capacity as i32,
                timeout_ms,
            )
        };

        if count < 0 {
            let err = io::Error::last_os_error();

            if err.kind() == io::ErrorKind::Interrupted {
                return Ok(0);
            }

            return Err(err);
        }

        unsafe {
            raw.set_len(count as usize);
        }

        for event in raw {
            let flags = event.events;

            events.push(EpollEvent {
                fd: event.u64 as SocketId,

                readable:
                    flags & libc::EPOLLIN as u32 != 0,

                writable:
                    flags & libc::EPOLLOUT as u32 != 0,

                error:
                    flags & libc::EPOLLERR as u32 != 0,

                hup:
                    flags & libc::EPOLLHUP as u32 != 0
                        || flags & libc::EPOLLRDHUP as u32 != 0,
            });
        }

        Ok(events.len())
    }
}

#[cfg(target_os = "linux")]
impl Drop for Epoll {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.epfd);
        }
    }
}

/* ============================================================
   MACOS
   ============================================================ */

#[cfg(target_os = "macos")]
pub struct Epoll {
    kqueue_fd: std::os::fd::RawFd,
}

#[cfg(target_os = "macos")]
impl Epoll {
    pub fn new() -> io::Result<Self> {
        let fd = unsafe { libc::kqueue() };

        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            kqueue_fd: fd,
        })
    }

    fn kevent(
        fd: SocketId,
        filter: i16,
        flags: u16,
    ) -> libc::kevent {
        libc::kevent {
            ident: fd as libc::uintptr_t,
            filter,
            flags,
            fflags: 0,
            data: 0,
            udata: std::ptr::null_mut(),
        }
    }

    pub fn register(
        &mut self,
        fd: SocketId,
        interest: Interest,
    ) -> io::Result<()> {
        let read_flags =
            if interest.contains(Interest::READABLE) {
                libc::EV_ADD | libc::EV_ENABLE
            } else {
                libc::EV_ADD | libc::EV_DISABLE
            };

        let write_flags =
            if interest.contains(Interest::WRITABLE) {
                libc::EV_ADD | libc::EV_ENABLE
            } else {
                libc::EV_ADD | libc::EV_DISABLE
            };

        let changes = [
            Self::kevent(
                fd,
                libc::EVFILT_READ,
                read_flags,
            ),

            Self::kevent(
                fd,
                libc::EVFILT_WRITE,
                write_flags,
            ),
        ];

        let result = unsafe {
            libc::kevent(
                self.kqueue_fd,
                changes.as_ptr(),
                changes.len() as i32,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };

        if result < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    pub fn modify(
        &mut self,
        fd: SocketId,
        interest: Interest,
    ) -> io::Result<()> {
        let read_flags =
            if interest.contains(Interest::READABLE) {
                libc::EV_ENABLE
            } else {
                libc::EV_DISABLE
            };

        let write_flags =
            if interest.contains(Interest::WRITABLE) {
                libc::EV_ENABLE
            } else {
                libc::EV_DISABLE
            };

        let changes = [
            Self::kevent(
                fd,
                libc::EVFILT_READ,
                read_flags,
            ),

            Self::kevent(
                fd,
                libc::EVFILT_WRITE,
                write_flags,
            ),
        ];

        let result = unsafe {
            libc::kevent(
                self.kqueue_fd,
                changes.as_ptr(),
                changes.len() as i32,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };

        if result < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    pub fn deregister(
        &mut self,
        fd: SocketId,
    ) -> io::Result<()> {
        let changes = [
            Self::kevent(
                fd,
                libc::EVFILT_READ,
                libc::EV_DELETE,
            ),

            Self::kevent(
                fd,
                libc::EVFILT_WRITE,
                libc::EV_DELETE,
            ),
        ];

        let result = unsafe {
            libc::kevent(
                self.kqueue_fd,
                changes.as_ptr(),
                changes.len() as i32,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };

        if result < 0 {
            let err = io::Error::last_os_error();

            if matches!(
                err.raw_os_error(),
                Some(libc::ENOENT) | Some(libc::EBADF)
            ) {
                return Ok(());
            }

            return Err(err);
        }

        Ok(())
    }

    pub fn wait(
        &mut self,
        events: &mut Vec<EpollEvent>,
        timeout_ms: i32,
    ) -> io::Result<usize> {
        use std::collections::HashMap;

        events.clear();

        let capacity = events.capacity().max(1);

        let mut raw =
            Vec::<libc::kevent>::with_capacity(capacity);

        let timeout;

        let timeout_ptr =
            if timeout_ms < 0 {
                std::ptr::null()
            } else {
                timeout = libc::timespec {
                    tv_sec:
                        (timeout_ms / 1000)
                            as libc::time_t,

                    tv_nsec:
                        ((timeout_ms % 1000) * 1_000_000)
                            as libc::c_long,
                };

                &timeout as *const libc::timespec
            };

        let count = unsafe {
            libc::kevent(
                self.kqueue_fd,
                std::ptr::null(),
                0,
                raw.as_mut_ptr(),
                capacity as i32,
                timeout_ptr,
            )
        };

        if count < 0 {
            let err = io::Error::last_os_error();

            if err.kind() == io::ErrorKind::Interrupted {
                return Ok(0);
            }

            return Err(err);
        }

        unsafe {
            raw.set_len(count as usize);
        }

        let mut merged:
            HashMap<SocketId, EpollEvent> =
            HashMap::new();

        for event in raw {
            let fd = event.ident as SocketId;

            let item =
                merged.entry(fd).or_insert(EpollEvent {
                    fd,
                    readable: false,
                    writable: false,
                    error: false,
                    hup: false,
                });

            if event.filter == libc::EVFILT_READ {
                item.readable = true;
            }

            if event.filter == libc::EVFILT_WRITE {
                item.writable = true;
            }

            if event.flags & libc::EV_ERROR != 0 {
                item.error = true;
            }

            if event.flags & libc::EV_EOF != 0 {
                item.hup = true;
            }
        }

        events.extend(merged.into_values());

        Ok(events.len())
    }
}

#[cfg(target_os = "macos")]
impl Drop for Epoll {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.kqueue_fd);
        }
    }
}

/* ============================================================
   WINDOWS
   ============================================================ */

#[cfg(windows)]
use std::collections::HashMap;

#[cfg(windows)]
#[repr(C)]
struct WsaPollFd {
    fd: SocketId,
    events: i16,
    revents: i16,
}

#[cfg(windows)]
#[link(name = "Ws2_32")]
extern "system" {
    fn WSAPoll(
        fd_array: *mut WsaPollFd,
        fds: u32,
        timeout: i32,
    ) -> i32;

    fn WSAGetLastError() -> i32;
}

#[cfg(windows)]
const POLLERR: i16 = 0x0001;

#[cfg(windows)]
const POLLHUP: i16 = 0x0002;

#[cfg(windows)]
const POLLNVAL: i16 = 0x0004;

#[cfg(windows)]
const POLLWRNORM: i16 = 0x0010;

#[cfg(windows)]
const POLLRDNORM: i16 = 0x0100;

#[cfg(windows)]
const POLLRDBAND: i16 = 0x0200;

#[cfg(windows)]
const POLLIN: i16 = POLLRDNORM | POLLRDBAND;

#[cfg(windows)]
pub struct Epoll {
    interests: HashMap<SocketId, Interest>,
}

#[cfg(windows)]
impl Epoll {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            interests: HashMap::new(),
        })
    }

    pub fn register(
        &mut self,
        fd: SocketId,
        interest: Interest,
    ) -> io::Result<()> {
        if self.interests.contains_key(&fd) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "socket already registered",
            ));
        }

        self.interests.insert(fd, interest);

        Ok(())
    }

    pub fn modify(
        &mut self,
        fd: SocketId,
        interest: Interest,
    ) -> io::Result<()> {
        match self.interests.get_mut(&fd) {
            Some(current) => {
                *current = interest;
                Ok(())
            }

            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "socket not registered",
            )),
        }
    }

    pub fn deregister(
        &mut self,
        fd: SocketId,
    ) -> io::Result<()> {
        self.interests.remove(&fd);

        Ok(())
    }

    pub fn wait(
        &mut self,
        events: &mut Vec<EpollEvent>,
        timeout_ms: i32,
    ) -> io::Result<usize> {
        events.clear();

        if self.interests.is_empty() {
            return Ok(0);
        }

        let mut poll_fds = Vec::with_capacity(
            self.interests.len(),
        );

        for (&fd, &interest) in &self.interests {
            let mut requested = 0i16;

            if interest.contains(Interest::READABLE) {
                requested |= POLLIN;
            }

            if interest.contains(Interest::WRITABLE) {
                requested |= POLLWRNORM;
            }

            poll_fds.push(WsaPollFd {
                fd,
                events: requested,
                revents: 0,
            });
        }

        let result = unsafe {
            WSAPoll(
                poll_fds.as_mut_ptr(),
                poll_fds.len() as u32,
                timeout_ms,
            )
        };

        if result < 0 {
            let code = unsafe { WSAGetLastError() };

            return Err(io::Error::from_raw_os_error(code));
        }

        if result == 0 {
            return Ok(0);
        }

        for poll in poll_fds {
            if poll.revents == 0 {
                continue;
            }

            events.push(EpollEvent {
                fd: poll.fd,

                readable:
                    poll.revents & POLLIN != 0,

                writable:
                    poll.revents & POLLWRNORM != 0,

                error:
                    poll.revents
                        & (POLLERR | POLLNVAL)
                        != 0,

                hup:
                    poll.revents & POLLHUP != 0,
            });
        }

        Ok(events.len())
    }
}

/* ============================================================
   COMMON TESTS
   ============================================================ */

#[cfg(test)]
mod tests {
    use super::*;

    use crate::net::socket::stream_id;

    use std::io::Write;
    use std::net::{TcpListener, TcpStream};

    fn pair() -> (TcpStream, TcpStream) {
        let listener =
            TcpListener::bind(("127.0.0.1", 0)).unwrap();

        let addr = listener.local_addr().unwrap();

        let client = TcpStream::connect(addr).unwrap();

        let (server, _) = listener.accept().unwrap();

        server.set_nonblocking(true).unwrap();

        (server, client)
    }

    #[test]
    fn readable_event_is_reported() {
        let (server, mut client) = pair();

        let fd = stream_id(&server);

        let mut poller = Epoll::new().unwrap();

        poller
            .register(fd, Interest::READABLE)
            .unwrap();

        client.write_all(b"x").unwrap();

        let mut events = Vec::with_capacity(16);

        let count =
            poller.wait(&mut events, 1000).unwrap();

        assert!(count >= 1);

        assert!(
            events
                .iter()
                .any(|event| {
                    event.fd == fd && event.readable
                })
        );
    }

    #[test]
    fn deregister_removes_socket() {
        let (server, mut client) = pair();

        let fd = stream_id(&server);

        let mut poller = Epoll::new().unwrap();

        poller
            .register(fd, Interest::READABLE)
            .unwrap();

        poller.deregister(fd).unwrap();

        client.write_all(b"x").unwrap();

        let mut events = Vec::with_capacity(16);

        let count =
            poller.wait(&mut events, 50).unwrap();

        assert_eq!(count, 0);
    }
}