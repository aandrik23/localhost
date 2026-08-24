use std::collections::HashMap;
use std::fs;
use std::time::{
    SystemTime,
    UNIX_EPOCH,
};

use localhost::config::{
    RouteConfig,
    ServerConfig,
};

use localhost::http::{
    handle_static_request,
    HttpRequest,
    HttpVersion,
    Method,
    StatusCode,
};

fn request(
    method: Method,
    path: &str,
) -> HttpRequest {
    HttpRequest {
        method,

        target:
            path.to_string(),

        path:
            path.to_string(),

        query:
            None,

        version:
            HttpVersion::Http11,

        headers:
            Vec::new(),

        body:
            Vec::new(),
    }
}

fn server_with_root(
    root: String,
) -> ServerConfig {
    server_with_root_and_listing(
        root,
        false,
    )
}

fn server_with_root_and_listing(
    root: String,
    directory_listing: bool,
) -> ServerConfig {
    ServerConfig {
        server_address:
            "127.0.0.1".to_string(),

        ports:
            vec![8080],

        server_name:
            vec!["localhost".to_string()],

        error_pages:
            HashMap::new(),

        client_max_body_size:
            1024 * 1024,

        routes:
            vec![
                RouteConfig {
                    path:
                        "/".to_string(),

                    methods:
                        vec![
                            "GET".to_string()
                        ],

                    root:
                        Some(root),

                    index:
                        Some(
                            "index.html"
                                .to_string()
                        ),

                    directory_listing,

                    redirect:
                        None,

                    redirect_status:
                        None,

                    cgi:
                        HashMap::new(),
                }
            ],
    }
}

fn temporary_directory() -> std::path::PathBuf {
    let unique =
        SystemTime::now()
            .duration_since(
                UNIX_EPOCH
            )
            .unwrap()
            .as_nanos();

    let path =
        std::env::temp_dir()
            .join(
                format!(
                    "localhost-phase5-{}",
                    unique
                )
            );

    fs::create_dir_all(
        &path
    )
    .unwrap();

    path
}

#[test]
fn serves_index_file() {
    let root =
        temporary_directory();

    fs::write(
        root.join("index.html"),
        b"<h1>Hello</h1>",
    )
    .unwrap();

    let server =
        server_with_root(
            root
                .to_string_lossy()
                .to_string()
        );

    let response =
        handle_static_request(
            &request(
                Method::Get,
                "/",
            ),
            &server,
            &server.routes[0],
        );

    assert_eq!(
        response.status,
        StatusCode::Ok
    );

    assert_eq!(
        response.body,
        b"<h1>Hello</h1>"
    );

    let _ =
        fs::remove_dir_all(
            root
        );
}

#[test]
fn serves_static_file() {
    let root =
        temporary_directory();

    fs::write(
        root.join("hello.txt"),
        b"hello",
    )
    .unwrap();

    let server =
        server_with_root(
            root
                .to_string_lossy()
                .to_string()
        );

    let response =
        handle_static_request(
            &request(
                Method::Get,
                "/hello.txt",
            ),
            &server,
            &server.routes[0],
        );

    assert_eq!(
        response.status,
        StatusCode::Ok
    );

    assert_eq!(
        response.body,
        b"hello"
    );

    let _ =
        fs::remove_dir_all(
            root
        );
}

#[test]
fn missing_file_returns_404() {
    let root =
        temporary_directory();

    let server =
        server_with_root(
            root
                .to_string_lossy()
                .to_string()
        );

    let response =
        handle_static_request(
            &request(
                Method::Get,
                "/missing.html",
            ),
            &server,
            &server.routes[0],
        );

    assert_eq!(
        response.status,
        StatusCode::NotFound
    );

    let _ =
        fs::remove_dir_all(
            root
        );
}

#[test]
fn post_returns_405() {
    let root =
        temporary_directory();

    fs::write(
        root.join("index.html"),
        b"hello",
    )
    .unwrap();

    let server =
        server_with_root(
            root
                .to_string_lossy()
                .to_string()
        );

    let response =
        handle_static_request(
            &request(
                Method::Post,
                "/",
            ),
            &server,
            &server.routes[0],
        );

    assert_eq!(
        response.status,
        StatusCode::MethodNotAllowed
    );

    let _ =
        fs::remove_dir_all(
            root
        );
}

#[test]
fn traversal_is_forbidden() {
    let root =
        temporary_directory();

    let server =
        server_with_root(
            root
                .to_string_lossy()
                .to_string()
        );

    let response =
        handle_static_request(
            &request(
                Method::Get,
                "/../secret.txt",
            ),
            &server,
            &server.routes[0],
        );

    assert_eq!(
        response.status,
        StatusCode::Forbidden
    );

    let _ =
        fs::remove_dir_all(
            root
        );
}

#[test]
fn directory_without_index_is_forbidden_when_listing_disabled() {
    let root =
        temporary_directory();

    let server =
        server_with_root(
            root
                .to_string_lossy()
                .to_string()
        );

    let response =
        handle_static_request(
            &request(
                Method::Get,
                "/",
            ),
            &server,
            &server.routes[0],
        );

    assert_eq!(
        response.status,
        StatusCode::Forbidden
    );

    let _ =
        fs::remove_dir_all(
            root
        );
}

#[test]
fn directory_listing_is_served_when_enabled() {
    let root =
        temporary_directory();

    fs::write(
        root.join("a.txt"),
        b"a",
    )
    .unwrap();

    fs::create_dir(
        root.join("sub")
    )
    .unwrap();

    let server =
        server_with_root_and_listing(
            root
                .to_string_lossy()
                .to_string(),
            true,
        );

    let response =
        handle_static_request(
            &request(
                Method::Get,
                "/",
            ),
            &server,
            &server.routes[0],
        );

    assert_eq!(
        response.status,
        StatusCode::Ok
    );

    let body =
        String::from_utf8(
            response.body.clone()
        )
        .unwrap();

    assert!(
        body.contains(
            "a.txt"
        )
    );

    assert!(
        body.contains(
            "sub/"
        )
    );

    let _ =
        fs::remove_dir_all(
            root
        );
}

#[test]
fn response_includes_date_header() {
    let root =
        temporary_directory();

    fs::write(
        root.join("index.html"),
        b"hello",
    )
    .unwrap();

    let server =
        server_with_root(
            root
                .to_string_lossy()
                .to_string()
        );

    let response =
        handle_static_request(
            &request(
                Method::Get,
                "/",
            ),
            &server,
            &server.routes[0],
        );

    let raw =
        String::from_utf8(
            response.to_bytes()
        )
        .unwrap();

    assert!(
        raw.contains(
            "Date: "
        )
    );

    assert!(
        raw.contains(
            " GMT\r\n"
        )
    );

    let _ =
        fs::remove_dir_all(
            root
        );
}

#[test]
fn serialized_response_has_status_and_length() {
    let root =
        temporary_directory();

    fs::write(
        root.join("index.html"),
        b"hello",
    )
    .unwrap();

    let server =
        server_with_root(
            root
                .to_string_lossy()
                .to_string()
        );

    let response =
        handle_static_request(
            &request(
                Method::Get,
                "/",
            ),
            &server,
            &server.routes[0],
        );

    let raw =
        String::from_utf8(
            response.to_bytes()
        )
        .unwrap();

    assert!(
        raw.starts_with(
            "HTTP/1.1 200 OK\r\n"
        )
    );

    assert!(
        raw.contains(
            "Content-Length: 5\r\n"
        )
    );

    assert!(
        raw.ends_with(
            "\r\n\r\nhello"
        )
    );

    let _ =
        fs::remove_dir_all(
            root
        );
}