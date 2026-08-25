//! Non-blocking CGI process lifecycle.
//!
//! A CgiProcess is driven exactly like a client Connection: the event
//! loop performs at most one read or one write per readiness event on
//! its stdin/stdout pipe fds, never a blocking wait() or an internal
//! read loop. See server::event_loop for how these fds are
//! registered with epoll alongside listeners and client sockets.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::{
    Path,
    PathBuf,
};
use std::time::Instant;

use crate::http::{
    HttpRequest,
    Method,
};

use crate::net::process::{
    kill_process,
    spawn_cgi,
    CgiChild,
};

use crate::net::socket::SocketId;

/// How long a CGI process may run before it is killed and the
/// client is sent a 500. Kept short and fixed rather than
/// configurable, since the spec only requires that a timeout exists.
pub const CGI_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgiState {
    /// Writing the request body to the child's stdin.
    WritingStdin,

    /// Reading the CGI's output from the child's stdout.
    ReadingStdout,
}

pub struct CgiProcess {
    pub pid: libc::pid_t,

    /// The client connection this CGI process's output must be
    /// delivered back to once it completes.
    pub client_id: SocketId,

    pub stdin_fd: Option<std::os::fd::RawFd>,
    pub stdout_fd: std::os::fd::RawFd,

    pub state: CgiState,

    pub request_body: Vec<u8>,
    pub stdin_written: usize,

    pub output: Vec<u8>,

    pub started_at: Instant,
}

impl CgiProcess {
    pub fn stdin_remaining(&self) -> &[u8] {
        &self.request_body[self.stdin_written..]
    }

    pub fn stdin_complete(&self) -> bool {
        self.stdin_written >= self.request_body.len()
    }

    pub fn timed_out(&self) -> bool {
        self.started_at.elapsed() > CGI_TIMEOUT
    }
}

/// Spawns a CGI process for `request`, matched to `route` via
/// `extension`/`executable`.
///
/// `script_path` is the resolved, already-traversal-checked path to
/// the script file on disk (the caller is responsible for that
/// safety check, the same way static file serving already resolves
/// and checks its own paths before this function is called).
pub fn start_cgi(
    request: &HttpRequest,
    executable: &str,
    script_path: &Path,
    route_path: &str,
    client_id: SocketId,
    server_name: &str,
    local_port: u16,
    peer_addr: Ipv4Addr,
) -> std::io::Result<CgiProcess> {
    let working_dir: PathBuf =
        script_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

    let env = build_cgi_env(
        request,
        script_path,
        route_path,
        server_name,
        local_port,
        peer_addr,
    );

    let CgiChild {
        pid,
        stdin_fd,
        stdout_fd,
    } = spawn_cgi(
        executable,
        script_path,
        &working_dir,
        &env,
    )?;

    Ok(CgiProcess {
        pid,
        client_id,
        stdin_fd: Some(stdin_fd),
        stdout_fd,
        state: CgiState::WritingStdin,
        request_body: request.body.clone(),
        stdin_written: 0,
        output: Vec::new(),
        started_at: Instant::now(),
    })
}

pub fn stop_cgi(process: &CgiProcess) {
    kill_process(process.pid);
}

fn build_cgi_env(
    request: &HttpRequest,
    script_path: &Path,
    route_path: &str,
    server_name: &str,
    local_port: u16,
    peer_addr: Ipv4Addr,
) -> HashMap<String, String> {
    let mut env = HashMap::new();

    env.insert(
        "GATEWAY_INTERFACE".to_string(),
        "CGI/1.1".to_string(),
    );

    env.insert(
        "SERVER_PROTOCOL".to_string(),
        "HTTP/1.1".to_string(),
    );

    env.insert(
        "SERVER_SOFTWARE".to_string(),
        "localhost-rust".to_string(),
    );

    env.insert("SERVER_NAME".to_string(), server_name.to_string());

    env.insert(
        "SERVER_PORT".to_string(),
        local_port.to_string(),
    );

    env.insert(
        "REQUEST_METHOD".to_string(),
        method_str(&request.method).to_string(),
    );

    env.insert(
        "SCRIPT_NAME".to_string(),
        script_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );

    /*
     * PATH_INFO: the portion of the request path after the route
     * and script name, per the spec ("CGI will check PATH_INFO
     * environment variable to define the full path"). If the
     * request path is exactly the script, PATH_INFO is empty.
     */
    let path_info = request
        .path
        .strip_prefix(route_path)
        .unwrap_or(&request.path);

    env.insert(
        "PATH_INFO".to_string(),
        path_info.to_string(),
    );

    env.insert(
        "QUERY_STRING".to_string(),
        request.query.clone().unwrap_or_default(),
    );

    env.insert(
        "CONTENT_LENGTH".to_string(),
        request.body.len().to_string(),
    );

    if let Some(content_type) = request.header("Content-Type") {
        env.insert(
            "CONTENT_TYPE".to_string(),
            content_type.to_string(),
        );
    }

    env.insert(
        "REMOTE_ADDR".to_string(),
        peer_addr.to_string(),
    );

    for header in &request.headers {
        let env_name = format!(
            "HTTP_{}",
            header
                .name
                .to_ascii_uppercase()
                .replace('-', "_")
        );

        env.insert(env_name, header.value.clone());
    }

    /*
     * PATH is required for execvpe's own PATH search to find the
     * interpreter (e.g. "python3"); without it the exec itself
     * would fail regardless of what the CGI script needs.
     */
    if let Ok(path) = std::env::var("PATH") {
        env.insert("PATH".to_string(), path);
    }

    env
}

fn method_str(method: &Method) -> &str {
    method.as_str()
}
