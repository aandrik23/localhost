//! The single central event loop.
//!
//! `EventLoop::run` contains the only `epoll_wait` call site used for
//! serving traffic (the wrapper in `net::epoll::Epoll::wait` is the actual
//! syscall; this is where it's invoked from, in a loop, forever). Every
//! iteration dispatches ready fds to exactly one accept, one read, or one
//! write -- never more than one I/O operation per fd per iteration.

use std::collections::HashMap;
use std::io;
use std::os::unix::io::RawFd;

use crate::net::epoll::{decode_events, Epoll, Interest};
use crate::net::io::{read_once, write_once, ReadOutcome, WriteOutcome};
use crate::net::listener::{accept_one, close_fd, AcceptResult, Listener};
use crate::server::connection::{ConnState, Connection};

/// Size of the per-read chunk requested from the client. A single
/// `read_once` call never asks for more than this many bytes at a time.
const READ_CHUNK: usize = 16 * 1024;

/// Owns the epoll instance, the listening sockets, and the table of live
/// client connections. This is the one and only event loop in the process.
pub struct EventLoop {
    epoll: Epoll,
    listeners: HashMap<RawFd, Listener>,
    connections: HashMap<RawFd, Connection>,
    events_buf: Vec<libc::epoll_event>,
}

impl EventLoop {
    pub fn new(listeners: Vec<Listener>) -> io::Result<EventLoop> {
        let epoll = Epoll::new()?;
        let mut listener_map = HashMap::new();

        for listener in listeners {
            epoll.register(listener.fd, Interest::READABLE)?;
            listener_map.insert(listener.fd, listener);
        }

        Ok(EventLoop {
            epoll,
            listeners: listener_map,
            connections: HashMap::new(),
            events_buf: Vec::with_capacity(1024),
        })
    }

    pub fn listener_count(&self) -> usize {
        self.listeners.len()
    }

    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Runs the event loop forever (or until `epoll_wait` returns a fatal
    /// error). This is the main event loop referenced throughout the
    /// design docs -- there is exactly one call to this method, from
    /// `main.rs`, on the single server thread.
    pub fn run(&mut self) -> io::Result<()> {
        loop {
            self.tick(-1)?;
        }
    }

    /// Runs exactly one iteration of the event loop: one `epoll_wait` call
    /// followed by dispatch of whatever fds were reported ready. Exposed
    /// separately from `run` so tests can drive the loop deterministically
    /// instead of blocking forever.
    pub fn tick(&mut self, timeout_ms: i32) -> io::Result<usize> {
        let n = self.epoll.wait(&mut self.events_buf, timeout_ms)?;

        // Decode first, into an owned Vec, so we don't hold a borrow of
        // `self.events_buf` while mutably dispatching into `self`.
        let events: Vec<_> = decode_events(&self.events_buf).collect();

        for ev in events {
            if self.listeners.contains_key(&ev.fd) {
                self.handle_listener_event(ev.fd);
                continue;
            }

            if !self.connections.contains_key(&ev.fd) {
                // Fd no longer tracked (e.g. removed earlier in this same
                // batch of events); nothing to do.
                continue;
            }

            if ev.error || ev.hup {
                // Fatal/half-closed socket condition reported directly by
                // epoll: remove the client without attempting further I/O
                // on it.
                self.remove_connection(ev.fd);
                continue;
            }

            if ev.writable {
                self.handle_client_writable(ev.fd);
                // A single dispatch performs a read OR a write, not both;
                // skip a same-iteration read even if the event also
                // reported readable.
                continue;
            }

            if ev.readable {
                self.handle_client_readable(ev.fd);
            }
        }

        Ok(n)
    }

    /// Accepts at most one client per listener-readable event, mirroring
    /// the same "one syscall per event" discipline applied to client
    /// sockets.
    fn handle_listener_event(&mut self, listener_fd: RawFd) {
        match accept_one(listener_fd) {
            Ok(AcceptResult::Accepted { fd, peer }) => {
                let conn = Connection::new(fd, peer.0, peer.1);
                if let Err(_err) = self.epoll.register(fd, Interest::READABLE) {
                    // Could not register the new client with epoll; close it
                    // immediately rather than leaving an unmultiplexed fd.
                    close_fd(fd);
                    return;
                }
                self.connections.insert(fd, conn);
            }
            Ok(AcceptResult::WouldBlock) => {
                // No pending connection; nothing to do until the next
                // readable event on the listener.
            }
            Ok(AcceptResult::Interrupted) => {
                // EINTR: benign, retry naturally on the next epoll event.
            }
            Err(_err) => {
                // A fatal accept() error on the listener itself (e.g.
                // ENFILE/EMFILE). The listener socket stays registered and
                // usable; we simply drop this attempt rather than crashing
                // the server or the other listeners.
            }
        }
    }

    /// Performs exactly one `read()` for `fd` and updates its connection
    /// state accordingly. Never loops until EAGAIN.
    fn handle_client_readable(&mut self, fd: RawFd) {
        let mut chunk = [0u8; READ_CHUNK];

        let outcome = {
            let conn = match self.connections.get_mut(&fd) {
                Some(c) => c,
                None => return,
            };
            read_once(conn.fd, &mut chunk)
        };

        match outcome {
            Ok(ReadOutcome::Read(n)) => {
                let conn = self.connections.get_mut(&fd).unwrap();
                conn.read_buf.extend_from_slice(&chunk[..n]);
                conn.touch();
                // Phase 2 has no HTTP parser yet; the bytes simply
                // accumulate in `read_buf`, demonstrating that partial
                // reads are represented in connection state rather than
                // consumed by a read-until-EAGAIN loop.
            }
            Ok(ReadOutcome::Closed) => {
                self.remove_connection(fd);
            }
            Ok(ReadOutcome::WouldBlock) => {
                // No data available right now; return to epoll_wait.
            }
            Ok(ReadOutcome::Interrupted) => {
                // EINTR on this read: benign, wait for the next readable
                // event rather than retrying inline.
            }
            Err(_err) => {
                // Fatal socket error (e.g. ECONNRESET surfaced as a hard
                // error rather than via EPOLLHUP): remove the client. One
                // broken client must not affect any other connection.
                self.remove_connection(fd);
            }
        }
    }

    /// Performs exactly one `write()` for `fd` and updates its connection
    /// state accordingly. Never loops until the buffer is fully drained.
    fn handle_client_writable(&mut self, fd: RawFd) {
        let outcome = {
            let conn = match self.connections.get_mut(&fd) {
                Some(c) => c,
                None => return,
            };
            if conn.pending_write().is_empty() {
                // Nothing queued to write; this can happen if we were
                // registered for EPOLLOUT but the buffer was already fully
                // drained by a previous event. Nothing to do.
                return;
            }
            write_once(conn.fd, conn.pending_write())
        };

        match outcome {
            Ok(WriteOutcome::Written(n)) => {
                let conn = self.connections.get_mut(&fd).unwrap();
                conn.write_offset += n;
                conn.touch();
                if conn.write_complete() {
                    // Response fully drained (in later phases). For Phase
                    // 2's purposes, go back to waiting for more input.
                    conn.state = ConnState::Reading;
                    conn.write_buf.clear();
                    conn.write_offset = 0;
                    if let Err(_err) = self.epoll.modify(conn.fd, Interest::READABLE) {
                        self.remove_connection(fd);
                    }
                }
            }
            Ok(WriteOutcome::WouldBlock) => {
                // Socket buffer full; wait for the next writable event.
            }
            Ok(WriteOutcome::Interrupted) => {
                // EINTR: benign, retry on the next writable event.
            }
            Err(_err) => {
                self.remove_connection(fd);
            }
        }
    }

    /// Switches a connection to `Writing` and registers writable interest.
    /// Exposed for tests / future phases that need to queue a response.
    pub fn queue_response(&mut self, fd: RawFd, data: Vec<u8>) -> io::Result<()> {
        if let Some(conn) = self.connections.get_mut(&fd) {
            conn.queue_write(data);
            self.epoll.modify(fd, Interest::WRITABLE)?;
        }
        Ok(())
    }

    /// Removes a client from epoll, closes its socket, and drops its
    /// connection state. This is the single cleanup path used for every
    /// fatal-error / disconnect / eviction scenario so that no cleanup
    /// branch can forget a step.
    fn remove_connection(&mut self, fd: RawFd) {
        if let Some(conn) = self.connections.remove(&fd) {
            let _ = self.epoll.deregister(conn.fd);
            close_fd(conn.fd);
            // `conn` (and its read/write buffers) is dropped here,
            // releasing all associated memory.
        }
    }
}

impl Drop for EventLoop {
    fn drop(&mut self) {
        // Best-effort cleanup of every remaining client and listener if the
        // event loop itself is torn down (e.g. in tests).
        let fds: Vec<RawFd> = self.connections.keys().copied().collect();
        for fd in fds {
            self.remove_connection(fd);
        }
        for (fd, _listener) in self.listeners.drain() {
            let _ = self.epoll.deregister(fd);
            close_fd(fd);
        }
    }
}
