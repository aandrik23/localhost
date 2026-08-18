//! Cross-platform TCP listener creation and single accept handling.
//!
//! Listening and client sockets are always non-blocking.
//! Exactly one accept attempt is performed for each listener event.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};

use crate::net::socket::{listener_id, SocketId};

pub struct Listener {
    pub id: SocketId,
    pub addr: Ipv4Addr,
    pub port: u16,
    socket: TcpListener,
}

impl Listener {
    pub fn socket(&self) -> &TcpListener {
        &self.socket
    }
}

/// Creates one non-blocking TCP listening socket.
pub fn bind_listener(addr: Ipv4Addr, port: u16) -> io::Result<Listener> {
    let requested = SocketAddrV4::new(addr, port);

    let socket = TcpListener::bind(requested)?;

    socket.set_nonblocking(true)?;

    let local_addr = socket.local_addr()?;

    let actual_port = local_addr.port();
    let id = listener_id(&socket);

    Ok(Listener {
        id,
        addr,
        port: actual_port,
        socket,
    })
}

pub enum AcceptResult {
    Accepted {
        stream: TcpStream,
        peer: (Ipv4Addr, u16),
    },

    WouldBlock,

    Interrupted,
}

/// Performs exactly one accept attempt.
pub fn accept_one(listener: &Listener) -> io::Result<AcceptResult> {
    match listener.socket.accept() {
        Ok((stream, peer_addr)) => {
            stream.set_nonblocking(true)?;

            let peer = match peer_addr {
                SocketAddr::V4(addr) => (*addr.ip(), addr.port()),

                SocketAddr::V6(addr) => {
                    let ip = addr
                        .ip()
                        .to_ipv4()
                        .unwrap_or(Ipv4Addr::UNSPECIFIED);

                    (ip, addr.port())
                }
            };

            Ok(AcceptResult::Accepted {
                stream,
                peer,
            })
        }

        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
            Ok(AcceptResult::WouldBlock)
        }

        Err(err) if err.kind() == io::ErrorKind::Interrupted => {
            Ok(AcceptResult::Interrupted)
        }

        Err(err) => Err(err),
    }
}