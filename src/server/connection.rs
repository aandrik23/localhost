use std::net::{Ipv4Addr, TcpStream};
use std::time::Instant;

use crate::net::socket::{stream_id, SocketId};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ConnState {
    Reading,
    Writing,
    Closing,
}

pub struct Connection {
    pub id: SocketId,

    pub socket: TcpStream,

    pub peer_addr: Ipv4Addr,
    pub peer_port: u16,

    pub state: ConnState,

    pub read_buf: Vec<u8>,

    pub write_buf: Vec<u8>,
    pub write_offset: usize,

    pub last_activity: Instant,
}

impl Connection {
    pub fn new(
        socket: TcpStream,
        peer_addr: Ipv4Addr,
        peer_port: u16,
    ) -> Self {
        let id = stream_id(&socket);

        Self {
            id,
            socket,

            peer_addr,
            peer_port,

            state: ConnState::Reading,

            read_buf: Vec::new(),

            write_buf: Vec::new(),
            write_offset: 0,

            last_activity: Instant::now(),
        }
    }

    pub fn pending_write(&self) -> &[u8] {
        &self.write_buf[self.write_offset..]
    }

    pub fn write_complete(&self) -> bool {
        self.write_offset >= self.write_buf.len()
    }

    pub fn queue_write(&mut self, data: Vec<u8>) {
        self.write_buf = data;
        self.write_offset = 0;
        self.state = ConnState::Writing;
    }

    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }
}