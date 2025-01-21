use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use clap::Parser;

#[derive(Parser)]
pub struct Cli {
    /// Port number for the proxy server (defaults to 3030)
    #[clap(short = 'p', long = "port", default_value = "3030")]
    pub port: u16,
}

/// Structure to receive and parse incoming proxy requests
#[derive(Deserialize)]
pub struct ProxyRequest {
    pub target: String,         // Base64 encoded target URL
    pub method: String,         // HTTP method (GET, POST, etc.)
    pub headers: String,        // Base64 encoded JSON string of headers
    pub body: Option<String>,   // Optional Base64 encoded request body
}

/// Structure to format and send proxy responses
#[derive(Serialize)]
pub struct ProxyResponse {
    pub status: u16,                        // HTTP status code
    pub headers: HashMap<String, String>,   // Response headers
    pub body: String,                       // Base64 encoded response body
}