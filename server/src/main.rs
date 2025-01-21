use reqwest::{header::HeaderMap, header::HeaderName, header::HeaderValue};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use flate2::read::{DeflateDecoder, GzDecoder};
use tokio::time::{Instant, Duration, timeout};
use std::collections::HashMap;
use std::io::Read;
use warp::Filter;
use clap::Parser;
use warp::Reply;
use url::Url;

mod structs;
use structs::{Cli, ProxyRequest, ProxyResponse};

const REQUEST_TIMEOUT: u64 = 30; // Request timeout in seconds
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024; // 10MB max body size
const MAX_RETRIES: u32 = 3; // Maximum number of retries

async fn display_banner(port: u16) {
    println!("      \x1b[1m\x1b[31m._______.\x1b[0m");
    println!("      \x1b[1m\x1b[31m| \\   / |\x1b[0m              Masquerade Proxy Server");
    println!("   .--\x1b[1m\x1b[31m|.O.|.O.|\x1b[32m______.\x1b[0m       v{}", env!("CARGO_PKG_VERSION"));
    println!("__). -\x1b[1m\x1b[31m| = | = |\x1b[32m/   \\ |\x1b[0m");
    println!(">__)  \x1b[1m\x1b[31m(.'---`.)\x1b[32mQ.|.Q.|\x1b[0m--.    http://localhost:{}", port); 
    println!("       \x1b[1m\x1b[31m\\\\___//\x1b[32m = | = |\x1b[0m-.(__  https://localhost:{}", port);

    // Try to get and display public IP address if available
    if let Some(ip) = public_ip::addr().await {
        println!("        \x1b[1m\x1b[31m`---'\x1b[32m( .---. )\x1b[0m (__<  http://{}:{}", ip, port);
        println!("              \x1b[1m\x1b[32m\\\\.-.//\x1b[0m        https://{}:{}", ip, port);
    } else {
        println!("        \x1b[1m\x1b[31m`---'\x1b[32m( .---. )\x1b[0m (__<");
        println!("              \x1b[1m\x1b[32m\\\\.-.//\x1b[0m");
    }

    println!("               \x1b[1m\x1b[32m`---'\x1b[0m");
}

#[tokio::main]
async fn main() {
    // Parse command line arguments
    let args = Cli::parse();
    let port = args.port;

    // Display ASCII art banner with server information
    display_banner(port).await;

    let client = create_client(REQUEST_TIMEOUT);

    // Set up the proxy route and start the server
    let proxy = warp::path!("proxy")
        .and(warp::query::<ProxyRequest>())
        .and(warp::any().map(move || client.clone()))
        .then(handle_proxy)
        .with(warp::cors().allow_any_origin())
        .with(warp::compression::gzip());

    warp::serve(proxy).run(([127, 0, 0, 1], port)).await;
}

/// Create a configured reqwest client
fn create_client(timeout_seconds: u64) -> reqwest::Client {
    reqwest::ClientBuilder::new()
        .timeout(Duration::from_secs(timeout_seconds))
        .pool_idle_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(32)
        .tcp_keepalive(Duration::from_secs(60))
        .build()
        .expect("Failed to create HTTP client")
}

/// Main proxy request handler
async fn handle_proxy(req: ProxyRequest, client: reqwest::Client) -> impl Reply {

    // Decode and validate the target URL
    let target_url = match decode_base64(&req.target) {
        Ok(url) => {
            println!("🔄 Received proxy request: {:?} \n🎯 {}", {req.method.clone()}, url);

            if !Url::parse(&url).is_ok() {
                println!("❌ Invalid target URL: {}", url);
                return warp::reply::json(&ProxyResponse {
                    status: 400,
                    headers: HashMap::new(),
                    body: format!("Invalid target URL: {}", url),
                });
            }
            url
        },
        Err(e) => {
            println!("❌ Failed to decode target URL: {}", e);
            return warp::reply::json(&ProxyResponse {
                status: 400,
                headers: HashMap::new(),
                body: format!("Invalid target URL encoding: {}", e),
            });
        }
    };

    // Decode and parse headers from base64 JSON string
    let mut headers: HeaderMap = match decode_base64(&req.headers).and_then(|h| {
        serde_json::from_str::<HashMap<String, String>>(&h).map_err(|e| e.to_string())
    }) {
        Ok(headers) => {
            let mut header_map = HeaderMap::new();
            // Convert each header key-value pair into proper HeaderName and HeaderValue types
            for (key, value) in headers {
                if let Ok(name) = HeaderName::from_bytes(key.as_bytes()) {
                    if let Ok(value) = HeaderValue::from_str(&value) {
                        header_map.insert(name, value);
                    } else {
                        println!("Invalid header value: {}", value);
                    }
                } else {
                    println!("Invalid header name: {}", key);
                }
            }
            header_map
        }
        Err(e) => {
            println!("❌ Failed to decode headers: {}", e);
            return warp::reply::json(&ProxyResponse {
                status: 400,
                headers: HashMap::new(),
                body: format!("Invalid headers encoding: {}", e),
            });
        }
    };

    headers.remove(reqwest::header::HOST);
    headers.remove(reqwest::header::CONNECTION);
    headers.remove(reqwest::header::CACHE_CONTROL);

    headers.insert(
        reqwest::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );

    // Create HTTP client and handle request based on method
    
    let body = decode_base64(&req.body.unwrap_or_default()).unwrap_or_default();
    let start_time = Instant::now();

    let request = match req.method.as_str() {
        "GET" => client
            .get(&target_url)
            .headers(headers.clone()),
        "POST" => client
            .post(&target_url)
            .headers(headers.clone())
            .body(body),
        "PUT" => client
            .put(&target_url)
            .headers(headers.clone())
            .body(body),
        "DELETE" => client
            .delete(&target_url)
            .headers(headers.clone()),
        _ => {
            println!("❌ Unsupported method: {}", req.method);
            return warp::reply::json(&ProxyResponse {
                status: 400,
                headers: HashMap::new(),
                body: format!("Unsupported method: {}", req.method),
            });
        }
    };

    let response = match timeout(
        Duration::from_secs(REQUEST_TIMEOUT), // REQUEST_TIMEOUT should be defined as a constant
        request.send()
    ).await {
        Ok(Ok(response)) => response,  // Request completed successfully
        Ok(Err(e)) => {  // Request failed (e.g. network error)
            println!("❌ Request failed: {}", e);
            return warp::reply::json(&ProxyResponse {
                status: 500,
                headers: HashMap::new(),
                body: format!("Request failed: {}", e),
            });
        },
        Err(_) => {  // Timeout occurred
            println!("❌ Request timed out");
            return warp::reply::json(&ProxyResponse {
                status: 504,  // Gateway Timeout
                headers: HashMap::new(),
                body: "Request timed out".to_string(),
            });
        }
    };

    println!(
        "🕒 Request completed in {:.2}s",
        start_time.elapsed().as_secs_f64()
    );

    // Handle response headers and body decompression
    let mut headers = response.headers().clone();
    let content_encoding = response.headers().get(reqwest::header::CONTENT_ENCODING);
    let mut decompressed_data = Vec::new();

    // Handle different content encoding types (gzip, deflate)
    match content_encoding.and_then(|v| v.to_str().ok()) {
        Some("gzip") => {
            let compressed_data = response.bytes().await.unwrap();
            let mut decoder = GzDecoder::new(&compressed_data[..]);
            decoder.read_to_end(&mut decompressed_data).unwrap();
        }
        Some("deflate") => {
            let compressed_data = response.bytes().await.unwrap();
            let mut decoder = DeflateDecoder::new(&compressed_data[..]);
            decoder.read_to_end(&mut decompressed_data).unwrap();
        }
        _ => {
            decompressed_data = response.bytes().await.unwrap().to_vec();
        }
    }

    // Clean up response headers
    headers.remove(reqwest::header::CONTENT_ENCODING);
    headers.remove(reqwest::header::TRANSFER_ENCODING);
    headers.insert(
        reqwest::header::CONTENT_LENGTH,
        HeaderValue::from_str(&decompressed_data.clone().len().to_string()).unwrap(),
    );

    // Encode response body to base64 and return
    let base64_body = BASE64.encode(&decompressed_data);
    let headers = headers.iter().map(|(k, v)| {(k.as_str().to_string(),v.to_str().unwrap_or_default().to_string(),)}).collect();

    warp::reply::json(&ProxyResponse {
        status: 200,
        headers: headers,
        body: base64_body,
    })
}

/// Decodes a base64 string into a UTF-8 string
fn decode_base64(input: &str) -> Result<String, String> {
    BASE64
        .decode(input)
        .map_err(|e| e.to_string())
        .and_then(|bytes| String::from_utf8(bytes).map_err(|e| e.to_string()))
}