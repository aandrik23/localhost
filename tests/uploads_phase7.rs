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
    resolve_route,
    HttpRequest,
    HttpResponse,
    HttpVersion,
    Method,
    RouteOutcome,
    StatusCode,
};

fn handle_static_request(
    request: &HttpRequest,
    server: &ServerConfig,
    route: &RouteConfig,
) -> HttpResponse {
    match resolve_route(request, server, route) {
        RouteOutcome::Response(response) => response,

        RouteOutcome::Cgi { .. } => {
            panic!("expected a direct response, got Cgi")
        }
    }
}

fn temporary_directory() -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let path = std::env::temp_dir()
        .join(format!("localhost-phase7-{}", unique));

    fs::create_dir_all(&path).unwrap();

    path
}

fn post_request(
    path: &str,
    body: &[u8],
) -> HttpRequest {
    HttpRequest {
        method: Method::Post,
        target: path.to_string(),
        path: path.to_string(),
        query: None,
        version: HttpVersion::Http11,
        headers: Vec::new(),
        body: body.to_vec(),
    }
}

fn upload_route(
    root: String,
    max_body_size: usize,
) -> (ServerConfig, RouteConfig) {
    let route = RouteConfig {
        path: "/uploads".to_string(),
        methods: vec!["GET".to_string(), "POST".to_string()],
        root: Some(root),
        index: None,
        directory_listing: false,
        redirect: None,
        redirect_status: None,
        cgi: HashMap::new(),
    };

    let server = ServerConfig {
        server_address: "127.0.0.1".to_string(),
        ports: vec![8080],
        server_name: vec!["localhost".to_string()],
        error_pages: HashMap::new(),
        client_max_body_size: max_body_size,
        routes: vec![route.clone()],
    };

    (server, route)
}

#[test]
fn post_writes_file_and_returns_201() {
    let root = temporary_directory();

    let (server, route) =
        upload_route(root.to_string_lossy().to_string(), 1024);

    let response = handle_static_request(
        &post_request("/uploads/note.txt", b"hello upload"),
        &server,
        &route,
    );

    assert_eq!(response.status, StatusCode::Created);

    let location = response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Location"))
        .map(|(_, value)| value.clone())
        .unwrap();

    assert_eq!(location, "/uploads/note.txt");

    let written = fs::read(root.join("note.txt")).unwrap();
    assert_eq!(written, b"hello upload");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn post_overwrites_existing_file() {
    let root = temporary_directory();

    fs::write(root.join("note.txt"), b"old content").unwrap();

    let (server, route) =
        upload_route(root.to_string_lossy().to_string(), 1024);

    let response = handle_static_request(
        &post_request("/uploads/note.txt", b"new content"),
        &server,
        &route,
    );

    assert_eq!(response.status, StatusCode::Created);

    let written = fs::read(root.join("note.txt")).unwrap();
    assert_eq!(written, b"new content");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn post_into_nested_existing_directory_succeeds() {
    let root = temporary_directory();

    fs::create_dir(root.join("nested")).unwrap();

    let (server, route) =
        upload_route(root.to_string_lossy().to_string(), 1024);

    let response = handle_static_request(
        &post_request("/uploads/nested/file.bin", b"binary data"),
        &server,
        &route,
    );

    assert_eq!(response.status, StatusCode::Created);

    let written = fs::read(root.join("nested/file.bin")).unwrap();
    assert_eq!(written, b"binary data");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn post_into_missing_parent_directory_returns_404() {
    let root = temporary_directory();

    let (server, route) =
        upload_route(root.to_string_lossy().to_string(), 1024);

    let response = handle_static_request(
        &post_request("/uploads/missing/file.txt", b"data"),
        &server,
        &route,
    );

    assert_eq!(response.status, StatusCode::NotFound);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn post_traversal_attempt_is_forbidden() {
    let root = temporary_directory();

    let (server, route) =
        upload_route(root.to_string_lossy().to_string(), 1024);

    let response = handle_static_request(
        &post_request("/uploads/../../etc/passwd", b"data"),
        &server,
        &route,
    );

    assert_eq!(response.status, StatusCode::Forbidden);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn post_to_directory_path_is_forbidden() {
    let root = temporary_directory();

    let (server, route) =
        upload_route(root.to_string_lossy().to_string(), 1024);

    let response = handle_static_request(
        &post_request("/uploads/", b"data"),
        &server,
        &route,
    );

    assert_eq!(response.status, StatusCode::Forbidden);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn post_onto_existing_directory_name_is_forbidden() {
    let root = temporary_directory();

    fs::create_dir(root.join("existing-dir")).unwrap();

    let (server, route) =
        upload_route(root.to_string_lossy().to_string(), 1024);

    let response = handle_static_request(
        &post_request("/uploads/existing-dir", b"data"),
        &server,
        &route,
    );

    assert_eq!(response.status, StatusCode::Forbidden);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn post_over_server_body_limit_returns_413() {
    let root = temporary_directory();

    let (server, route) =
        upload_route(root.to_string_lossy().to_string(), 4);

    let response = handle_static_request(
        &post_request("/uploads/note.txt", b"this is too long"),
        &server,
        &route,
    );

    assert_eq!(response.status, StatusCode::PayloadTooLarge);

    assert!(!root.join("note.txt").exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn get_after_post_serves_uploaded_content() {
    let root = temporary_directory();

    let (server, route) =
        upload_route(root.to_string_lossy().to_string(), 1024);

    let post_response = handle_static_request(
        &post_request("/uploads/note.txt", b"round trip"),
        &server,
        &route,
    );

    assert_eq!(post_response.status, StatusCode::Created);

    let get_request = HttpRequest {
        method: Method::Get,
        target: "/uploads/note.txt".to_string(),
        path: "/uploads/note.txt".to_string(),
        query: None,
        version: HttpVersion::Http11,
        headers: Vec::new(),
        body: Vec::new(),
    };

    let get_response =
        handle_static_request(&get_request, &server, &route);

    assert_eq!(get_response.status, StatusCode::Ok);
    assert_eq!(get_response.body, b"round trip");

    let _ = fs::remove_dir_all(root);
}
