use std::collections::HashMap;
use std::fs;
use std::net::Ipv4Addr;
use std::time::{
    SystemTime,
    UNIX_EPOCH,
};

use localhost::config::{
    Config,
    RouteConfig,
    ServerConfig,
};

use localhost::http::{
    HttpRequest,
    HttpVersion,
    Method,
    StatusCode,
};

use localhost::server::http_handler::handle_request;
use localhost::server::routing::{
    select_route,
    select_server,
};
use localhost::server::session::SessionStore;

fn temporary_directory() -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let path = std::env::temp_dir()
        .join(format!("localhost-phase6-{}", unique));

    fs::create_dir_all(&path).unwrap();

    path
}

fn request_with_host(
    method: Method,
    path: &str,
    host: Option<&str>,
) -> HttpRequest {
    let mut headers = Vec::new();

    if let Some(host) = host {
        headers.push(localhost::http::Header {
            name: "Host".to_string(),
            value: host.to_string(),
        });
    }

    HttpRequest {
        method,
        target: path.to_string(),
        path: path.to_string(),
        query: None,
        version: HttpVersion::Http11,
        headers,
        body: Vec::new(),
    }
}

fn route(
    path: &str,
    methods: &[&str],
    root: Option<&str>,
) -> RouteConfig {
    RouteConfig {
        path: path.to_string(),
        methods: methods.iter().map(|m| m.to_string()).collect(),
        root: root.map(String::from),
        index: Some("index.html".to_string()),
        directory_listing: false,
        redirect: None,
        redirect_status: None,
        cgi: HashMap::new(),
    }
}

fn server(
    names: &[&str],
    ports: Vec<u16>,
    routes: Vec<RouteConfig>,
) -> ServerConfig {
    ServerConfig {
        server_address: "127.0.0.1".to_string(),
        ports,
        server_name: names.iter().map(|n| n.to_string()).collect(),
        error_pages: HashMap::new(),
        client_max_body_size: 1024 * 1024,
        routes,
    }
}

#[test]
fn selects_virtual_server_by_host_header() {
    let root_a = temporary_directory();
    let root_b = temporary_directory();

    fs::write(root_a.join("index.html"), b"site A").unwrap();
    fs::write(root_b.join("index.html"), b"site B").unwrap();

    let config = Config {
        servers: vec![
            server(
                &["a.test"],
                vec![8080],
                vec![route("/", &["GET"], Some(&root_a.to_string_lossy()))],
            ),
            server(
                &["b.test"],
                vec![8080],
                vec![route("/", &["GET"], Some(&root_b.to_string_lossy()))],
            ),
        ],
    };

    let response = handle_request(
        &config,
        &request_with_host(Method::Get, "/", Some("b.test")),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut SessionStore::new(),
    );

    assert_eq!(response.status, StatusCode::Ok);
    assert_eq!(response.body, b"site B");

    let _ = fs::remove_dir_all(root_a);
    let _ = fs::remove_dir_all(root_b);
}

#[test]
fn unmatched_host_falls_back_to_default_server() {
    let root_a = temporary_directory();
    let root_b = temporary_directory();

    fs::write(root_a.join("index.html"), b"default site").unwrap();
    fs::write(root_b.join("index.html"), b"site B").unwrap();

    let config = Config {
        servers: vec![
            server(
                &["a.test"],
                vec![8080],
                vec![route("/", &["GET"], Some(&root_a.to_string_lossy()))],
            ),
            server(
                &["b.test"],
                vec![8080],
                vec![route("/", &["GET"], Some(&root_b.to_string_lossy()))],
            ),
        ],
    };

    let response = handle_request(
        &config,
        &request_with_host(Method::Get, "/", Some("unknown.test")),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut SessionStore::new(),
    );

    assert_eq!(response.status, StatusCode::Ok);
    assert_eq!(response.body, b"default site");

    let _ = fs::remove_dir_all(root_a);
    let _ = fs::remove_dir_all(root_b);
}

#[test]
fn different_ports_select_different_servers() {
    let root_8080 = temporary_directory();
    let root_9090 = temporary_directory();

    fs::write(root_8080.join("index.html"), b"port 8080").unwrap();
    fs::write(root_9090.join("index.html"), b"port 9090").unwrap();

    let config = Config {
        servers: vec![
            server(
                &[],
                vec![8080],
                vec![route("/", &["GET"], Some(&root_8080.to_string_lossy()))],
            ),
            server(
                &[],
                vec![9090],
                vec![route("/", &["GET"], Some(&root_9090.to_string_lossy()))],
            ),
        ],
    };

    let response = handle_request(
        &config,
        &request_with_host(Method::Get, "/", None),
        Ipv4Addr::new(127, 0, 0, 1),
        9090,
        &mut SessionStore::new(),
    );

    assert_eq!(response.status, StatusCode::Ok);
    assert_eq!(response.body, b"port 9090");

    let _ = fs::remove_dir_all(root_8080);
    let _ = fs::remove_dir_all(root_9090);
}

#[test]
fn route_with_no_configured_methods_denies_all() {
    let root = temporary_directory();

    fs::write(root.join("index.html"), b"hello").unwrap();

    let config = Config {
        servers: vec![server(
            &["a.test"],
            vec![8080],
            vec![route("/", &[], Some(&root.to_string_lossy()))],
        )],
    };

    let response = handle_request(
        &config,
        &request_with_host(Method::Get, "/", Some("a.test")),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut SessionStore::new(),
    );

    assert_eq!(response.status, StatusCode::MethodNotAllowed);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn method_not_allowed_includes_allow_header() {
    let root = temporary_directory();

    fs::write(root.join("index.html"), b"hello").unwrap();

    let config = Config {
        servers: vec![server(
            &["a.test"],
            vec![8080],
            vec![route("/", &["GET", "POST"], Some(&root.to_string_lossy()))],
        )],
    };

    let response = handle_request(
        &config,
        &request_with_host(Method::Delete, "/", Some("a.test")),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut SessionStore::new(),
    );

    assert_eq!(response.status, StatusCode::MethodNotAllowed);

    let allow = response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Allow"))
        .map(|(_, value)| value.clone())
        .unwrap();

    assert!(allow.contains("GET"));
    assert!(allow.contains("POST"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn redirect_route_returns_302_and_location() {
    let config = Config {
        servers: vec![server(
            &["a.test"],
            vec![8080],
            vec![RouteConfig {
                path: "/old".to_string(),
                methods: vec!["GET".to_string()],
                root: None,
                index: None,
                directory_listing: false,
                redirect: Some("/new".to_string()),
                redirect_status: None,
                cgi: HashMap::new(),
            }],
        )],
    };

    let response = handle_request(
        &config,
        &request_with_host(Method::Get, "/old", Some("a.test")),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut SessionStore::new(),
    );

    assert_eq!(response.status.code(), 302);

    let location = response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Location"))
        .map(|(_, value)| value.clone())
        .unwrap();

    assert_eq!(location, "/new");
}

#[test]
fn redirect_route_honors_configured_status() {
    let config = Config {
        servers: vec![server(
            &["a.test"],
            vec![8080],
            vec![RouteConfig {
                path: "/old".to_string(),
                methods: vec!["GET".to_string()],
                root: None,
                index: None,
                directory_listing: false,
                redirect: Some("/new".to_string()),
                redirect_status: Some(301),
                cgi: HashMap::new(),
            }],
        )],
    };

    let response = handle_request(
        &config,
        &request_with_host(Method::Get, "/old", Some("a.test")),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut SessionStore::new(),
    );

    assert_eq!(response.status.code(), 301);
}

#[test]
fn longest_matching_route_wins_over_root() {
    let root = temporary_directory();
    let uploads = temporary_directory();

    fs::write(root.join("index.html"), b"root page").unwrap();
    fs::write(uploads.join("index.html"), b"uploads page").unwrap();

    let config = Config {
        servers: vec![server(
            &["a.test"],
            vec![8080],
            vec![
                route("/", &["GET"], Some(&root.to_string_lossy())),
                route("/uploads", &["GET"], Some(&uploads.to_string_lossy())),
            ],
        )],
    };

    let response = handle_request(
        &config,
        &request_with_host(Method::Get, "/uploads/", Some("a.test")),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut SessionStore::new(),
    );

    assert_eq!(response.status, StatusCode::Ok);
    assert_eq!(response.body, b"uploads page");

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(uploads);
}

#[test]
fn no_matching_route_returns_404() {
    let config = Config {
        servers: vec![server(&["a.test"], vec![8080], vec![])],
    };

    let response = handle_request(
        &config,
        &request_with_host(Method::Get, "/anything", Some("a.test")),
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        &mut SessionStore::new(),
    );

    assert_eq!(response.status, StatusCode::NotFound);
}

#[test]
fn select_server_and_select_route_are_directly_usable() {
    let root = temporary_directory();

    let config = Config {
        servers: vec![server(
            &["a.test"],
            vec![8080],
            vec![route("/", &["GET"], Some(&root.to_string_lossy()))],
        )],
    };

    let server = select_server(
        &config,
        Ipv4Addr::new(127, 0, 0, 1),
        8080,
        Some("a.test"),
    )
    .expect("server should be selected");

    let route = select_route(server, "/").expect("route should be selected");

    assert_eq!(route.path, "/");

    let _ = fs::remove_dir_all(root);
}
