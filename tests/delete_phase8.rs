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
        .join(format!("localhost-phase8-delete-{}", unique));

    fs::create_dir_all(&path).unwrap();

    path
}

fn delete_request(path: &str) -> HttpRequest {
    HttpRequest {
        method: Method::Delete,
        target: path.to_string(),
        path: path.to_string(),
        query: None,
        version: HttpVersion::Http11,
        headers: Vec::new(),
        body: Vec::new(),
    }
}

fn delete_route(root: String) -> (ServerConfig, RouteConfig) {
    let route = RouteConfig {
        path: "/files".to_string(),
        methods: vec!["GET".to_string(), "DELETE".to_string()],
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
        client_max_body_size: 1024,
        routes: vec![route.clone()],
    };

    (server, route)
}

#[test]
fn delete_removes_existing_file_and_returns_204() {
    let root = temporary_directory();

    fs::write(root.join("doomed.txt"), b"delete me").unwrap();

    let (server, route) =
        delete_route(root.to_string_lossy().to_string());

    let response = handle_static_request(
        &delete_request("/files/doomed.txt"),
        &server,
        &route,
    );

    assert_eq!(response.status, StatusCode::NoContent);
    assert!(!root.join("doomed.txt").exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_missing_file_returns_404() {
    let root = temporary_directory();

    let (server, route) =
        delete_route(root.to_string_lossy().to_string());

    let response = handle_static_request(
        &delete_request("/files/missing.txt"),
        &server,
        &route,
    );

    assert_eq!(response.status, StatusCode::NotFound);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_directory_is_forbidden() {
    let root = temporary_directory();

    fs::create_dir(root.join("subdir")).unwrap();

    let (server, route) =
        delete_route(root.to_string_lossy().to_string());

    let response = handle_static_request(
        &delete_request("/files/subdir"),
        &server,
        &route,
    );

    assert_eq!(response.status, StatusCode::Forbidden);
    assert!(root.join("subdir").exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_traversal_attempt_is_forbidden() {
    let root = temporary_directory();

    let (server, route) =
        delete_route(root.to_string_lossy().to_string());

    let response = handle_static_request(
        &delete_request("/files/../../etc/passwd"),
        &server,
        &route,
    );

    assert_eq!(response.status, StatusCode::Forbidden);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_root_path_is_forbidden() {
    let root = temporary_directory();

    let (server, route) =
        delete_route(root.to_string_lossy().to_string());

    let response = handle_static_request(
        &delete_request("/files/"),
        &server,
        &route,
    );

    assert_eq!(response.status, StatusCode::Forbidden);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_on_route_without_delete_method_returns_405() {
    let root = temporary_directory();

    fs::write(root.join("protected.txt"), b"keep me").unwrap();

    let route = RouteConfig {
        path: "/files".to_string(),
        methods: vec!["GET".to_string()],
        root: Some(root.to_string_lossy().to_string()),
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
        client_max_body_size: 1024,
        routes: vec![route.clone()],
    };

    let response = handle_static_request(
        &delete_request("/files/protected.txt"),
        &server,
        &route,
    );

    assert_eq!(response.status, StatusCode::MethodNotAllowed);
    assert!(root.join("protected.txt").exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn get_after_delete_returns_404() {
    let root = temporary_directory();

    fs::write(root.join("temp.txt"), b"temporary").unwrap();

    let (server, route) =
        delete_route(root.to_string_lossy().to_string());

    let delete_response = handle_static_request(
        &delete_request("/files/temp.txt"),
        &server,
        &route,
    );

    assert_eq!(delete_response.status, StatusCode::NoContent);

    let get_request = HttpRequest {
        method: Method::Get,
        target: "/files/temp.txt".to_string(),
        path: "/files/temp.txt".to_string(),
        query: None,
        version: HttpVersion::Http11,
        headers: Vec::new(),
        body: Vec::new(),
    };

    let get_response =
        handle_static_request(&get_request, &server, &route);

    assert_eq!(get_response.status, StatusCode::NotFound);

    let _ = fs::remove_dir_all(root);
}
