use std::net::Ipv4Addr;

use crate::config::Config;

use crate::http::{
    default_error_response,
    error_response,
    handle_static_request,
    HttpRequest,
    HttpResponse,
    StatusCode,
};

use crate::server::routing::{
    select_route,
    select_server,
};

pub fn handle_request(
    config: &Config,
    request: &HttpRequest,
    local_addr: Ipv4Addr,
    local_port: u16,
) -> HttpResponse {
    let host = request.header("Host");

    let server =
        match select_server(
            config,
            local_addr,
            local_port,
            host,
        ) {
            Some(server) => server,

            None => {
                return default_error_response(
                    StatusCode::InternalServerError,
                );
            }
        };

    let route =
        match select_route(server, &request.path) {
            Some(route) => route,

            None => {
                return error_response(
                    server,
                    StatusCode::NotFound,
                );
            }
        };

    handle_static_request(
        request,
        server,
        route,
    )
}

pub fn bad_request_response(
    config: &Config,
) -> HttpResponse {
    match config.servers.first() {
        Some(server) => {
            error_response(
                server,
                StatusCode::BadRequest,
            )
            .with_header(
                "Connection",
                "close",
            )
        }

        None => {
            default_error_response(
                StatusCode::BadRequest,
            )
            .with_header(
                "Connection",
                "close",
            )
        }
    }
}

pub fn payload_too_large_response(
    config: &Config,
) -> HttpResponse {
    match config.servers.first() {
        Some(server) => {
            error_response(
                server,
                StatusCode::PayloadTooLarge,
            )
            .with_header(
                "Connection",
                "close",
            )
        }

        None => {
            default_error_response(
                StatusCode::PayloadTooLarge,
            )
            .with_header(
                "Connection",
                "close",
            )
        }
    }
}