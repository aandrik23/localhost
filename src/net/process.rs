//! Non-blocking CGI child process creation.
//!
//! Spawns a CGI executable as a child process connected to the
//! server via two pipes (stdin, stdout), both set non-blocking on
//! the parent's end so the event loop can drive them exactly like
//! any other file descriptor - one read or one write per readiness
//! event, never a blocking wait.
//!
//! This module is the only place in the codebase that calls fork(2)
//! and execve(2). Unix-only: CGI, like epoll, is not implemented for
//! non-Unix targets.

use std::collections::HashMap;
use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;
use std::path::Path;

pub struct CgiChild {
    pub pid: libc::pid_t,

    /// Parent's write end of the child's stdin. Write the request
    /// body here, then close it (drop this) once fully written so
    /// the child sees EOF.
    pub stdin_fd: RawFd,

    /// Parent's read end of the child's stdout.
    pub stdout_fd: RawFd,
}

/// Spawns `executable` with `script_path` as its first argument,
/// running with `working_dir` as its current directory and `env` as
/// its complete environment.
///
/// Safety / correctness notes:
///
/// - Only async-signal-safe operations occur between fork() and
///   execve() in the child, per POSIX: dup2, close, chdir, and
///   execve itself. No Rust allocation happens in that window (all
///   CStrings are built in the parent, before forking).
/// - If execve fails, the child calls `_exit` (not the normal Rust
///   panic/exit path) to avoid running any parent-side destructors
///   or buffered-output flushing twice across the fork.
/// - On any setup failure in the parent, already-opened fds are
///   closed before returning the error, so no descriptor leaks past
///   a failed spawn.
pub fn spawn_cgi(
    executable: &str,
    script_path: &Path,
    working_dir: &Path,
    env: &HashMap<String, String>,
) -> io::Result<CgiChild> {
    let mut stdin_pipe = [0 as RawFd; 2];
    let mut stdout_pipe = [0 as RawFd; 2];

    if unsafe { libc::pipe(stdin_pipe.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }

    if unsafe { libc::pipe(stdout_pipe.as_mut_ptr()) } != 0 {
        close_fd(stdin_pipe[0]);
        close_fd(stdin_pipe[1]);

        return Err(io::Error::last_os_error());
    }

    let executable_c =
        CString::new(executable)
            .map_err(invalid_argument)?;

    let script_path_c =
        CString::new(
            script_path.to_string_lossy().into_owned(),
        )
        .map_err(invalid_argument)?;

    let working_dir_c =
        CString::new(
            working_dir.to_string_lossy().into_owned(),
        )
        .map_err(invalid_argument)?;

    let argv: Vec<CString> = vec![
        executable_c.clone(),
        script_path_c,
    ];

    let mut argv_ptrs: Vec<*const libc::c_char> =
        argv.iter().map(|arg| arg.as_ptr()).collect();

    argv_ptrs.push(std::ptr::null());

    let env_c: Vec<CString> = env
        .iter()
        .filter_map(|(key, value)| {
            CString::new(format!("{}={}", key, value)).ok()
        })
        .collect();

    let mut envp_ptrs: Vec<*const libc::c_char> =
        env_c.iter().map(|entry| entry.as_ptr()).collect();

    envp_ptrs.push(std::ptr::null());

    let pid = unsafe { libc::fork() };

    if pid < 0 {
        let err = io::Error::last_os_error();

        close_fd(stdin_pipe[0]);
        close_fd(stdin_pipe[1]);
        close_fd(stdout_pipe[0]);
        close_fd(stdout_pipe[1]);

        return Err(err);
    }

    if pid == 0 {
        // Child process.
        unsafe {
            libc::close(stdin_pipe[1]);
            libc::close(stdout_pipe[0]);

            libc::dup2(stdin_pipe[0], libc::STDIN_FILENO);
            libc::dup2(stdout_pipe[1], libc::STDOUT_FILENO);

            libc::close(stdin_pipe[0]);
            libc::close(stdout_pipe[1]);

            if libc::chdir(working_dir_c.as_ptr()) != 0 {
                libc::_exit(127);
            }

            /*
             * execvpe (glibc extension) searches PATH like execvp,
             * but also lets us set the exact environment instead of
             * inheriting the parent's - CGI scripts must see only
             * the CGI environment (PATH_INFO, REQUEST_METHOD, etc.),
             * not whatever the server process happened to be
             * started with.
             */
            libc::execvpe(
                executable_c.as_ptr(),
                argv_ptrs.as_ptr(),
                envp_ptrs.as_ptr(),
            );

            // execvpe only returns on failure.
            libc::_exit(127);
        }
    }

    // Parent process.
    unsafe {
        libc::close(stdin_pipe[0]);
        libc::close(stdout_pipe[1]);
    }

    if let Err(err) = set_nonblocking(stdin_pipe[1]) {
        close_fd(stdin_pipe[1]);
        close_fd(stdout_pipe[0]);

        return Err(err);
    }

    if let Err(err) = set_nonblocking(stdout_pipe[0]) {
        close_fd(stdin_pipe[1]);
        close_fd(stdout_pipe[0]);

        return Err(err);
    }

    Ok(CgiChild {
        pid,
        stdin_fd: stdin_pipe[1],
        stdout_fd: stdout_pipe[0],
    })
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };

    if flags < 0 {
        return Err(io::Error::last_os_error());
    }

    let result = unsafe {
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK)
    };

    if result < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

fn close_fd(fd: RawFd) {
    unsafe {
        libc::close(fd);
    }
}

fn invalid_argument(
    _err: std::ffi::NulError,
) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "CGI argument contained a NUL byte",
    )
}

/// Non-blocking check for whether `pid` has exited.
///
/// Returns Ok(None) if the process is still running, Ok(Some(exit_code))
/// once it has exited normally, or an exit code of -1 if it was
/// killed by a signal. Never blocks: uses WNOHANG.
pub fn try_wait(
    pid: libc::pid_t,
) -> io::Result<Option<i32>> {
    let mut status: libc::c_int = 0;

    let result = unsafe {
        libc::waitpid(pid, &mut status, libc::WNOHANG)
    };

    if result == 0 {
        return Ok(None);
    }

    if result < 0 {
        return Err(io::Error::last_os_error());
    }

    if libc::WIFEXITED(status) {
        return Ok(Some(libc::WEXITSTATUS(status)));
    }

    Ok(Some(-1))
}

/// Sends SIGKILL to `pid`. Used to enforce the CGI timeout.
pub fn kill_process(pid: libc::pid_t) {
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
}
