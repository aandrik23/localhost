//! Phase 12 integration tests: symlink containment.
//!
//! src/http/static_files.rs enforces the same "canonicalize, then
//! check starts_with(canonical_root)" rule for GET, POST (upload),
//! and DELETE - this file proves that rule actually holds for real
//! symlinks, which none of the earlier phase test files exercised
//! (they only tested plain path-traversal strings like "../").

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::symlink;
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
    HttpVersion,
    Method,
    RouteOutcome,
    StatusCode,
};

fn request(
    method: Method,
    path: &str,
) -> HttpRequest {
    HttpRequest {
        method,
        target: path.to_string(),
        path: path.to_string(),
        query: None,
        version: HttpVersion::Http11,
        headers: Vec::new(),
        body: Vec::new(),
    }
}

fn server_with_root(root: String) -> ServerConfig {
    ServerConfig {
        server_address: "127.0.0.1".to_string(),
        ports: vec![8080],
        server_name: vec!["localhost".to_string()],
        error_pages: HashMap::new(),
        client_max_body_size: 1024 * 1024,
        routes: vec![RouteConfig {
            path: "/".to_string(),
            methods: vec![
                "GET".to_string(),
                "DELETE".to_string(),
            ],
            root: Some(root),
            index: Some("index.html".to_string()),
            directory_listing: false,
            redirect: None,
            redirect_status: None,
            cgi: HashMap::new(),
        }],
    }
}

fn temporary_directory(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let path = std::env::temp_dir()
        .join(format!("localhost-phase12-{}-{}", label, unique));

    fs::create_dir_all(&path).unwrap();

    path
}

#[test]
fn symlink_resolving_inside_root_is_served() {
    let root = temporary_directory("inside-root");

    let real_dir = root.join("real");
    fs::create_dir(&real_dir).unwrap();
    fs::write(real_dir.join("secret.txt"), b"inside root").unwrap();

    // A symlink inside the root pointing at another file also inside
    // the root: this must resolve normally.
    symlink(
        real_dir.join("secret.txt"),
        root.join("link.txt"),
    )
    .unwrap();

    let server = server_with_root(root.to_string_lossy().to_string());

    let response = match resolve_route(
        &request(Method::Get, "/link.txt"),
        &server,
        &server.routes[0],
    ) {
        RouteOutcome::Response(response) => response,
        RouteOutcome::Cgi { .. } => panic!("expected a direct response"),
    };

    assert_eq!(response.status, StatusCode::Ok);
    assert_eq!(response.body, b"inside root");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn symlink_escaping_root_is_forbidden() {
    let root = temporary_directory("escape-root");
    let outside = temporary_directory("escape-target");

    fs::write(outside.join("secret.txt"), b"outside root").unwrap();

    // A symlink that lives inside the configured root but resolves to
    // a file outside it must be rejected with 403, not served.
    symlink(
        outside.join("secret.txt"),
        root.join("escape.txt"),
    )
    .unwrap();

    let server = server_with_root(root.to_string_lossy().to_string());

    let response = match resolve_route(
        &request(Method::Get, "/escape.txt"),
        &server,
        &server.routes[0],
    ) {
        RouteOutcome::Response(response) => response,
        RouteOutcome::Cgi { .. } => panic!("expected a direct response"),
    };

    assert_eq!(response.status, StatusCode::Forbidden);

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&outside);
}

#[test]
fn symlinked_directory_escaping_root_is_forbidden() {
    let root = temporary_directory("escape-root-dir");
    let outside = temporary_directory("escape-target-dir");

    fs::write(outside.join("file.txt"), b"outside root").unwrap();

    // A symlink to a directory outside the root: requesting a file
    // through that symlinked directory must also be rejected.
    symlink(&outside, root.join("outside_link")).unwrap();

    let server = server_with_root(root.to_string_lossy().to_string());

    let response = match resolve_route(
        &request(Method::Get, "/outside_link/file.txt"),
        &server,
        &server.routes[0],
    ) {
        RouteOutcome::Response(response) => response,
        RouteOutcome::Cgi { .. } => panic!("expected a direct response"),
    };

    assert_eq!(response.status, StatusCode::Forbidden);

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&outside);
}

#[test]
fn delete_through_symlink_escaping_root_is_forbidden() {
    let root = temporary_directory("escape-root-delete");
    let outside = temporary_directory("escape-target-delete");

    fs::write(outside.join("victim.txt"), b"do not delete me").unwrap();

    symlink(
        outside.join("victim.txt"),
        root.join("victim_link.txt"),
    )
    .unwrap();

    let server = server_with_root(root.to_string_lossy().to_string());

    let response = match resolve_route(
        &request(Method::Delete, "/victim_link.txt"),
        &server,
        &server.routes[0],
    ) {
        RouteOutcome::Response(response) => response,
        RouteOutcome::Cgi { .. } => panic!("expected a direct response"),
    };

    assert_eq!(response.status, StatusCode::Forbidden);

    // The file outside the root must still exist - the escape was
    // correctly blocked, not just mis-reported.
    assert!(outside.join("victim.txt").exists());

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&outside);
}
