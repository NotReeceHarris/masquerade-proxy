use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use flate2::read::{DeflateDecoder, GzDecoder};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use reqwest::Client;
use httparse::Request as HttpParseRequest;
use serde_json::json;
use std::io::Read;
use std::collections::HashMap;
use std::error::Error;
use serde::Deserialize;
use clap::Parser;

#[derive(Parser)]
struct Cli {
    #[clap(short = 'p', long = "port", default_value = "8080")]
    port: u16,
}

#[derive(Deserialize, Debug)]
struct ProxyResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {

    let args = Cli::parse();
    let port = args.port;

    let listener = TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    
    println!("      \x1b[1m\x1b[31m._______.\x1b[0m");
    println!("      \x1b[1m\x1b[31m| \\   / |\x1b[0m              Masquerade Proxy Client");
    println!("   .--\x1b[1m\x1b[31m|.O.|.O.|\x1b[32m______.\x1b[0m       v{}", env!("CARGO_PKG_VERSION"));
    println!("__). -\x1b[1m\x1b[31m| = | = |\x1b[32m/   \\ |\x1b[0m");
    println!(">__)  \x1b[1m\x1b[31m(.'---`.)\x1b[32mQ.|.Q.|\x1b[0m--.    127.0.0.1:{}", port); 
    println!("       \x1b[1m\x1b[31m\\\\___//\x1b[32m = | = |\x1b[0m-.(__  127.0.0.1:{}", port);
    println!("        \x1b[1m\x1b[31m`---'\x1b[32m( .---. )\x1b[0m (__<");
    println!("              \x1b[1m\x1b[32m\\\\.-.//\x1b[0m        Listening on port {}", port);
    println!("               \x1b[1m\x1b[32m`---'\x1b[0m");

    let client = Client::new();
    
    loop {
        let (mut stream, addr) = listener.accept().await?;
        println!("\n➡️  New connection from: {}", addr);
        
        let client = client.clone();
        
        tokio::spawn(async move {
            let mut buffer = [0; 4096];
            
            match stream.read(&mut buffer).await {
                Ok(n) => {
                    if n == 0 { return; }
                    
                    let mut headers = [httparse::EMPTY_HEADER; 64];
                    let mut req = HttpParseRequest::new(&mut headers);
                    
                    if let Ok(_) = req.parse(&buffer[..n]) {
                        let method = req.method.unwrap_or("GET");
                        let target_url = req.path.unwrap_or("/");
                        
                        println!("\n🌐 {} {} HTTP/1.1", method, target_url);
                        println!("Headers:");
                        
                        // Create header map once and reuse it
                        let mut header_map = HashMap::new();
                        for header in req.headers {
                            println!("   {}: {}", 
                                header.name, 
                                String::from_utf8_lossy(header.value)
                            );
                            header_map.insert(
                                header.name.to_string(),
                                String::from_utf8_lossy(header.value).to_string()
                            );
                        }
                        
                        // Handle HTTPS CONNECT requests
                        if method == "CONNECT" {
                            println!("🔒 HTTPS CONNECT request for: {}", target_url);
                            handle_connect(&mut stream, target_url).await;
                            return;
                        }

                        println!("🎯 Target URL: {}", target_url);
                        
                        // Encode parameters for proxy request
                        let encoded_url = BASE64.encode(&target_url);
                        let encoded_headers = BASE64.encode(json!(header_map).to_string());
                        
                        let body_start = find_body_start(&buffer[..n]);
                        let encoded_body = if let Some(start) = body_start {
                            let body = &buffer[start..n];
                            println!("📦 Request body ({} bytes)", body.len());
                            BASE64.encode(body)
                        } else {
                            println!("📦 No request body");
                            "".to_string()
                        };
                        
                        let proxy_url = format!(
                            "http://localhost:3030/proxy?\
                            target={}&\
                            method={}&\
                            headers={}&\
                            body={}",
                            encoded_url,
                            method,
                            encoded_headers,
                            encoded_body
                        );
                        
                        println!("📤 Forwarding to proxy server...");
                        
                        // Forward request to proxy server
                        match client.get(&proxy_url).send().await {
                            Ok(proxy_response) => {

                                let status = proxy_response.status();
                                println!("📥 Proxy response: {} {}", 
                                    status.as_u16(), 
                                    status.canonical_reason().unwrap_or("")
                                );
                                
                                let headers: String = proxy_response.headers()
                                    .iter()
                                    .map(|(name, value)| {
                                        format!(
                                            "{}: {}\r\n",
                                            name,
                                            value.to_str().unwrap_or_default()
                                        )
                                    })
                                    .collect();

                                let content_encoding = proxy_response.headers().get(reqwest::header::CONTENT_ENCODING);
                                let mut decompressed_data = Vec::new();
                            
                                // Handle different content encoding types (gzip, deflate)
                                match content_encoding.and_then(|v| v.to_str().ok()) {
                                    Some("gzip") => {
                                        let compressed_data = proxy_response.bytes().await.unwrap();
                                        let mut decoder = GzDecoder::new(&compressed_data[..]);
                                        decoder.read_to_end(&mut decompressed_data).unwrap();
                                    }
                                    Some("deflate") => {
                                        let compressed_data = proxy_response.bytes().await.unwrap();
                                        let mut decoder = DeflateDecoder::new(&compressed_data[..]);
                                        decoder.read_to_end(&mut decompressed_data).unwrap();
                                    }
                                    _ => {
                                        decompressed_data = proxy_response.bytes().await.unwrap().to_vec();
                                    }
                                }

                                let decoded = serde_json::from_slice::<ProxyResponse>(&decompressed_data).unwrap();
                                let decoded_status = http::StatusCode::from_u16(decoded.status).unwrap();
                                let decoded_headers = decoded.headers.iter().map(|(k, v)| format!("{}: {}\r\n", k, v)).collect::<String>();
                                let decoded_body = BASE64.decode(&decoded.body).unwrap();

                                let status_line = format!(
                                    "HTTP/1.1 {} {}\r\n",
                                    decoded_status.as_u16(),
                                    decoded_status.canonical_reason().unwrap_or("")
                                );
                                
                                let _ = stream.write_all(status_line.as_bytes()).await;
                                let _ = stream.write_all(decoded_headers.as_bytes()).await;
                                let _ = stream.write_all(b"\r\n").await;
                                let _ = stream.write_all(&decoded_body).await;
                                
                            }
                            Err(e) => {
                                println!("❌ Proxy request failed: {}", e);
                                let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                            }
                        }
                    }
                }
                Err(e) => eprintln!("❌ Error reading from socket: {}", e),
            }
        });
    }
}

async fn handle_connect(client_stream: &mut TcpStream, addr: &str) {
    println!("🔐 Establishing HTTPS tunnel to {}", addr);
    
    match TcpStream::connect(addr).await {
        Ok(mut server_stream) => {
            println!("✅ Connected to target server");
            let response = "HTTP/1.1 200 Connection Established\r\n\r\n";
            if client_stream.write_all(response.as_bytes()).await.is_ok() {
                println!("🔄 Starting bidirectional tunnel");
                
                match tokio::io::copy_bidirectional(client_stream, &mut server_stream).await {
                    Ok((from_client, from_server)) => {
                        println!("🔄 Tunnel closed. Bytes transferred:");
                        println!("   Client → Server: {} bytes", from_client);
                        println!("   Server → Client: {} bytes", from_server);
                    }
                    Err(e) => println!("❌ Error in HTTPS tunnel: {}", e),
                }
            }
        }
        Err(e) => {
            println!("❌ Failed to connect to target server: {}", e);
            let _ = client_stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
        }
    }
}

fn find_body_start(buffer: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < buffer.len() - 3 {
        if &buffer[i..i+4] == b"\r\n\r\n" {
            return Some(i + 4);
        }
        i += 1;
    }
    None
}

