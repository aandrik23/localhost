use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(rename = "server")]
    pub servers: Vec<ServerConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_server_address")]
    pub server_address: String,

    pub ports: Vec<u16>,

    #[serde(default, alias = "server_names")]
    pub server_name: Vec<String>,

    #[serde(default)]
    pub error_pages: HashMap<String, String>,

    #[serde(default = "default_body_size")]
    pub client_max_body_size: usize,

    #[serde(default)]
    pub routes: Vec<RouteConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouteConfig {
    pub path: String,

    #[serde(default)]
    pub methods: Vec<String>,

    pub root: Option<String>,

    pub index: Option<String>,

    #[serde(default)]
    pub directory_listing: bool,

    pub redirect: Option<String>,

    pub redirect_status: Option<u16>,

    #[serde(default)]
    pub cgi: HashMap<String, String>,
}

fn default_server_address() -> String {
    "0.0.0.0".to_string()
}

fn default_body_size() -> usize {
    10 * 1024 * 1024
}