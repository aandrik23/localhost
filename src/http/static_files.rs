use std::fs;
use std::io;
use std::path::{
    Component,
    Path,
    PathBuf,
};

use crate::config::{
    RouteConfig,
    ServerConfig,
};

use crate::http::{
    HttpRequest,
    HttpResponse,
    Method,
    StatusCode,
};

/// What a route resolves to once its method/body-limit/redirect
/// guards have passed.
pub enum RouteOutcome {
    /// A complete, immediately-sendable HTTP response - the normal
    /// case for GET/POST/DELETE against static files.
    Response(HttpResponse),

    /// The request matched a CGI extension mapping and passed every
    /// filesystem-safety check a static request would; `script_path`
    /// is the canonicalized, root-contained path to the script.
    /// Execution itself is the caller's responsibility (see
    /// server::cgi and server::event_loop) because it involves
    /// forking a process and registering pipe fds with the event
    /// loop, neither of which this module has access to.
    Cgi {
        executable: String,
        script_path: PathBuf,
    },
}

/// Resolves a request against an already-selected route: enforces
/// allowed methods, the server's body-size limit, and redirects,
/// then dispatches to either CGI or static file handling.
///
/// Route selection (which route matches `request.path`) and virtual
/// server selection happen before this function is called; see
/// `server::routing`.
pub fn resolve_route(
    request: &HttpRequest,
    server: &ServerConfig,
    route: &RouteConfig,
) -> RouteOutcome {
    if !method_allowed(route, &request.method) {
        return RouteOutcome::Response(
            error_response(
                server,
                StatusCode::MethodNotAllowed,
            )
            .with_header(
                "Allow",
                allow_header_value(route),
            ),
        );
    }

    /*
     * The connection-level cap in http::parse_request only protects
     * against unbounded memory use while parsing; it uses the
     * largest client_max_body_size across every configured server.
     * Now that routing has selected the specific server, enforce
     * its own (possibly smaller) limit.
     */
    if request.body.len() > server.client_max_body_size {
        return RouteOutcome::Response(error_response(
            server,
            StatusCode::PayloadTooLarge,
        ));
    }

    if let Some(location) = &route.redirect {
        let status = StatusCode::from_redirect_status(
            route.redirect_status.unwrap_or(302),
        );

        return RouteOutcome::Response(
            HttpResponse::new(
                status,
                Vec::new(),
            )
            .with_header(
                "Location",
                location.clone(),
            ),
        );
    }

    let root =
        match &route.root {
            Some(root) => PathBuf::from(root),

            None => {
                return RouteOutcome::Response(error_response(
                    server,
                    StatusCode::InternalServerError,
                ));
            }
        };

    let relative_path =
        request
            .path
            .strip_prefix(&route.path)
            .unwrap_or(&request.path)
            .trim_start_matches('/');

    if !is_safe_relative_path(relative_path) {
        return RouteOutcome::Response(error_response(
            server,
            StatusCode::Forbidden,
        ));
    }

    if let Some((executable, script_relative_path)) =
        cgi_executable_for(route, relative_path)
    {
        return resolve_cgi_script(
            server,
            &root,
            script_relative_path,
            executable,
        );
    }

    RouteOutcome::Response(handle_static_request(
        request,
        server,
        route,
        &root,
        relative_path,
    ))
}

/// Finds a CGI mapping within `relative_path`, scanning from the
/// first segment. This handles PATH_INFO-style requests like
/// "/script.py/extra/segments", where only "script.py" (not the
/// trailing segments) is the actual file to execute - checking only
/// the final path segment's extension would miss this, since
/// Path::extension() looks at the last component only.
///
/// Returns the matched executable and the relative path up to and
/// including the script itself (excluding any trailing PATH_INFO
/// segments); the caller computes PATH_INFO itself from the full
/// request path, so only the script's own location is needed here.
fn cgi_executable_for<'a, 'b>(
    route: &'a RouteConfig,
    relative_path: &'b str,
) -> Option<(&'a str, &'b str)> {
    let mut consumed_len = 0;

    for segment in relative_path.split('/') {
        consumed_len += segment.len();

        if let Some(extension) =
            Path::new(segment)
                .extension()
                .and_then(|ext| ext.to_str())
        {
            if let Some(executable) =
                route.cgi.get(&format!(".{}", extension))
            {
                return Some((
                    executable.as_str(),
                    &relative_path[..consumed_len],
                ));
            }
        }

        // Account for the '/' separator consumed by split, except
        // after the last segment.
        consumed_len += 1;
    }

    None
}

/// Resolves and safety-checks the CGI script path, reusing the exact
/// same canonicalize-then-contain check GET/POST/DELETE already use,
/// so a CGI request can no more escape the route root than a static
/// one can.
fn resolve_cgi_script(
    server: &ServerConfig,
    root: &Path,
    relative_path: &str,
    executable: &str,
) -> RouteOutcome {
    let requested_path = root.join(relative_path);

    let canonical_root =
        match fs::canonicalize(root) {
            Ok(path) => path,

            Err(_) => {
                return RouteOutcome::Response(error_response(
                    server,
                    StatusCode::InternalServerError,
                ));
            }
        };

    let canonical_script =
        match fs::canonicalize(&requested_path) {
            Ok(path) => path,

            Err(err) => {
                let status = match err.kind() {
                    io::ErrorKind::NotFound => {
                        StatusCode::NotFound
                    }

                    io::ErrorKind::PermissionDenied => {
                        StatusCode::Forbidden
                    }

                    _ => StatusCode::InternalServerError,
                };

                return RouteOutcome::Response(error_response(
                    server, status,
                ));
            }
        };

    if !canonical_script.starts_with(&canonical_root) {
        return RouteOutcome::Response(error_response(
            server,
            StatusCode::Forbidden,
        ));
    }

    if !canonical_script.is_file() {
        return RouteOutcome::Response(error_response(
            server,
            StatusCode::NotFound,
        ));
    }

    RouteOutcome::Cgi {
        executable: executable.to_string(),
        script_path: canonical_script,
    }
}

/// Handles GET/POST/DELETE against the filesystem for a route that
/// did not resolve to CGI. `root` and `relative_path` are passed in
/// already computed by resolve_route so the CGI-vs-static branch
/// point stays in one place.
fn handle_static_request(
    request: &HttpRequest,
    server: &ServerConfig,
    route: &RouteConfig,
    root: &Path,
    relative_path: &str,
) -> HttpResponse {
    if request.method == Method::Post {
        return handle_upload(
            request,
            server,
            root,
            relative_path,
        );
    }

    if request.method == Method::Delete {
        return handle_delete(
            server,
            root,
            relative_path,
        );
    }

    let mut requested_path =
        root.join(relative_path);

    /*
     * If:
     *
     * GET /
     *
     * points to a directory, use the configured index file.
     *
     * If there is no index file, fall back to a generated
     * directory listing when the route allows it.
     */
    let mut list_directory = false;

    if requested_path.is_dir() {
        match &route.index {
            Some(index) => {
                let index_path =
                    requested_path.join(index);

                if index_path.is_file() {
                    requested_path = index_path;
                } else if route.directory_listing {
                    list_directory = true;
                } else {
                    return error_response(
                        server,
                        StatusCode::Forbidden,
                    );
                }
            }

            None => {
                if route.directory_listing {
                    list_directory = true;
                } else {
                    return error_response(
                        server,
                        StatusCode::Forbidden,
                    );
                }
            }
        }
    }

    let canonical_root =
        match fs::canonicalize(root) {
            Ok(path) => path,

            Err(_) => {
                return error_response(
                    server,
                    StatusCode::InternalServerError,
                );
            }
        };

    let canonical_file =
        match fs::canonicalize(
            &requested_path
        ) {
            Ok(path) => path,

            Err(err) => {
                return match err.kind() {
                    io::ErrorKind::NotFound => {
                        error_response(
                            server,
                            StatusCode::NotFound,
                        )
                    }

                    io::ErrorKind::PermissionDenied => {
                        error_response(
                            server,
                            StatusCode::Forbidden,
                        )
                    }

                    _ => {
                        error_response(
                            server,
                            StatusCode::InternalServerError,
                        )
                    }
                };
            }
        };

    /*
     * Important path traversal protection.
     *
     * Even if somebody requests:
     *
     * /../../secret.txt
     *
     * the final canonical path may never escape the
     * configured web root.
     */
    if !canonical_file.starts_with(
        &canonical_root
    ) {
        return error_response(
            server,
            StatusCode::Forbidden,
        );
    }

    if list_directory {
        return match render_directory_listing(
            &canonical_file,
            &request.path,
        ) {
            Ok(response) => response,

            Err(_) => error_response(
                server,
                StatusCode::InternalServerError,
            ),
        };
    }

    if !canonical_file.is_file() {
        return error_response(
            server,
            StatusCode::NotFound,
        );
    }

    let body =
        match fs::read(&canonical_file) {
            Ok(data) => data,

            Err(err) => {
                return match err.kind() {
                    io::ErrorKind::NotFound => {
                        error_response(
                            server,
                            StatusCode::NotFound,
                        )
                    }

                    io::ErrorKind::PermissionDenied => {
                        error_response(
                            server,
                            StatusCode::Forbidden,
                        )
                    }

                    _ => {
                        error_response(
                            server,
                            StatusCode::InternalServerError,
                        )
                    }
                };
            }
        };

    HttpResponse::new(
        StatusCode::Ok,
        body,
    )
    .with_header(
        "Content-Type",
        content_type_for_path(
            &canonical_file
        ),
    )
}

/// Handles POST as a file upload: the request body is written to the
/// filesystem at `root` joined with `relative_path`, mirroring how
/// GET resolves the same path for static serving.
///
/// Returns 201 Created with a Location header on success. The parent
/// directory must already exist inside the route's root; this
/// function does not create directories on the client's behalf.
fn handle_upload(
    request: &HttpRequest,
    server: &ServerConfig,
    root: &Path,
    relative_path: &str,
) -> HttpResponse {
    if relative_path.is_empty()
        || relative_path.ends_with('/')
    {
        return error_response(
            server,
            StatusCode::Forbidden,
        );
    }

    let target_path =
        root.join(relative_path);

    let parent =
        match target_path.parent() {
            Some(parent) => parent,

            None => {
                return error_response(
                    server,
                    StatusCode::Forbidden,
                );
            }
        };

    let canonical_root =
        match fs::canonicalize(root) {
            Ok(path) => path,

            Err(_) => {
                return error_response(
                    server,
                    StatusCode::InternalServerError,
                );
            }
        };

    let canonical_parent =
        match fs::canonicalize(parent) {
            Ok(path) => path,

            Err(err) => {
                return match err.kind() {
                    io::ErrorKind::NotFound => {
                        error_response(
                            server,
                            StatusCode::NotFound,
                        )
                    }

                    io::ErrorKind::PermissionDenied => {
                        error_response(
                            server,
                            StatusCode::Forbidden,
                        )
                    }

                    _ => {
                        error_response(
                            server,
                            StatusCode::InternalServerError,
                        )
                    }
                };
            }
        };

    /*
     * Same containment rule as GET: the resolved parent directory
     * must never escape the configured route root, even through
     * symlinks.
     */
    if !canonical_parent.starts_with(&canonical_root) {
        return error_response(
            server,
            StatusCode::Forbidden,
        );
    }

    let final_path =
        canonical_parent.join(
            target_path
                .file_name()
                .unwrap_or_default(),
        );

    /*
     * Refuse to overwrite an existing directory with a file upload.
     */
    if final_path.is_dir() {
        return error_response(
            server,
            StatusCode::Forbidden,
        );
    }

    match fs::write(&final_path, &request.body) {
        Ok(()) => {
            HttpResponse::new(
                StatusCode::Created,
                Vec::new(),
            )
            .with_header(
                "Location",
                request.path.clone(),
            )
        }

        Err(err) => match err.kind() {
            io::ErrorKind::PermissionDenied => {
                error_response(
                    server,
                    StatusCode::Forbidden,
                )
            }

            _ => {
                error_response(
                    server,
                    StatusCode::InternalServerError,
                )
            }
        },
    }
}

/// Handles DELETE: removes a single file at `root` joined with
/// `relative_path`.
///
/// Directories are never deleted (returns 403), even if the
/// filesystem would allow it - the spec calls out "directories where
/// deletion is not allowed" as a case to handle explicitly, and this
/// keeps DELETE's blast radius limited to exactly one file, matching
/// how POST/GET are also single-file operations here.
fn handle_delete(
    server: &ServerConfig,
    root: &Path,
    relative_path: &str,
) -> HttpResponse {
    if relative_path.is_empty()
        || relative_path.ends_with('/')
    {
        return error_response(
            server,
            StatusCode::Forbidden,
        );
    }

    let target_path =
        root.join(relative_path);

    let canonical_root =
        match fs::canonicalize(root) {
            Ok(path) => path,

            Err(_) => {
                return error_response(
                    server,
                    StatusCode::InternalServerError,
                );
            }
        };

    let canonical_target =
        match fs::canonicalize(&target_path) {
            Ok(path) => path,

            Err(err) => {
                return match err.kind() {
                    io::ErrorKind::NotFound => {
                        error_response(
                            server,
                            StatusCode::NotFound,
                        )
                    }

                    io::ErrorKind::PermissionDenied => {
                        error_response(
                            server,
                            StatusCode::Forbidden,
                        )
                    }

                    _ => {
                        error_response(
                            server,
                            StatusCode::InternalServerError,
                        )
                    }
                };
            }
        };

    /*
     * Same containment rule as GET/POST: the resolved target must
     * never escape the configured route root, even through symlinks.
     */
    if !canonical_target.starts_with(&canonical_root) {
        return error_response(
            server,
            StatusCode::Forbidden,
        );
    }

    if canonical_target.is_dir() {
        return error_response(
            server,
            StatusCode::Forbidden,
        );
    }

    match fs::remove_file(&canonical_target) {
        Ok(()) => {
            HttpResponse::new(
                StatusCode::NoContent,
                Vec::new(),
            )
        }

        Err(err) => match err.kind() {
            io::ErrorKind::NotFound => {
                error_response(
                    server,
                    StatusCode::NotFound,
                )
            }

            io::ErrorKind::PermissionDenied => {
                error_response(
                    server,
                    StatusCode::Forbidden,
                )
            }

            _ => {
                error_response(
                    server,
                    StatusCode::InternalServerError,
                )
            }
        },
    }
}

pub fn error_response(
    server: &ServerConfig,
    status: StatusCode,
) -> HttpResponse {
    let key =
        status.code().to_string();

    /*
     * First try the custom error page from config.toml.
     */
    if let Some(path) =
        server.error_pages.get(&key)
    {
        if let Ok(body) = fs::read(path) {
            return HttpResponse::new(
                status,
                body,
            )
            .with_header(
                "Content-Type",
                "text/html; charset=utf-8",
            );
        }
    }

    /*
     * If custom page does not exist,
     * the server must still return a valid response.
     */
    default_error_response(status)
}

pub fn default_error_response(
    status: StatusCode,
) -> HttpResponse {
    let html = format!(
        "<!DOCTYPE html>\
<html>\
<head>\
<meta charset=\"utf-8\">\
<title>{0} {1}</title>\
</head>\
<body>\
<h1>{0} {1}</h1>\
<hr>\
<p>localhost-rust</p>\
</body>\
</html>",
        status.code(),
        status.reason_phrase()
    );

    HttpResponse::html(
        status,
        html,
    )
}

/// Deny-all default: a route with no configured methods accepts
/// nothing until methods are explicitly listed.
fn method_allowed(
    route: &RouteConfig,
    method: &Method,
) -> bool {
    route
        .methods
        .iter()
        .any(|allowed| allowed.as_str() == method.as_str())
}

fn allow_header_value(
    route: &RouteConfig,
) -> String {
    route.methods.join(", ")
}

fn is_safe_relative_path(
    path: &str,
) -> bool {
    /*
     * Backslashes are directory separators on Windows.
     *
     * Reject them explicitly so:
     *
     * ..\..\file
     *
     * cannot become a traversal attack.
     */
    if path.contains('\\')
        || path.contains('\0')
    {
        return false;
    }

    let path =
        Path::new(path);

    for component in path.components() {
        match component {
            Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return false;
            }

            Component::CurDir
            | Component::Normal(_) => {}
        }
    }

    true
}

fn render_directory_listing(
    dir: &Path,
    request_path: &str,
) -> io::Result<HttpResponse> {
    let mut entries: Vec<String> =
        fs::read_dir(dir)?
            .filter_map(|entry| entry.ok())
            .map(|entry| {
                let name = entry
                    .file_name()
                    .to_string_lossy()
                    .into_owned();

                let is_dir = entry
                    .file_type()
                    .map(|ft| ft.is_dir())
                    .unwrap_or(false);

                if is_dir {
                    format!("{}/", name)
                } else {
                    name
                }
            })
            .collect();

    entries.sort();

    let display_path =
        if request_path.ends_with('/') {
            request_path.to_string()
        } else {
            format!("{}/", request_path)
        };

    let mut list_items = String::new();

    for name in &entries {
        let href = html_escape(&format!("{}{}", display_path, name));
        let label = html_escape(name);

        list_items.push_str(&format!(
            "<li><a href=\"{0}\">{1}</a></li>",
            href, label
        ));
    }

    let html = format!(
        "<!DOCTYPE html>\
<html>\
<head>\
<meta charset=\"utf-8\">\
<title>Index of {0}</title>\
</head>\
<body>\
<h1>Index of {0}</h1>\
<ul>{1}</ul>\
<hr>\
<p>localhost-rust</p>\
</body>\
</html>",
        html_escape(&display_path),
        list_items,
    );

    Ok(HttpResponse::html(
        StatusCode::Ok,
        html,
    ))
}

fn html_escape(
    input: &str,
) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn content_type_for_path(
    path: &Path,
) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("html") | Some("htm") => {
            "text/html; charset=utf-8"
        }

        Some("css") => {
            "text/css; charset=utf-8"
        }

        Some("js") => {
            "application/javascript; charset=utf-8"
        }

        Some("json") => {
            "application/json; charset=utf-8"
        }

        Some("txt") => {
            "text/plain; charset=utf-8"
        }

        Some("png") => {
            "image/png"
        }

        Some("jpg")
        | Some("jpeg") => {
            "image/jpeg"
        }

        Some("gif") => {
            "image/gif"
        }

        Some("svg") => {
            "image/svg+xml"
        }

        Some("ico") => {
            "image/x-icon"
        }

        Some("pdf") => {
            "application/pdf"
        }

        Some("xml") => {
            "application/xml"
        }

        Some("wasm") => {
            "application/wasm"
        }

        _ => {
            "application/octet-stream"
        }
    }
}