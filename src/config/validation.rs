use std::collections::{
    HashMap,
    HashSet,
};

use std::net::Ipv4Addr;

use super::model::{
    Config,
    RouteConfig,
};

pub fn validate_config(
    config: &Config,
) -> Result<(), String> {
    if config.servers.is_empty() {
        return Err(
            "configuration must contain at least one server"
                .to_string(),
        );
    }

    let mut listener_occurrences:
        HashMap<(Ipv4Addr, u16), usize> =
        HashMap::new();

    let mut listener_names:
        HashMap<(Ipv4Addr, u16), HashSet<String>> =
        HashMap::new();

    for (server_index, server)
        in config.servers.iter().enumerate()
    {
        let address =
            server
                .server_address
                .parse::<Ipv4Addr>()
                .map_err(|_| {
                    format!(
                        "server {} has invalid IPv4 address '{}'",
                        server_index,
                        server.server_address
                    )
                })?;

        if server.ports.is_empty() {
            return Err(format!(
                "server {} must define at least one port",
                server_index
            ));
        }

        let mut local_ports = HashSet::new();

        for &port in &server.ports {
            if port == 0 {
                return Err(format!(
                    "server {} contains invalid port 0",
                    server_index
                ));
            }

            if !local_ports.insert(port) {
                return Err(format!(
                    "server {} contains duplicate port {}",
                    server_index,
                    port
                ));
            }

            let listener = (address, port);

            let already_present =
                *listener_occurrences
                    .get(&listener)
                    .unwrap_or(&0);

            if already_present > 0
                && server.server_name.is_empty()
            {
                return Err(format!(
                    "server {} shares {}:{} with another server but has no server_name",
                    server_index,
                    address,
                    port
                ));
            }

            let names =
                listener_names
                    .entry(listener)
                    .or_default();

            for server_name in &server.server_name {
                let normalized =
                    server_name
                        .trim()
                        .to_ascii_lowercase();

                if normalized.is_empty() {
                    return Err(format!(
                        "server {} contains an empty server_name",
                        server_index
                    ));
                }

                if !names.insert(normalized.clone()) {
                    return Err(format!(
                        "duplicate server_name '{}' on {}:{}",
                        server_name,
                        address,
                        port
                    ));
                }
            }

            *listener_occurrences
                .entry(listener)
                .or_insert(0) += 1;
        }

        if server.client_max_body_size == 0 {
            return Err(format!(
                "server {} client_max_body_size must be greater than zero",
                server_index
            ));
        }

        validate_error_pages(
            server_index,
            &server.error_pages,
        )?;

        let mut route_paths = HashSet::new();

        for route in &server.routes {
            validate_route(
                server_index,
                route,
            )?;

            if !route_paths.insert(
                route.path.clone(),
            ) {
                return Err(format!(
                    "server {} contains duplicate route '{}'",
                    server_index,
                    route.path
                ));
            }
        }
    }

    Ok(())
}

fn validate_error_pages(
    server_index: usize,
    error_pages: &HashMap<String, String>,
) -> Result<(), String> {
    for (code, path) in error_pages {
        let status =
            code.parse::<u16>().map_err(|_| {
                format!(
                    "server {} has invalid error status '{}'",
                    server_index,
                    code
                )
            })?;

        if !(400..=599).contains(&status) {
            return Err(format!(
                "server {} error page status {} is not an error status",
                server_index,
                status
            ));
        }

        if path.trim().is_empty() {
            return Err(format!(
                "server {} error page {} has empty path",
                server_index,
                status
            ));
        }
    }

    Ok(())
}

fn validate_route(
    server_index: usize,
    route: &RouteConfig,
) -> Result<(), String> {
    if !route.path.starts_with('/') {
        return Err(format!(
            "server {} route '{}' must start with '/'",
            server_index,
            route.path
        ));
    }

    let valid_methods = [
        "GET",
        "POST",
        "DELETE",
    ];

    for method in &route.methods {
        if !valid_methods.contains(
            &method.as_str(),
        ) {
            return Err(format!(
                "server {} route '{}' contains unsupported method '{}'",
                server_index,
                route.path,
                method
            ));
        }
    }

    if route.redirect.is_some()
        && route.root.is_some()
    {
        return Err(format!(
            "server {} route '{}' cannot contain both redirect and root",
            server_index,
            route.path
        ));
    }

    if route.redirect_status.is_some()
        && route.redirect.is_none()
    {
        return Err(format!(
            "server {} route '{}' defines redirect_status without redirect",
            server_index,
            route.path
        ));
    }

    if let Some(status) = route.redirect_status {
        if !(300..=399).contains(&status) {
            return Err(format!(
                "server {} route '{}' has invalid redirect status {}",
                server_index,
                route.path,
                status
            ));
        }
    }

    if route.index.is_some()
        && route.root.is_none()
    {
        return Err(format!(
            "server {} route '{}' defines index without root",
            server_index,
            route.path
        ));
    }

    if route.directory_listing
        && route.root.is_none()
    {
        return Err(format!(
            "server {} route '{}' enables directory listing without root",
            server_index,
            route.path
        ));
    }

    for (extension, executable)
        in &route.cgi
    {
        if !extension.starts_with('.') {
            return Err(format!(
                "server {} route '{}' CGI extension '{}' must start with '.'",
                server_index,
                route.path,
                extension
            ));
        }

        if extension.len() <= 1 {
            return Err(format!(
                "server {} route '{}' contains empty CGI extension",
                server_index,
                route.path
            ));
        }

        if executable.trim().is_empty() {
            return Err(format!(
                "server {} route '{}' CGI '{}' has empty executable",
                server_index,
                route.path,
                extension
            ));
        }
    }

    if route.root.is_none()
        && route.redirect.is_none()
        && route.cgi.is_empty()
    {
        return Err(format!(
            "server {} route '{}' does not define root, redirect or CGI",
            server_index,
            route.path
        ));
    }

    Ok(())
}