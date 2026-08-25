use std::collections::HashMap;
use std::io;

use crate::config::{
    Config,
    ServerConfig,
};

use crate::http::{
    error_response,
    parse_request,
    HttpRequest,
    HttpResponse,
    ParseError,
    ParseResult,
    StatusCode,
};

use crate::net::epoll::{
    Epoll,
    EpollEvent,
    Interest,
};

use crate::net::io::{
    read_fd_once,
    read_once,
    write_fd_once,
    write_once,
    ReadOutcome,
    WriteOutcome,
};

use crate::net::listener::{
    accept_one,
    AcceptResult,
    Listener,
};

use crate::net::process::try_wait;

use crate::net::socket::SocketId;

use crate::server::cgi::{
    start_cgi,
    stop_cgi,
    CgiProcess,
};

use crate::server::connection::{
    ConnState,
    Connection,
};

use crate::server::http_handler::{
    bad_request_response,
    handle_request,
    payload_too_large_response,
    request_timeout_response,
    RequestOutcome,
};

use crate::server::session::SessionStore;

const READ_CHUNK: usize = 16 * 1024;

/// How often tick() sweeps expired sessions. Swept opportunistically
/// from the event loop rather than a background thread, per the
/// project's one-thread constraint.
const SESSION_SWEEP_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(60);

/// How long a client connection may go without any read or write
/// activity before it is closed. Covers every idle-connection case
/// the spec calls out: incomplete request headers, incomplete
/// bodies, slow uploads, slow response consumers, and idle
/// persistent connections. Matches CGI_TIMEOUT for consistency.
pub const CONNECTION_IDLE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(10);

/// How often tick() sweeps idle connections. A shorter interval than
/// CONNECTION_IDLE_TIMEOUT itself so an idle connection is closed
/// reasonably close to its actual deadline, not up to a full sweep
/// interval late.
const CONNECTION_SWEEP_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(2);

/// Default hard cap on simultaneous client connections, independent
/// of any one virtual server's configuration. Bounds memory and file
/// descriptor usage under a connection flood; once at the cap,
/// listener-readable events are ignored until a connection frees up.
/// EventLoop::new uses this; EventLoop::with_max_connections allows
/// a smaller cap, primarily so tests can exercise the cap without
/// opening a thousand-plus real sockets.
pub const DEFAULT_MAX_CONNECTIONS: usize = 1024;

pub struct EventLoop {
    epoll: Epoll,

    listeners:
        HashMap<SocketId, Listener>,

    connections:
        HashMap<SocketId, Connection>,

    events_buf:
        Vec<EpollEvent>,

    config: Config,

    /*
     * Used as a connection-level cap while parsing, before routing
     * selects a specific server whose
     * limit actually applies. See http::parse_request.
     */
    max_body_size: usize,

    max_connections: usize,

    /*
     * Owned by the single server thread; no synchronization needed.
     * Expiration is swept from tick(), never from a background
     * thread.
     */
    sessions: SessionStore,

    last_session_sweep: std::time::Instant,

    last_connection_sweep: std::time::Instant,

    /*
     * One entry per in-flight CGI process, keyed by its stdout fd
     * (the fd registered with epoll for reading the CGI's output).
     * The stdin fd, when still open, is tracked inside CgiMeta so it
     * can be looked up and deregistered/closed independently.
     */
    cgi_processes: HashMap<SocketId, CgiMeta>,

    /*
     * Maps a CGI process's stdin fd back to its stdout fd, so a
     * writable event on stdin can find the right CgiMeta without a
     * second top-level map keyed the same way as connections.
     */
    cgi_stdin_to_stdout: HashMap<SocketId, SocketId>,

    /*
     * PIDs of CGI children whose HTTP response has already been
     * sent but whose exit status has not yet been collected.
     * finish_cgi never blocks waiting for a child to become
     * reapable (a killed process can take a moment for the kernel
     * to finish tearing down, and busy-waiting for that inside
     * request handling would stall the whole event loop). Instead
     * every pending pid is retried with a single non-blocking
     * waitpid(WNOHANG) each tick until it succeeds, which is what
     * actually prevents zombies from accumulating without ever
     * blocking.
     */
    pending_reap: Vec<libc::pid_t>,
}

struct CgiMeta {
    process: CgiProcess,
    server: ServerConfig,
    set_cookie: Option<String>,
}

impl EventLoop {
    pub fn new(
        listeners: Vec<Listener>,
        config: Config,
    ) -> io::Result<Self> {
        Self::with_max_connections(
            listeners,
            config,
            DEFAULT_MAX_CONNECTIONS,
        )
    }

    /// Same as `new`, but with an explicit connection cap instead of
    /// DEFAULT_MAX_CONNECTIONS. Exists primarily so tests can
    /// exercise the connection-cap behavior without opening a
    /// thousand-plus real sockets.
    pub fn with_max_connections(
        listeners: Vec<Listener>,
        config: Config,
        max_connections: usize,
    ) -> io::Result<Self> {
        let mut epoll =
            Epoll::new()?;

        let mut listener_map =
            HashMap::new();

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

        let max_body_size =
            config
                .servers
                .iter()
                .map(|server| server.client_max_body_size)
                .max()
                .unwrap_or(0);

        Ok(Self {
            epoll,

            listeners:
                listener_map,

            connections:
                HashMap::new(),

            events_buf:
                Vec::with_capacity(1024),

            config,

            max_body_size,

            max_connections,

            sessions: SessionStore::new(),

            last_session_sweep: std::time::Instant::now(),

            last_connection_sweep: std::time::Instant::now(),

            cgi_processes: HashMap::new(),

            cgi_stdin_to_stdout: HashMap::new(),

            pending_reap: Vec::new(),
        })
    }

    pub fn listener_count(
        &self,
    ) -> usize {
        self.listeners.len()
    }

    pub fn connection_count(
        &self,
    ) -> usize {
        self.connections.len()
    }

    pub fn run(
        &mut self,
    ) -> io::Result<()> {
        loop {
            self.tick(-1)?;
        }
    }

    pub fn tick(
        &mut self,
        timeout_ms: i32,
    ) -> io::Result<usize> {
        if self.last_session_sweep.elapsed()
            >= SESSION_SWEEP_INTERVAL
        {
            self.sessions.sweep_expired();

            self.last_session_sweep =
                std::time::Instant::now();
        }

        self.sweep_cgi_timeouts();

        self.sweep_pending_reaps();

        if self.last_connection_sweep.elapsed()
            >= CONNECTION_SWEEP_INTERVAL
        {
            self.sweep_idle_connections();

            self.last_connection_sweep =
                std::time::Instant::now();
        }

        /*
         * epoll_wait only returns when a registered fd becomes
         * ready. A CGI child that is simply slow (e.g. stuck in a
         * sleep, not writing to its stdout pipe) never signals
         * anything, so without a bound here sweep_cgi_timeouts would
         * never get another chance to run and the timeout could be
         * exceeded indefinitely on an otherwise idle server. The
         * same applies to a killed-but-not-yet-reapable child and
         * sweep_pending_reaps. The idle-connection sweep has the
         * identical problem: a client that connects and sends
         * nothing generates no epoll event at all. Cap the wait
         * whenever any of this bookkeeping is outstanding; a
         * 1-second cap is frequent enough relative to both
         * CGI_TIMEOUT and CONNECTION_IDLE_TIMEOUT (10s each) without
         * turning this into a busy-wait on an idle server.
         */
        let cgi_work_pending =
            !self.cgi_processes.is_empty()
                || !self.pending_reap.is_empty();

        let connections_pending =
            !self.connections.is_empty();

        let effective_timeout_ms =
            if (cgi_work_pending || connections_pending)
                && (timeout_ms < 0 || timeout_ms > 1000)
            {
                1000
            } else {
                timeout_ms
            };

        let count =
            self.epoll.wait(
                &mut self.events_buf,
                effective_timeout_ms,
            )?;

        let events =
            self.events_buf.clone();

        for event in events {
            if self
                .listeners
                .contains_key(
                    &event.fd
                )
            {
                self.handle_listener_event(
                    event.fd
                );

                continue;
            }

            if self
                .cgi_processes
                .contains_key(
                    &event.fd
                )
            {
                if event.error || event.hup {
                    /*
                     * A HUP on the stdout pipe with no prior
                     * readable event still means "drain whatever is
                     * left, then finish" - handle_cgi_stdout_readable
                     * performs exactly one read and finalizes the
                     * process once the child has closed its end.
                     */
                    self.handle_cgi_stdout_readable(
                        event.fd
                    );

                    continue;
                }

                if event.readable {
                    self.handle_cgi_stdout_readable(
                        event.fd
                    );
                }

                continue;
            }

            if self
                .cgi_stdin_to_stdout
                .contains_key(
                    &event.fd
                )
            {
                if event.writable {
                    self.handle_cgi_stdin_writable(
                        event.fd
                    );
                }

                continue;
            }

            if !self
                .connections
                .contains_key(
                    &event.fd
                )
            {
                continue;
            }

            if event.error {
                self.remove_connection(
                    event.fd
                );

                continue;
            }

            /*
             * Perform at most one socket write for this event.
             */
            if event.writable {
                self.handle_client_writable(
                    event.fd
                );

                continue;
            }

            /*
             * Perform at most one socket read for this event.
             */
            if event.readable {
                self.handle_client_readable(
                    event.fd
                );

                continue;
            }

            if event.hup {
                self.remove_connection(
                    event.fd
                );
            }
        }

        Ok(count)
    }

    fn handle_listener_event(
        &mut self,
        listener_id: SocketId,
    ) {
        /*
         * At the connection cap: leave the pending client in the
         * OS's listen backlog rather than accepting it. epoll is
         * level-triggered here, so the listener will keep reporting
         * readable on subsequent ticks until either a connection
         * frees up (accepted then) or the peer gives up waiting -
         * this bounds memory/fd usage without needing to accept and
         * immediately close, which would still cost one fd
         * momentarily and one extra syscall pair per rejected
         * client.
         */
        if self.connections.len() >= self.max_connections {
            return;
        }

        let (result, local_addr, local_port) = {
            let listener =
                match self
                    .listeners
                    .get(&listener_id)
                {
                    Some(listener) => {
                        listener
                    }

                    None => {
                        return;
                    }
                };

            /*
             * Exactly one accept attempt.
             */
            (
                accept_one(listener),
                listener.addr,
                listener.port,
            )
        };

        match result {
            Ok(
                AcceptResult::Accepted {
                    stream,
                    peer,
                }
            ) => {
                let connection =
                    Connection::new(
                        stream,
                        peer.0,
                        peer.1,
                        local_addr,
                        local_port,
                    );

                let id =
                    connection.id;

                if self
                    .epoll
                    .register(
                        id,
                        Interest::READABLE,
                    )
                    .is_err()
                {
                    return;
                }

                self.connections.insert(
                    id,
                    connection,
                );
            }

            Ok(
                AcceptResult::WouldBlock
            ) => {}

            Ok(
                AcceptResult::Interrupted
            ) => {}

            Err(err) => {
                eprintln!(
                    "accept error: {}",
                    err
                );
            }
        }
    }

    fn handle_client_readable(
        &mut self,
        id: SocketId,
    ) {
        let mut chunk =
            [0u8; READ_CHUNK];

        let outcome = {
            let connection =
                match self
                    .connections
                    .get_mut(&id)
                {
                    Some(connection) => {
                        connection
                    }

                    None => return,
                };

            /*
             * ONE read only.
             */
            read_once(
                &mut connection.socket,
                &mut chunk,
            )
        };

        match outcome {
            Ok(
                ReadOutcome::Read(count)
            ) => {
                let mut parse_failed =
                    false;

                let mut body_too_large =
                    false;

                let max_body_size =
                    self.max_body_size;

                if let Some(connection) =
                    self
                        .connections
                        .get_mut(&id)
                {
                    connection
                        .read_buf
                        .extend_from_slice(
                            &chunk[..count],
                        );

                    connection.touch();

                    /*
                     * This does NOT perform another socket read.
                     */
                    loop {
                        match parse_request(
                            &connection.read_buf,
                            max_body_size,
                        ) {
                            Ok(
                                ParseResult::Complete {
                                    value,
                                    consumed,
                                }
                            ) => {
                                connection
                                    .requests
                                    .push_back(
                                        value
                                    );

                                connection
                                    .read_buf
                                    .drain(
                                        ..consumed
                                    );
                            }

                            Ok(
                                ParseResult::Incomplete
                            ) => {
                                break;
                            }

                            Err(err) => {
                                eprintln!(
                                    "bad request from {}:{}: {}",
                                    connection.peer_addr,
                                    connection.peer_port,
                                    err
                                );

                                body_too_large =
                                    err == ParseError::BodyTooLarge;

                                parse_failed =
                                    true;

                                connection
                                    .read_buf
                                    .clear();

                                break;
                            }
                        }
                    }
                }

                /*
                 * Remove parsed requests from the connection
                 * before handling them so we don't keep a mutable
                 * borrow of self.connections.
                 */
                let mut requests =
                    Vec::new();

                let mut local_addr =
                    std::net::Ipv4Addr::UNSPECIFIED;

                let mut local_port = 0;

                if let Some(connection) =
                    self
                        .connections
                        .get_mut(&id)
                {
                    local_addr =
                        connection.local_addr;

                    local_port =
                        connection.local_port;

                    /*
                     * A CGI process is already running for an
                     * earlier request on this connection; any newly
                     * parsed requests must wait in the queue until
                     * that response has been sent, to preserve
                     * HTTP/1.1 response ordering. process_queued_requests
                     * (called from finish_cgi once the CGI response
                     * has been sent) is what eventually drains them.
                     */
                    if !connection.awaiting_cgi {
                        while let Some(request) =
                            connection
                                .requests
                                .pop_front()
                        {
                            requests.push(
                                request
                            );
                        }
                    }
                }

                for request in requests.into_iter().rev() {
                    if let Some(connection) =
                        self.connections.get_mut(&id)
                    {
                        connection.requests.push_front(request);
                    }
                }

                self.process_queued_requests(
                    id,
                    local_addr,
                    local_port,
                );

                /*
                 * Malformed HTTP gets a real 400/413 response now
                 * instead of silently dropping the connection.
                 */
                if parse_failed {
                    let response =
                        if body_too_large {
                            payload_too_large_response(
                                &self.config
                            )
                        } else {
                            bad_request_response(
                                &self.config
                            )
                        };

                    let bytes =
                        response.to_bytes();

                    if let Some(connection) =
                        self
                            .connections
                            .get_mut(&id)
                    {
                        connection
                            .queue_write_and_close(
                                bytes
                            );
                    }

                    let _ = self.epoll.modify(
                        id,
                        Interest::WRITABLE,
                    );
                }

            }

            Ok(
                ReadOutcome::Closed
            ) => {
                self.remove_connection(
                    id
                );
            }

            Ok(
                ReadOutcome::WouldBlock
            ) => {}

            Ok(
                ReadOutcome::Interrupted
            ) => {}

            Err(err) => {
                eprintln!(
                    "read error: {}",
                    err
                );

                self.remove_connection(
                    id
                );
            }
        }
    }

    fn handle_client_writable(
        &mut self,
        id: SocketId,
    ) {
        let outcome = {
            let connection =
                match self
                    .connections
                    .get_mut(&id)
                {
                    Some(connection) => {
                        connection
                    }

                    None => return,
                };

            if connection
                .write_complete()
            {
                return;
            }

            let offset =
                connection.write_offset;

            let socket =
                &mut connection.socket;

            let data =
                &connection
                    .write_buf[offset..];

            /*
             * ONE write only.
             */
            write_once(
                socket,
                data,
            )
        };

        match outcome {
            Ok(
                WriteOutcome::Written(count)
            ) => {
                let mut remove =
                    false;

                let mut switch_to_read =
                    false;

                if let Some(connection) =
                    self
                        .connections
                        .get_mut(&id)
                {
                    connection
                        .write_offset
                        += count;

                    connection.touch();

                    if connection
                        .write_complete()
                    {
                        if connection
                            .close_after_write
                        {
                            remove = true;
                        } else {
                            connection.state =
                                ConnState::Reading;

                            connection
                                .write_buf
                                .clear();

                            connection
                                .write_offset =
                                0;

                            switch_to_read =
                                true;
                        }
                    }
                }

                if remove {
                    self.remove_connection(
                        id
                    );

                    return;
                }

                if switch_to_read {
                    if self
                        .epoll
                        .modify(
                            id,
                            Interest::READABLE,
                        )
                        .is_err()
                    {
                        self.remove_connection(
                            id
                        );
                    }
                }
            }

            Ok(
                WriteOutcome::WouldBlock
            ) => {}

            Ok(
                WriteOutcome::Interrupted
            ) => {}

            Err(err) => {
                eprintln!(
                    "write error: {}",
                    err
                );

                self.remove_connection(
                    id
                );
            }
        }
    }

    pub fn queue_response(
        &mut self,
        id: SocketId,
        data: Vec<u8>,
    ) -> io::Result<()> {
        if let Some(connection) =
            self
                .connections
                .get_mut(&id)
        {
            connection
                .queue_write(
                    data
                );

            self.epoll.modify(
                id,
                Interest::WRITABLE,
            )?;
        }

        Ok(())
    }

    /// Forks the CGI process, registers its stdin/stdout pipes with
    /// epoll, and marks the originating connection as waiting.
    ///
    /// If spawning fails for any reason (missing executable, fork
    /// failure, and so on), a 500 response is queued immediately on
    /// the connection instead - the caller does not need to handle
    /// that failure itself.
    fn spawn_cgi_for_connection(
        &mut self,
        client_id: SocketId,
        request: &HttpRequest,
        executable: &str,
        script_path: &std::path::Path,
        route_path: &str,
        server: ServerConfig,
        set_cookie: Option<String>,
    ) {
        let peer_addr =
            match self.connections.get(&client_id) {
                Some(connection) => connection.peer_addr,

                None => return,
            };

        let local_port =
            self.connections
                .get(&client_id)
                .map(|connection| connection.local_port)
                .unwrap_or(0);

        let server_name = server
            .server_name
            .first()
            .cloned()
            .unwrap_or_else(|| server.server_address.clone());

        let spawn_result = start_cgi(
            request,
            executable,
            script_path,
            route_path,
            client_id,
            &server_name,
            local_port,
            peer_addr,
        );

        let process =
            match spawn_result {
                Ok(process) => process,

                Err(err) => {
                    eprintln!(
                        "CGI spawn failed for {}: {}",
                        script_path.display(),
                        err
                    );

                    let response = with_optional_cookie(
                        error_response(
                            &server,
                            StatusCode::InternalServerError,
                        ),
                        set_cookie,
                    );

                    self.queue_response_now(
                        client_id,
                        response.to_bytes(),
                    );

                    return;
                }
            };

        let stdout_fd = process.stdout_fd;

        if let Some(connection) =
            self.connections.get_mut(&client_id)
        {
            connection.awaiting_cgi = true;
        }

        let stdin_registered =
            if let Some(stdin_fd) = process.stdin_fd {
                if process.stdin_complete() {
                    /*
                     * Empty body (e.g. a GET-driven CGI): nothing to
                     * write, close stdin immediately so the child
                     * sees EOF right away instead of waiting for a
                     * writable event that will never carry data.
                     */
                    unsafe {
                        libc::close(stdin_fd);
                    }

                    None
                } else {
                    match self.epoll.register(
                        stdin_fd,
                        Interest::WRITABLE,
                    ) {
                        Ok(()) => {
                            self.cgi_stdin_to_stdout
                                .insert(stdin_fd, stdout_fd);

                            Some(stdin_fd)
                        }

                        Err(_) => {
                            unsafe {
                                libc::close(stdin_fd);
                            }

                            None
                        }
                    }
                }
            } else {
                None
            };

        let mut process = process;

        if stdin_registered.is_none() {
            process.stdin_fd = None;
        }

        if self
            .epoll
            .register(stdout_fd, Interest::READABLE)
            .is_err()
        {
            stop_cgi(&process);

            if let Some(fd) = process.stdin_fd {
                let _ = self.epoll.deregister(fd);
                self.cgi_stdin_to_stdout.remove(&fd);
                unsafe {
                    libc::close(fd);
                }
            }

            let response = with_optional_cookie(
                error_response(
                    &server,
                    StatusCode::InternalServerError,
                ),
                set_cookie,
            );

            if let Some(connection) =
                self.connections.get_mut(&client_id)
            {
                connection.awaiting_cgi = false;
            }

            self.queue_response_now(
                client_id,
                response.to_bytes(),
            );

            return;
        }

        self.cgi_processes.insert(
            stdout_fd,
            CgiMeta {
                process,
                server,
                set_cookie,
            },
        );
    }

    /// Performs at most one read from a CGI process's stdout pipe.
    /// Finalizes the process (builds the HTTP response, delivers it
    /// to the client connection, cleans up fds/epoll registrations)
    /// once the child closes its end.
    fn handle_cgi_stdout_readable(
        &mut self,
        stdout_fd: SocketId,
    ) {
        let mut buf = [0u8; READ_CHUNK];

        let outcome = read_fd_once(stdout_fd, &mut buf);

        match outcome {
            Ok(ReadOutcome::Read(count)) => {
                if let Some(meta) =
                    self.cgi_processes.get_mut(&stdout_fd)
                {
                    meta.process
                        .output
                        .extend_from_slice(&buf[..count]);
                }
            }

            Ok(ReadOutcome::Closed) => {
                self.finish_cgi(stdout_fd, None);
            }

            Ok(ReadOutcome::WouldBlock) => {}

            Ok(ReadOutcome::Interrupted) => {}

            Err(err) => {
                self.finish_cgi(
                    stdout_fd,
                    Some(err.to_string()),
                );
            }
        }
    }

    /// Performs at most one write to a CGI process's stdin pipe.
    fn handle_cgi_stdin_writable(
        &mut self,
        stdin_fd: SocketId,
    ) {
        let stdout_fd =
            match self.cgi_stdin_to_stdout.get(&stdin_fd) {
                Some(fd) => *fd,

                None => return,
            };

        let outcome = {
            let meta =
                match self.cgi_processes.get(&stdout_fd) {
                    Some(meta) => meta,

                    None => return,
                };

            write_fd_once(
                stdin_fd,
                meta.process.stdin_remaining(),
            )
        };

        match outcome {
            Ok(WriteOutcome::Written(count)) => {
                if let Some(meta) =
                    self.cgi_processes.get_mut(&stdout_fd)
                {
                    meta.process.stdin_written += count;

                    if meta.process.stdin_complete() {
                        self.close_cgi_stdin(stdin_fd);
                    }
                }
            }

            Ok(WriteOutcome::WouldBlock) => {}

            Ok(WriteOutcome::Interrupted) => {}

            Err(_) => {
                self.close_cgi_stdin(stdin_fd);
            }
        }
    }

    fn close_cgi_stdin(
        &mut self,
        stdin_fd: SocketId,
    ) {
        let _ = self.epoll.deregister(stdin_fd);

        let stdout_fd =
            self.cgi_stdin_to_stdout.remove(&stdin_fd);

        unsafe {
            libc::close(stdin_fd);
        }

        if let Some(stdout_fd) = stdout_fd {
            if let Some(meta) =
                self.cgi_processes.get_mut(&stdout_fd)
            {
                meta.process.stdin_fd = None;
            }
        }
    }

    /// Reaps every CGI process whose runtime has exceeded
    /// CGI_TIMEOUT, killing it and delivering a 500 to its client.
    /// Called once per tick(); never blocks.
    fn sweep_cgi_timeouts(&mut self) {
        let timed_out: Vec<SocketId> = self
            .cgi_processes
            .iter()
            .filter(|(_, meta)| meta.process.timed_out())
            .map(|(fd, _)| *fd)
            .collect();

        for stdout_fd in timed_out {
            self.finish_cgi(
                stdout_fd,
                Some("CGI process timed out".to_string()),
            );
        }
    }

    /// Closes every connection that has gone CONNECTION_IDLE_TIMEOUT
    /// without any read or write activity. Covers incomplete request
    /// headers, incomplete bodies, slow uploads, slow response
    /// consumers, and idle persistent connections - the same
    /// last_activity timestamp is touched by every read and write,
    /// so one check covers all of these cases uniformly. Called
    /// periodically from tick(), never blocks.
    ///
    /// A connection currently waiting on a CGI process is left
    /// alone here even if idle past the timeout - that case is
    /// already governed by CGI_TIMEOUT and sweep_cgi_timeouts, which
    /// owns closing it out (via finish_cgi) once the CGI itself
    /// times out, avoiding a race between two sweeps closing the
    /// same connection two different ways.
    fn sweep_idle_connections(&mut self) {
        let timed_out: Vec<SocketId> = self
            .connections
            .iter()
            .filter(|(_, connection)| {
                !connection.awaiting_cgi
                    && connection.last_activity.elapsed()
                        >= CONNECTION_IDLE_TIMEOUT
            })
            .map(|(id, _)| *id)
            .collect();

        for id in timed_out {
            let has_incomplete_request = self
                .connections
                .get(&id)
                .map(|connection| {
                    !connection.read_buf.is_empty()
                })
                .unwrap_or(false);

            if has_incomplete_request {
                let response =
                    request_timeout_response(&self.config);

                if let Some(connection) =
                    self.connections.get_mut(&id)
                {
                    connection.queue_write_and_close(
                        response.to_bytes(),
                    );
                }

                let _ = self
                    .epoll
                    .modify(id, Interest::WRITABLE);
            } else {
                self.remove_connection(id);
            }
        }
    }

    /// Retries a single non-blocking waitpid(WNOHANG) for every CGI
    /// child whose response was already sent but that hadn't yet
    /// exited at the time. Called once per tick(); never blocks -
    /// a pid that still isn't reapable simply stays queued for the
    /// next tick.
    fn sweep_pending_reaps(&mut self) {
        self.pending_reap.retain(|&pid| {
            !matches!(try_wait(pid), Ok(Some(_)) | Err(_))
        });
    }

    /// Finalizes a CGI process: reaps it (non-blocking; if it hasn't
    /// exited yet, it is killed first so waitpid succeeds), builds
    /// either a real response from its stdout output or a 500 if
    /// `failure_reason` is set or the process exited abnormally, and
    /// delivers that response to the originating client connection.
    fn finish_cgi(
        &mut self,
        stdout_fd: SocketId,
        failure_reason: Option<String>,
    ) {
        let meta =
            match self.cgi_processes.remove(&stdout_fd) {
                Some(meta) => meta,

                None => return,
            };

        let _ = self.epoll.deregister(stdout_fd);

        unsafe {
            libc::close(stdout_fd);
        }

        if let Some(stdin_fd) = meta.process.stdin_fd {
            let _ = self.epoll.deregister(stdin_fd);

            self.cgi_stdin_to_stdout.remove(&stdin_fd);

            unsafe {
                libc::close(stdin_fd);
            }
        }

        /*
         * On the timeout/failure path the process is still running
         * and must be killed before it can ever be reaped. On the
         * normal-completion path (stdout closed on its own) the
         * child is usually already exiting or exited, so killing it
         * is harmless - SIGKILL on an already-dead pid is a no-op
         * once it's been reaped, and this function never reaps
         * synchronously, so there's no race to worry about here.
         */
        if failure_reason.is_some() {
            stop_cgi(&meta.process);
        }

        /*
         * One non-blocking check only - never loop or sleep waiting
         * for the child to become reapable. If it hasn't exited yet,
         * its pid is queued for the periodic sweep in tick() to keep
         * retrying with WNOHANG until it succeeds; this function
         * must return promptly regardless.
         */
        let exit_ok = match try_wait(meta.process.pid) {
            Ok(Some(code)) => {
                failure_reason.is_none() && code == 0
            }

            Ok(None) => {
                self.pending_reap.push(meta.process.pid);

                failure_reason.is_none()
            }

            Err(_) => false,
        };

        /*
         * A non-zero exit code alone does not invalidate output the
         * script already wrote: CGI/1.1 has no notion of "exit code
         * as status," only the response bytes themselves (a script
         * signals a non-2xx outcome via a Status: header, not via
         * its exit code). Only treat this as a hard failure when
         * there is no output to fall back on - an empty stdout from
         * a nonzero-exit script has nothing valid to build a
         * response from either way.
         */
        let response =
            if let Some(reason) = &failure_reason {
                eprintln!(
                    "CGI process {} failed: {}",
                    meta.process.pid,
                    reason
                );

                error_response(
                    &meta.server,
                    StatusCode::InternalServerError,
                )
            } else if !exit_ok && meta.process.output.is_empty() {
                error_response(
                    &meta.server,
                    StatusCode::InternalServerError,
                )
            } else {
                build_cgi_response(&meta.server, &meta.process.output)
            };

        let response =
            with_optional_cookie(response, meta.set_cookie);

        let client_id = meta.process.client_id;

        let (local_addr, local_port) =
            match self.connections.get_mut(&client_id) {
                Some(connection) => {
                    connection.awaiting_cgi = false;

                    (connection.local_addr, connection.local_port)
                }

                None => {
                    return;
                }
            };

        self.queue_response_now(client_id, response.to_bytes());

        self.process_queued_requests(
            client_id,
            local_addr,
            local_port,
        );
    }

    /// Queues a response for immediate delivery to `client_id` and
    /// switches epoll interest to writable, exactly like
    /// queue_response but without returning a Result - used from
    /// contexts (CGI completion) that already log their own errors.
    fn queue_response_now(
        &mut self,
        client_id: SocketId,
        bytes: Vec<u8>,
    ) {
        if let Some(connection) =
            self.connections.get_mut(&client_id)
        {
            connection.queue_write(bytes);
        }

        let _ = self
            .epoll
            .modify(client_id, Interest::WRITABLE);
    }

    /// Dispatches every request currently queued on connection `id`,
    /// in order, stopping early (and leaving the rest queued) if one
    /// resolves to CGI. Arms writable interest afterward if any
    /// response was queued.
    ///
    /// This is the single place responsible for draining
    /// Connection::requests. It is called both from
    /// handle_client_readable (after a read parses new requests) and
    /// from finish_cgi (once a CGI response has been sent, so any
    /// requests that piled up behind it - pushed back onto the front
    /// of the queue when the CGI request was first dispatched - get
    /// processed immediately instead of waiting for a socket event
    /// that may never come, since HTTP/1.1 pipelining allows a
    /// client to wait for prior responses before sending more data).
    fn process_queued_requests(
        &mut self,
        id: SocketId,
        local_addr: std::net::Ipv4Addr,
        local_port: u16,
    ) {
        loop {
            if self
                .connections
                .get(&id)
                .map(|connection| connection.awaiting_cgi)
                .unwrap_or(true)
            {
                break;
            }

            let request =
                match self
                    .connections
                    .get_mut(&id)
                    .and_then(|connection| {
                        connection.requests.pop_front()
                    }) {
                    Some(request) => request,

                    None => break,
                };

            let outcome = handle_request(
                &self.config,
                &request,
                local_addr,
                local_port,
                &mut self.sessions,
            );

            match outcome {
                RequestOutcome::Response(response) => {
                    let bytes = response.to_bytes();

                    if let Some(connection) =
                        self.connections.get_mut(&id)
                    {
                        connection.queue_write(bytes);
                    }
                }

                RequestOutcome::StartCgi {
                    executable,
                    script_path,
                    route_path,
                    set_cookie,
                    server,
                } => {
                    self.spawn_cgi_for_connection(
                        id,
                        &request,
                        &executable,
                        &script_path,
                        &route_path,
                        server,
                        set_cookie,
                    );

                    break;
                }
            }
        }

        let should_write = self
            .connections
            .get(&id)
            .map(|connection| !connection.write_complete())
            .unwrap_or(false);

        if should_write
            && self
                .epoll
                .modify(id, Interest::WRITABLE)
                .is_err()
        {
            self.remove_connection(id);
        }
    }

    fn remove_connection(
        &mut self,
        id: SocketId,
    ) {
        if self
            .connections
            .remove(&id)
            .is_some()
        {
            let _ =
                self
                    .epoll
                    .deregister(id);
        }

        let orphaned_cgi: Vec<SocketId> = self
            .cgi_processes
            .iter()
            .filter(|(_, meta)| meta.process.client_id == id)
            .map(|(fd, _)| *fd)
            .collect();

        for stdout_fd in orphaned_cgi {
            if let Some(meta) =
                self.cgi_processes.remove(&stdout_fd)
            {
                let _ = self.epoll.deregister(stdout_fd);

                unsafe {
                    libc::close(stdout_fd);
                }

                if let Some(stdin_fd) = meta.process.stdin_fd {
                    let _ = self.epoll.deregister(stdin_fd);

                    self.cgi_stdin_to_stdout.remove(&stdin_fd);

                    unsafe {
                        libc::close(stdin_fd);
                    }
                }

                stop_cgi(&meta.process);

                /*
                 * The client is gone, so there is no response left
                 * to deliver, but the process still must be reaped
                 * or it becomes a permanent zombie. Same one-shot,
                 * non-blocking check as finish_cgi: if it isn't
                 * reapable yet, defer to the periodic sweep instead
                 * of looping here.
                 */
                if matches!(
                    try_wait(meta.process.pid),
                    Ok(None)
                ) {
                    self.pending_reap.push(meta.process.pid);
                }
            }
        }
    }
}

/// Builds the final HTTP response from a CGI script's raw output.
///
/// CGI output is a set of headers (at minimum Content-Type),
/// CRLF-or-LF terminated, followed by a blank line, followed by the
/// body - the same shape as an HTTP message but without a status
/// line. A malformed CGI response (no header/body separator) is
/// treated as 500, per the spec's explicit failure case.
fn build_cgi_response(
    server: &ServerConfig,
    output: &[u8],
) -> HttpResponse {
    let separator = output
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|pos| (pos, 2))
        .or_else(|| {
            output
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|pos| (pos, 4))
        });

    let (header_end, separator_len) =
        match separator {
            Some(value) => value,

            None => {
                return error_response(
                    server,
                    StatusCode::InternalServerError,
                );
            }
        };

    let header_bytes = &output[..header_end];
    let body = &output[header_end + separator_len..];

    let header_text =
        match std::str::from_utf8(header_bytes) {
            Ok(text) => text,

            Err(_) => {
                return error_response(
                    server,
                    StatusCode::InternalServerError,
                );
            }
        };

    let mut response =
        HttpResponse::new(StatusCode::Ok, body.to_vec());

    let mut has_content_type = false;

    for line in header_text.split(['\n', '\r']) {
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim();
            let value = value.trim();

            if name.eq_ignore_ascii_case("content-type") {
                has_content_type = true;
            }

            response = response.with_header(name, value);
        }
    }

    if !has_content_type {
        return error_response(
            server,
            StatusCode::InternalServerError,
        );
    }

    response
}

fn with_optional_cookie(
    response: HttpResponse,
    set_cookie: Option<String>,
) -> HttpResponse {
    match set_cookie {
        Some(cookie) => response.with_header("Set-Cookie", cookie),

        None => response,
    }
}

impl Drop for EventLoop {
    fn drop(&mut self) {
        let connections:
            Vec<SocketId> =
            self
                .connections
                .keys()
                .copied()
                .collect();

        for id in connections {
            self.remove_connection(
                id
            );
        }

        let listeners:
            Vec<SocketId> =
            self
                .listeners
                .keys()
                .copied()
                .collect();

        for id in listeners {
            let _ =
                self
                    .epoll
                    .deregister(id);
        }

        self.listeners.clear();
    }
}