//! Per-client connection state.
//!
//! Phase 2 scope: no HTTP parsing lives here yet. This struct exists to
//! prove that partial reads/writes are represented as connection state
//! rather than handled with blocking loops. Later phases will attach an
//! HTTP parser/response state to this struct without changing the
//! networking discipline established here.

use std::net::Ipv4Addr;
use std::os::unix::io::RawFd;
use std::time::Instant;

/// Coarse connection state for Phase 2. `Reading` and `Writing` are where
/// partial I/O accumulates; `Closing` marks a connection queued for
/// cleanup.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ConnState {
    /// Waiting for / accumulating bytes from the client.
    Reading,
    /// Waiting to drain `write_buf` back to the client.
    Writing,
    /// Connection is being torn down; no further I/O will be attempted.
    Closing,
}

/// All state associated with one accepted client socket.
pub struct Connection {
    pub fd: RawFd,
    pub peer_addr: Ipv4Addr,
    pub peer_port: u16,
    pub state: ConnState,

    /// Bytes read from the client but not yet consumed by (future) request
    /// parsing. A single `read_once` call appends whatever it returns here;
    /// nothing is discarded between events.
    pub read_buf: Vec<u8>,

    /// Bytes queued to be written back to the client, plus how much of it
    /// has already been sent. `write_once` sends from `write_offset..` and
    /// advances `write_offset` on partial writes -- this is the "partial
    /// write represented in connection state" requirement.
    pub write_buf: Vec<u8>,
    pub write_offset: usize,

    pub last_activity: Instant,
}

impl Connection {
    pub fn new(fd: RawFd, peer_addr: Ipv4Addr, peer_port: u16) -> Connection {
        Connection {
            fd,
            peer_addr,
            peer_port,
            state: ConnState::Reading,
            read_buf: Vec::new(),
            write_buf: Vec::new(),
            write_offset: 0,
            last_activity: Instant::now(),
        }
    }

    /// Bytes of the write buffer not yet sent.
    pub fn pending_write(&self) -> &[u8] {
        &self.write_buf[self.write_offset..]
    }

    /// True once every byte of `write_buf` has been written.
    pub fn write_complete(&self) -> bool {
        self.write_offset >= self.write_buf.len()
    }

    /// Queues bytes for writing and switches state to `Writing`. Resets the
    /// write cursor since this is a fresh buffer.
    pub fn queue_write(&mut self, data: Vec<u8>) {
        self.write_buf = data;
        self.write_offset = 0;
        self.state = ConnState::Writing;
    }

    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }
}
