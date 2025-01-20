use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use flate2::read::{DeflateDecoder, GzDecoder};
use reqwest::{header::HeaderMap, header::HeaderName, header::HeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use warp::Reply;
use warp::Filter;
use clap::Parser;

#[derive(Parser)]
struct Cli {
    #[clap(short = 'p', long = "port", default_value = "3030")]
    port: u16,
}

// Structure to receive query parameters
#[derive(Deserialize)]
struct ProxyRequest {
    target: String,
    method: String,
    headers: String,
    body: Option<String>,
}

// Structure to send response back
#[derive(Serialize)]
struct ProxyResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
}

#[tokio::main]
async fn main() {

    let args = Cli::parse();

    let port = args.port;

    /* if let Some(ip) = public_ip::addr().await {
        
        println!("        \x1b[1m\x1b[31m`---'\x1b[32m( .---. )\x1b[0m (__<  https://{:?}:{}", ip, port);
    } else {
        println!("       \x1b[1m\x1b[31m\\\\___//\x1b[32m = | = |\x1b[0m-.(__");
        println!("        \x1b[1m\x1b[31m`---'\x1b[32m( .---. )\x1b[0m (__<");
    } */


    println!("      \x1b[1m\x1b[31m._______.\x1b[0m");
    println!("      \x1b[1m\x1b[31m| \\   / |\x1b[0m              Masquerade Proxy Server");
    println!("   .--\x1b[1m\x1b[31m|.O.|.O.|\x1b[32m______.\x1b[0m       v{}", env!("CARGO_PKG_VERSION"));
    println!("__). -\x1b[1m\x1b[31m| = | = |\x1b[32m/   \\ |\x1b[0m");
    println!(">__)  \x1b[1m\x1b[31m(.'---`.)\x1b[32mQ.|.Q.|\x1b[0m--.    http://localhost:{}", port); 
    println!("       \x1b[1m\x1b[31m\\\\___//\x1b[32m = | = |\x1b[0m-.(__  https://localhost:{}", port);

    if let Some(ip) = public_ip::addr().await {
        println!("        \x1b[1m\x1b[31m`---'\x1b[32m( .---. )\x1b[0m (__<  http://{}:{}", ip, port);
        println!("              \x1b[1m\x1b[32m\\\\.-.//\x1b[0m        https://{}:{}", ip, port);
    } else {
        println!("        \x1b[1m\x1b[31m`---'\x1b[32m( .---. )\x1b[0m (__<");
        println!("              \x1b[1m\x1b[32m\\\\.-.//\x1b[0m");
    }
    
    println!("               \x1b[1m\x1b[32m`---'\x1b[0m");

    // Create the proxy route
    let proxy = warp::path!("proxy")
        .and(warp::query::<ProxyRequest>())
        .then(handle_proxy);

    warp::serve(proxy).run(([127, 0, 0, 1], port)).await;
}

async fn handle_proxy(req: ProxyRequest) -> impl Reply {
    println!("\n🔄 Received proxy request: {:?}", {req.method.clone()});

    // Decode target URL
    let target_url = match decode_base64(&req.target) {
        Ok(url) => url,
        Err(e) => {
            println!("❌ Failed to decode target URL: {}", e);
            return warp::reply::json(&ProxyResponse {
                status: 400,
                headers: HashMap::new(),
                body: format!("Invalid target URL encoding: {}", e),
            });
        }
    };

    let headers: HeaderMap = match decode_base64(&req.headers).and_then(|h| {
        serde_json::from_str::<HashMap<String, String>>(&h).map_err(|e| e.to_string())
    }) {
        Ok(headers) => {
            let mut header_map = HeaderMap::new();
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

    // Use the client with headers to make the request
    let client = reqwest::Client::builder().build().unwrap();

    let resp = match req.method.as_str() {
        "GET" => client
            .get(&target_url)
            .headers(headers.clone())
            .send()
            .await
            .unwrap(),
        "POST" => {
            let body = decode_base64(&req.body.unwrap_or_default()).unwrap_or_default();
            client
                .post(&target_url)
                .headers(headers.clone())
                .body(body)
                .send()
                .await
                .unwrap()
        }
        "PUT" => {
            let body = req.body.unwrap_or_default();
            client
                .put(&target_url)
                .headers(headers.clone())
                .body(body)
                .send()
                .await
                .unwrap()
        }
        "DELETE" => client
            .delete(&target_url)
            .headers(headers.clone())
            .send()
            .await
            .unwrap(),
        _ => {
            println!("❌ Unsupported method: {}", req.method);
            return warp::reply::json(&ProxyResponse {
                status: 400,
                headers: HashMap::new(),
                body: format!("Unsupported method: {}", req.method),
            });
        }
    };

    let content_encoding = resp.headers().get(reqwest::header::CONTENT_ENCODING);

    let mut decompressed_data = Vec::new();

    match content_encoding.and_then(|v| v.to_str().ok()) {
        Some("gzip") => {
            // The response is gzip-compressed, so decode it accordingly
            let compressed_data = resp.bytes().await.unwrap();
            let mut decoder = GzDecoder::new(&compressed_data[..]);
            decoder.read_to_end(&mut decompressed_data).unwrap();
        }
        Some("deflate") => {
            // The response is deflate-compressed, so decode it accordingly
            let compressed_data = resp.bytes().await.unwrap();
            let mut decoder = DeflateDecoder::new(&compressed_data[..]);
            decoder.read_to_end(&mut decompressed_data).unwrap();
        }
        _ => {
            decompressed_data = resp.bytes().await.unwrap().to_vec();
        }
    }

    // Encode body to base64
    let base64_body = BASE64.encode(&decompressed_data);

    warp::reply::json(&ProxyResponse {
        status: 200, // Use the stored status
        headers: headers
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    v.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect(),
        body: base64_body,
    })
}

fn decode_base64(input: &str) -> Result<String, String> {
    BASE64
        .decode(input)
        .map_err(|e| e.to_string())
        .and_then(|bytes| String::from_utf8(bytes).map_err(|e| e.to_string()))
}
