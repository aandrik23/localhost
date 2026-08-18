use std::collections::HashMap;
use std::io;

use crate::net::epoll::{
    Epoll,
    EpollEvent,
    Interest,
};

use crate::net::io::{
    read_once,
    write_once,
    ReadOutcome,
    WriteOutcome,
};

use crate::net::listener::{
    accept_one,
    AcceptResult,
    Listener,
};

use crate::net::socket::SocketId;

use crate::server::connection::{
    ConnState,
    Connection,
};

const READ_CHUNK: usize = 16 * 1024;

pub struct EventLoop {
    epoll: Epoll,

    listeners: HashMap<SocketId, Listener>,

    connections: HashMap<SocketId, Connection>,

    events_buf: Vec<EpollEvent>,
}

impl EventLoop {
    pub fn new(
        listeners: Vec<Listener>,
    ) -> io::Result<Self> {
        let mut epoll = Epoll::new()?;

        let mut listener_map = HashMap::new();

        for listener in listeners {
            epoll.register(
                listener.id,
                Interest::READABLE,
            )?;

            listener_map.insert(
                listener.id,
                listener,
            );
        }

        Ok(Self {
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

    pub fn run(&mut self) -> io::Result<()> {
        loop {
            self.tick(-1)?;
        }
    }

    pub fn tick(
        &mut self,
        timeout_ms: i32,
    ) -> io::Result<usize> {
        let count =
            self.epoll.wait(
                &mut self.events_buf,
                timeout_ms,
            )?;

        let events = self.events_buf.clone();

        for event in events {
            if self.listeners.contains_key(&event.fd) {
                self.handle_listener_event(event.fd);
                continue;
            }

            if !self.connections.contains_key(&event.fd) {
                continue;
            }

            if event.error {
                self.remove_connection(event.fd);
                continue;
            }

            if event.writable {
                self.handle_client_writable(event.fd);
                continue;
            }

            if event.readable {
                self.handle_client_readable(event.fd);
                continue;
            }

            if event.hup {
                self.remove_connection(event.fd);
            }
        }

        Ok(count)
    }

    fn handle_listener_event(
        &mut self,
        listener_id: SocketId,
    ) {
        let result = {
            let listener =
                match self.listeners.get(&listener_id) {
                    Some(listener) => listener,
                    None => return,
                };

            accept_one(listener)
        };

        match result {
            Ok(AcceptResult::Accepted {
                stream,
                peer,
            }) => {
                let connection =
                    Connection::new(
                        stream,
                        peer.0,
                        peer.1,
                    );

                let id = connection.id;

                if self
                    .epoll
                    .register(id, Interest::READABLE)
                    .is_err()
                {
                    return;
                }

                self.connections.insert(
                    id,
                    connection,
                );
            }

            Ok(AcceptResult::WouldBlock) => {}

            Ok(AcceptResult::Interrupted) => {}

            Err(_) => {}
        }
    }

    fn handle_client_readable(
        &mut self,
        id: SocketId,
    ) {
        let mut chunk = [0u8; READ_CHUNK];

        let outcome = {
            let connection =
                match self.connections.get_mut(&id) {
                    Some(connection) => connection,
                    None => return,
                };

            read_once(
                &mut connection.socket,
                &mut chunk,
            )
        };

        match outcome {
            Ok(ReadOutcome::Read(count)) => {
                if let Some(connection) =
                    self.connections.get_mut(&id)
                {
                    connection
                        .read_buf
                        .extend_from_slice(
                            &chunk[..count],
                        );

                    connection.touch();
                }
            }

            Ok(ReadOutcome::Closed) => {
                self.remove_connection(id);
            }

            Ok(ReadOutcome::WouldBlock) => {}

            Ok(ReadOutcome::Interrupted) => {}

            Err(_) => {
                self.remove_connection(id);
            }
        }
    }

    fn handle_client_writable(
        &mut self,
        id: SocketId,
    ) {
        let outcome = {
            let connection =
                match self.connections.get_mut(&id) {
                    Some(connection) => connection,
                    None => return,
                };

            if connection.write_offset
                >= connection.write_buf.len()
            {
                return;
            }

            let offset = connection.write_offset;

            let socket = &mut connection.socket;

            let data =
                &connection.write_buf[offset..];

            write_once(socket, data)
        };

        match outcome {
            Ok(WriteOutcome::Written(count)) => {
                let mut should_remove = false;

                if let Some(connection) =
                    self.connections.get_mut(&id)
                {
                    connection.write_offset += count;

                    connection.touch();

                    if connection.write_complete() {
                        connection.state =
                            ConnState::Reading;

                        connection.write_buf.clear();

                        connection.write_offset = 0;

                        if self
                            .epoll
                            .modify(
                                id,
                                Interest::READABLE,
                            )
                            .is_err()
                        {
                            should_remove = true;
                        }
                    }
                }

                if should_remove {
                    self.remove_connection(id);
                }
            }

            Ok(WriteOutcome::WouldBlock) => {}

            Ok(WriteOutcome::Interrupted) => {}

            Err(_) => {
                self.remove_connection(id);
            }
        }
    }

    pub fn queue_response(
        &mut self,
        id: SocketId,
        data: Vec<u8>,
    ) -> io::Result<()> {
        if let Some(connection) =
            self.connections.get_mut(&id)
        {
            connection.queue_write(data);

            self.epoll.modify(
                id,
                Interest::WRITABLE,
            )?;
        }

        Ok(())
    }

    fn remove_connection(
        &mut self,
        id: SocketId,
    ) {
        if self.connections.remove(&id).is_some() {
            let _ = self.epoll.deregister(id);

            // TcpStream is automatically closed here when
            // Connection is dropped.
        }
    }
}

impl Drop for EventLoop {
    fn drop(&mut self) {
        let connections:
            Vec<SocketId> =
            self.connections
                .keys()
                .copied()
                .collect();

        for id in connections {
            self.remove_connection(id);
        }

        let listeners:
            Vec<SocketId> =
            self.listeners
                .keys()
                .copied()
                .collect();

        for id in listeners {
            let _ = self.epoll.deregister(id);
        }

        self.listeners.clear();
    }
}