//! Virtual-host and route selection.
//!
//! Server selection uses (local address, local port, Host header).
//! Route selection uses longest matching path segment prefix.
//!
//! Neither function performs any I/O; both operate purely on the
//! already-parsed configuration and request.

use std::net::Ipv4Addr;

use crate::config::{
    Config,
    RouteConfig,
    ServerConfig,
};

/// Selects the virtual server that should handle a request arriving on
/// `local_addr:local_port`, using the `Host` header to disambiguate
/// between virtual servers sharing the same listener.
///
/// If no `server_name` matches (or no Host header is present), the
/// first configured server bound to that listener is used, per the
/// project specification's "first server is the default" rule.
pub fn select_server<'a>(
    config: &'a Config,
    local_addr: Ipv4Addr,
    local_port: u16,
    host: Option<&str>,
) -> Option<&'a ServerConfig> {
    let candidates: Vec<&ServerConfig> = config
        .servers
        .iter()
        .filter(|server| {
            server_binds(server, local_addr, local_port)
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }

    if let Some(host) = host {
        let requested_name = normalize_host(host);

        if let Some(matched) = candidates.iter().find(|server| {
            server
                .server_name
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&requested_name))
        }) {
            return Some(matched);
        }
    }

    candidates.into_iter().next()
}

fn server_binds(
    server: &ServerConfig,
    local_addr: Ipv4Addr,
    local_port: u16,
) -> bool {
    let bind_address_matches = match server.server_address.parse::<Ipv4Addr>() {
        Ok(address) => {
            address == local_addr || address == Ipv4Addr::UNSPECIFIED
        }

        Err(_) => false,
    };

    bind_address_matches && server.ports.contains(&local_port)
}

/// Strips an optional ":port" suffix from a Host header value and
/// lowercases it, so "Example.com:8080" matches a configured
/// server_name of "example.com".
fn normalize_host(host: &str) -> String {
    let without_port = match host.rfind(':') {
        Some(index) => &host[..index],

        None => host,
    };

    without_port.trim().to_ascii_lowercase()
}

/// Selects the best-matching route for `path` within `server`.
///
/// Matching is longest-prefix by path segment: a route path of "/foo"
/// matches "/foo" and "/foo/bar" but not "/foobar". The root route
/// ("/") matches everything as the fallback.
pub fn select_route<'a>(
    server: &'a ServerConfig,
    path: &str,
) -> Option<&'a RouteConfig> {
    server
        .routes
        .iter()
        .filter(|route| path_matches_route(path, &route.path))
        .max_by_key(|route| route.path.len())
}

fn path_matches_route(
    path: &str,
    route_path: &str,
) -> bool {
    if route_path == "/" {
        return true;
    }

    if !path.starts_with(route_path) {
        return false;
    }

    /*
     * Require a segment boundary right after the route path so that
     * "/foo" does not match "/foobar".
     */
    match path.as_bytes().get(route_path.len()) {
        None => true,

        Some(b'/') => true,

        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn server(
        address: &str,
        ports: Vec<u16>,
        names: Vec<&str>,
    ) -> ServerConfig {
        ServerConfig {
            server_address: address.to_string(),
            ports,
            server_name: names.into_iter().map(String::from).collect(),
            error_pages: HashMap::new(),
            client_max_body_size: 1024,
            routes: Vec::new(),
        }
    }

    fn route(path: &str) -> RouteConfig {
        RouteConfig {
            path: path.to_string(),
            methods: Vec::new(),
            root: Some("./www".to_string()),
            index: None,
            directory_listing: false,
            redirect: None,
            redirect_status: None,
            cgi: HashMap::new(),
        }
    }

    #[test]
    fn selects_server_by_host_header() {
        let config = Config {
            servers: vec![
                server("0.0.0.0", vec![8080], vec!["a.test"]),
                server("0.0.0.0", vec![8080], vec!["b.test"]),
            ],
        };

        let selected = select_server(
            &config,
            Ipv4Addr::new(127, 0, 0, 1),
            8080,
            Some("b.test"),
        )
        .unwrap();

        assert_eq!(selected.server_name, vec!["b.test".to_string()]);
    }

    #[test]
    fn falls_back_to_first_server_when_host_unmatched() {
        let config = Config {
            servers: vec![
                server("0.0.0.0", vec![8080], vec!["a.test"]),
                server("0.0.0.0", vec![8080], vec!["b.test"]),
            ],
        };

        let selected = select_server(
            &config,
            Ipv4Addr::new(127, 0, 0, 1),
            8080,
            Some("unknown.test"),
        )
        .unwrap();

        assert_eq!(selected.server_name, vec!["a.test".to_string()]);
    }

    #[test]
    fn host_header_port_suffix_is_ignored() {
        let config = Config {
            servers: vec![server("0.0.0.0", vec![8080], vec!["a.test"])],
        };

        let selected = select_server(
            &config,
            Ipv4Addr::new(127, 0, 0, 1),
            8080,
            Some("A.Test:8080"),
        )
        .unwrap();

        assert_eq!(selected.server_name, vec!["a.test".to_string()]);
    }

    #[test]
    fn does_not_select_server_on_unbound_port() {
        let config = Config {
            servers: vec![server("0.0.0.0", vec![8080], vec!["a.test"])],
        };

        let selected = select_server(
            &config,
            Ipv4Addr::new(127, 0, 0, 1),
            9090,
            Some("a.test"),
        );

        assert!(selected.is_none());
    }

    #[test]
    fn route_matching_prefers_longest_prefix() {
        let server = ServerConfig {
            routes: vec![route("/"), route("/uploads")],
            ..server("0.0.0.0", vec![8080], vec![])
        };

        let selected = select_route(&server, "/uploads/file.txt").unwrap();

        assert_eq!(selected.path, "/uploads");
    }

    #[test]
    fn route_matching_requires_segment_boundary() {
        let server = ServerConfig {
            routes: vec![route("/foo")],
            ..server("0.0.0.0", vec![8080], vec![])
        };

        assert!(select_route(&server, "/foobar").is_none());

        assert!(select_route(&server, "/foo/bar").is_some());
    }
}
