use std::net::{TcpListener, TcpStream};

#[cfg(unix)]
pub type SocketId = std::os::fd::RawFd;

#[cfg(windows)]
pub type SocketId = std::os::windows::io::RawSocket;

#[cfg(unix)]
pub fn listener_id(listener: &TcpListener) -> SocketId {
    use std::os::fd::AsRawFd;
    listener.as_raw_fd()
}

#[cfg(windows)]
pub fn listener_id(listener: &TcpListener) -> SocketId {
    use std::os::windows::io::AsRawSocket;
    listener.as_raw_socket()
}

#[cfg(unix)]
pub fn stream_id(stream: &TcpStream) -> SocketId {
    use std::os::fd::AsRawFd;
    stream.as_raw_fd()
}

#[cfg(windows)]
pub fn stream_id(stream: &TcpStream) -> SocketId {
    use std::os::windows::io::AsRawSocket;
    stream.as_raw_socket()
}