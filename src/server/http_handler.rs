use crate::config::Config;

use crate::http::{
    default_error_response,
    error_response,
    handle_static_request,
    HttpRequest,
    HttpResponse,
    StatusCode,
};

pub fn handle_request(
    config: &Config,
    request: &HttpRequest,
) -> HttpResponse {
    /*
     * Phase 5 uses the first configured server.
     *
     * Phase 6 will use:
     *
     * - listener address
     * - listener port
     * - Host header
     * - route matching
     *
     * to select the correct virtual server.
     */
    let server =
        match config.servers.first() {
            Some(server) => server,

            None => {
                return default_error_response(
                    StatusCode::InternalServerError,
                );
            }
        };

    handle_static_request(
        request,
        server,
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