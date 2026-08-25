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

use crate::server::session::{
    SessionId,
    SessionStore,
    SESSION_COOKIE_NAME,
};

const SESSION_INFO_PATH: &str = "/session-info";

const VISIT_COUNT_KEY: &str = "visit_count";

pub fn handle_request(
    config: &Config,
    request: &HttpRequest,
    local_addr: Ipv4Addr,
    local_port: u16,
    session_store: &mut SessionStore,
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

    let (
        session_id,
        visit_count,
        is_new_session,
    ) = resolve_session(request, session_store);

    let response =
        if request.path == SESSION_INFO_PATH {
            session_info_response(
                &session_id,
                visit_count,
            )
        } else {
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
        };

    if is_new_session {
        response.with_header(
            "Set-Cookie",
            format!(
                "{}={}; Path=/; HttpOnly",
                SESSION_COOKIE_NAME,
                session_id.as_str(),
            ),
        )
    } else {
        response
    }
}

/// Looks up the session named by the request's session cookie,
/// creating a new one if the cookie is missing or the session has
/// expired. Every request bumps the session's visit counter.
///
/// Returns the session id, the visit count after this request, and
/// whether a new session was created (callers use this to decide
/// whether Set-Cookie needs to be sent back).
fn resolve_session(
    request: &HttpRequest,
    session_store: &mut SessionStore,
) -> (SessionId, u64, bool) {
    let existing_id = request
        .cookie(SESSION_COOKIE_NAME)
        .and_then(SessionId::parse);

    let found_existing = existing_id
        .as_ref()
        .is_some_and(|id| session_store.touch(id).is_some());

    let (session_id, is_new) = if found_existing {
        (existing_id.expect("checked by found_existing"), false)
    } else {
        (session_store.create().id.clone(), true)
    };

    let session = session_store
        .touch(&session_id)
        .expect("session was just created or found");

    let count = session
        .data
        .entry(VISIT_COUNT_KEY.to_string())
        .or_insert_with(|| "0".to_string());

    let next_count =
        count.parse::<u64>().unwrap_or(0) + 1;

    *count = next_count.to_string();

    (session_id, next_count, is_new)
}

fn session_info_response(
    session_id: &SessionId,
    visit_count: u64,
) -> HttpResponse {
    let body = format!(
        "{{\"session_id\":\"{}\",\"visit_count\":{}}}",
        session_id.as_str(),
        visit_count,
    );

    HttpResponse::new(
        StatusCode::Ok,
        body.into_bytes(),
    )
    .with_header(
        "Content-Type",
        "application/json",
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