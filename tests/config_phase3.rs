use localhost::config::parse_config_str;

#[test]
fn valid_single_server_config() {
    let input = r#"
[[server]]
server_address = "127.0.0.1"
ports = [8080]
server_name = ["localhost"]
client_max_body_size = 1048576

[[server.routes]]
path = "/"
methods = ["GET"]
root = "./www"
index = "index.html"
"#;

    assert!(parse_config_str(input).is_ok());
}

#[test]
fn multiple_ports_are_valid() {
    let input = r#"
[[server]]
server_address = "127.0.0.1"
ports = [8080, 8081, 8082]
server_name = ["localhost"]

[[server.routes]]
path = "/"
methods = ["GET"]
root = "./www"
"#;

    let config =
        parse_config_str(input).unwrap();

    assert_eq!(
        config.servers[0].ports.len(),
        3
    );
}

#[test]
fn virtual_hosts_can_share_listener() {
    let input = r#"
[[server]]
server_address = "127.0.0.1"
ports = [8080]
server_name = ["one.local"]

[[server.routes]]
path = "/"
methods = ["GET"]
root = "./one"

[[server]]
server_address = "127.0.0.1"
ports = [8080]
server_name = ["two.local"]

[[server.routes]]
path = "/"
methods = ["GET"]
root = "./two"
"#;

    assert!(parse_config_str(input).is_ok());
}

#[test]
fn duplicate_port_in_server_is_rejected() {
    let input = r#"
[[server]]
server_address = "127.0.0.1"
ports = [8080, 8080]
server_name = ["localhost"]

[[server.routes]]
path = "/"
methods = ["GET"]
root = "./www"
"#;

    assert!(parse_config_str(input).is_err());
}

#[test]
fn invalid_method_is_rejected() {
    let input = r#"
[[server]]
server_address = "127.0.0.1"
ports = [8080]

[[server.routes]]
path = "/"
methods = ["GET", "POTATO"]
root = "./www"
"#;

    assert!(parse_config_str(input).is_err());
}

#[test]
fn invalid_route_is_rejected() {
    let input = r#"
[[server]]
server_address = "127.0.0.1"
ports = [8080]

[[server.routes]]
path = "wrong"
methods = ["GET"]
root = "./www"
"#;

    assert!(parse_config_str(input).is_err());
}

#[test]
fn invalid_body_limit_is_rejected() {
    let input = r#"
[[server]]
server_address = "127.0.0.1"
ports = [8080]
client_max_body_size = 0

[[server.routes]]
path = "/"
methods = ["GET"]
root = "./www"
"#;

    assert!(parse_config_str(input).is_err());
}

#[test]
fn malformed_toml_is_rejected() {
    let input = r#"
[[server
ports = [8080]
"#;

    assert!(parse_config_str(input).is_err());
}

#[test]
fn invalid_cgi_is_rejected() {
    let input = r#"
[[server]]
server_address = "127.0.0.1"
ports = [8080]

[[server.routes]]
path = "/cgi"
methods = ["GET"]
root = "./cgi"

[server.routes.cgi]
"py" = "python3"
"#;

    assert!(parse_config_str(input).is_err());
}

#[test]
fn custom_error_pages_are_valid() {
    let input = r#"
[[server]]
server_address = "127.0.0.1"
ports = [8080]

[server.error_pages]
"404" = "./errors/404.html"
"500" = "./errors/500.html"

[[server.routes]]
path = "/"
methods = ["GET"]
root = "./www"
"#;

    assert!(parse_config_str(input).is_ok());
}