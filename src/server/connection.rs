use std::collections::VecDeque;
use std::net::{
    Ipv4Addr,
    TcpStream,
};
use std::time::Instant;

use crate::http::HttpRequest;

use crate::net::socket::{
    stream_id,
    SocketId,
};

#[derive(
    Debug,
    PartialEq,
    Eq,
    Clone,
    Copy,
)]
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

    pub local_addr: Ipv4Addr,
    pub local_port: u16,

    pub state: ConnState,

    pub read_buf: Vec<u8>,

    pub requests: VecDeque<HttpRequest>,

    pub write_buf: Vec<u8>,

    pub write_offset: usize,

    pub close_after_write: bool,

    pub last_activity: Instant,
}

impl Connection {
    pub fn new(
        socket: TcpStream,
        peer_addr: Ipv4Addr,
        peer_port: u16,
        local_addr: Ipv4Addr,
        local_port: u16,
    ) -> Self {
        let id =
            stream_id(&socket);

        Self {
            id,

            socket,

            peer_addr,
            peer_port,

            local_addr,
            local_port,

            state:
                ConnState::Reading,

            read_buf:
                Vec::new(),

            requests:
                VecDeque::new(),

            write_buf:
                Vec::new(),

            write_offset:
                0,

            close_after_write:
                false,

            last_activity:
                Instant::now(),
        }
    }

    pub fn pending_write(
        &self,
    ) -> &[u8] {
        &self.write_buf[
            self.write_offset..
        ]
    }

    pub fn write_complete(
        &self,
    ) -> bool {
        self.write_offset
            >= self.write_buf.len()
    }

    /*
     * Adds bytes to the outgoing buffer.
     *
     * If a response is already waiting, append the next
     * response instead of replacing it.
     *
     * This allows basic HTTP/1.1 pipelining.
     */
    pub fn queue_write(
        &mut self,
        data: Vec<u8>,
    ) {
        if self.write_complete() {
            self.write_buf = data;

            self.write_offset = 0;
        } else {
            self.write_buf
                .extend_from_slice(&data);
        }

        self.state =
            ConnState::Writing;
    }

    pub fn queue_write_and_close(
        &mut self,
        data: Vec<u8>,
    ) {
        self.queue_write(data);

        self.close_after_write = true;

        self.state =
            ConnState::Closing;
    }

    pub fn touch(
        &mut self,
    ) {
        self.last_activity =
            Instant::now();
    }
}