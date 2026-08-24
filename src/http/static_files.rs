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

pub fn handle_static_request(
    request: &HttpRequest,
    server: &ServerConfig,
) -> HttpResponse {
    if request.method != Method::Get {
        return error_response(
            server,
            StatusCode::MethodNotAllowed,
        )
        .with_header(
            "Allow",
            "GET",
        );
    }

    /*
     * Phase 5 deliberately uses only the root route.
     *
     * Full route selection is implemented in Phase 6.
     */
    let route =
        match root_route(server) {
            Some(route) => route,

            None => {
                return error_response(
                    server,
                    StatusCode::InternalServerError,
                );
            }
        };

    let root =
        match &route.root {
            Some(root) => PathBuf::from(root),

            None => {
                return error_response(
                    server,
                    StatusCode::InternalServerError,
                );
            }
        };

    let relative_path =
        request
            .path
            .trim_start_matches('/');

    if !is_safe_relative_path(relative_path) {
        return error_response(
            server,
            StatusCode::Forbidden,
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
        match fs::canonicalize(&root) {
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

fn root_route(
    server: &ServerConfig,
) -> Option<&RouteConfig> {
    server
        .routes
        .iter()
        .find(|route| {
            route.path == "/"
                && route.root.is_some()
        })
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
        let href = html_escape(name);

        list_items.push_str(&format!(
            "<li><a href=\"{0}\">{0}</a></li>",
            href
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