//! TinyAPI – a minimal, no_std HTTP API framework for Embassy.
//!
//! # Example (on ESP32‑S3)
//! ```no_run
//! use tinyapi::{register_route, Response, web_server_task};
//!
//! register_route("/", |_req| Response::html(include_str!("index.html"))).await;
//! register_route("/led/{state}", |req| {
//!     let state = req.param("state").unwrap_or("?");
//!     Response::text(&format!("LED set to {}", state))
//! }).await;
//!
//! spawner.spawn(web_server_task(stack)).unwrap();
//! ```

#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use embassy_net::{Stack, tcp::TcpSocket};
use embassy_time::{Duration, Timer};
use embassy_futures::select::select;
use embassy_sync::mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use heapless::FnvIndexMap;

// Conditional logging
#[cfg(feature = "defmt")]
use defmt::info;
#[cfg(not(feature = "defmt"))]
macro_rules! info {
    ($($arg:tt)*) => {};
}

// -----------------------------------------------------------------------------
// Public API
// -----------------------------------------------------------------------------

/// HTTP request data – lives only as long as the request buffer.
pub struct Request<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub params: FnvIndexMap<&'a str, &'a str, 8>,
}

impl<'a> Request<'a> {
    pub fn param(&self, name: &str) -> Option<&'a str> {
        self.params.get(name).copied()
    }
}

/// HTTP response (owned data).
pub struct Response {
    pub status: &'static str,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(body: &str) -> Self {
        Self {
            status: "200 OK",
            content_type: "text/plain",
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn html(body: &str) -> Self {
        Self {
            status: "200 OK",
            content_type: "text/html; charset=utf-8",
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn not_found() -> Self {
        Self {
            status: "404 Not Found",
            content_type: "text/html",
            body: b"<h1>404 Not Found</h1>".to_vec(),
        }
    }
}

// Handler trait – implemented for closures.
pub trait Handler: Send + Sync {
    fn call(&self, req: Request) -> Response;
}

impl<F> Handler for F
where
    F: Fn(Request) -> Response + Send + Sync,
{
    fn call(&self, req: Request) -> Response {
        self(req)
    }
}

// Router
struct Router {
    routes: Vec<(String, Box<dyn Handler>)>,
}

impl Router {
    const fn new() -> Self {
        Self { routes: Vec::new() }
    }

    fn get(&mut self, pattern: &str, handler: impl Handler + 'static) {
        self.routes.push((pattern.to_string(), Box::new(handler)));
    }

    fn route(&self, method: &str, path: &str) -> Option<Response> {
        if method != "GET" {
            return None;
        }
        for (pattern, handler) in &self.routes {
            if let Some(params) = match_route(path, pattern) {
                let req = Request { method, path, params };
                return Some(handler.call(req));
            }
        }
        None
    }
}

fn match_route<'a>(path: &'a str, pattern: &'a str) -> Option<FnvIndexMap<&'a str, &'a str, 8>> {
    let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let pattern_segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();

    if path_segments.len() != pattern_segments.len() {
        return None;
    }

    let mut params = FnvIndexMap::new();
    for (p_seg, pat_seg) in path_segments.iter().zip(pattern_segments.iter()) {
        if pat_seg.starts_with('{') && pat_seg.ends_with('}') {
            let param_name = &pat_seg[1..pat_seg.len() - 1];
            let _ = params.insert(param_name, *p_seg);
        } else if pat_seg != p_seg {
            return None;
        }
    }
    Some(params)
}

// Global router
static ROUTER: Mutex<CriticalSectionRawMutex, Router> = Mutex::new(Router::new());

/// Register a GET route with a path pattern (e.g., `/led/{state}`).
/// Must be called before spawning `web_server_task`.
pub async fn register_route(pattern: &str, handler: impl Handler + 'static) {
    let mut router = ROUTER.lock().await;
    router.get(pattern, handler);
}

// -----------------------------------------------------------------------------
// HTTP Server Task
// -----------------------------------------------------------------------------

async fn write_all(socket: &mut TcpSocket<'_>, mut buf: &[u8]) -> Result<(), embassy_net::tcp::Error> {
    while !buf.is_empty() {
        let n = socket.write(buf).await?;
        buf = &buf[n..];
    }
    Ok(())
}

async fn write_content_length(socket: &mut TcpSocket<'_>, len: usize) -> Result<(), embassy_net::tcp::Error> {
    let mut buf = [0u8; 10];
    let mut idx = buf.len();
    let mut n = len;
    if n == 0 {
        buf[idx - 1] = b'0';
        idx -= 1;
    } else {
        while n > 0 {
            idx -= 1;
            buf[idx] = b'0' + (n % 10) as u8;
            n /= 10;
        }
    }
    write_all(socket, &buf[idx..]).await
}

async fn send_response(socket: &mut TcpSocket<'_>, resp: Response) -> Result<(), embassy_net::tcp::Error> {
    write_all(socket, b"HTTP/1.1 ").await?;
    write_all(socket, resp.status.as_bytes()).await?;
    write_all(socket, b"\r\nContent-Type: ").await?;
    write_all(socket, resp.content_type.as_bytes()).await?;
    write_all(socket, b"\r\nContent-Length: ").await?;
    write_content_length(socket, resp.body.len()).await?;
    write_all(socket, b"\r\nConnection: close\r\n\r\n").await?;
    write_all(socket, &resp.body).await?;
    socket.flush().await?;
    Ok(())
}

/// The main HTTP server task. Spawn this after registering routes.
#[embassy_executor::task]
pub async fn web_server_task(stack: &'static Stack<'static>) -> ! {
    const PORT: u16 = 80;
    info!("HTTP server starting on port {}", PORT);

    loop {
        let mut rx_buffer = [0; 1024];
        let mut tx_buffer = [0; 1024];
        let mut socket = TcpSocket::new(*stack, &mut rx_buffer, &mut tx_buffer);

        let accept_fut = socket.accept(PORT);
        let timeout_fut = Timer::after(Duration::from_secs(60));
        match select(accept_fut, timeout_fut).await {
            embassy_futures::select::Either::First(Ok(())) => {
                if let Some(addr) = socket.remote_endpoint() {
                    info!("Connection accepted from {}", addr);
                } else {
                    info!("Connection accepted (unknown remote)");
                }
            }
            embassy_futures::select::Either::First(Err(e)) => {
                info!("Accept error: {:?}", e);
                Timer::after(Duration::from_millis(100)).await;
                continue;
            }
            embassy_futures::select::Either::Second(_) => continue,
        }

        // Read headers
        let mut request_buf = [0; 512];
        let mut total_read = 0;
        let mut found_end = false;

        let read_timeout = Timer::after(Duration::from_secs(2));
        let read_fut = async {
            while total_read < request_buf.len() {
                match socket.read(&mut request_buf[total_read..]).await {
                    Ok(n) if n == 0 => break,
                    Ok(n) => {
                        total_read += n;
                        if total_read >= 4 && &request_buf[total_read - 4..total_read] == b"\r\n\r\n" {
                            found_end = true;
                            break;
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok::<_, embassy_net::tcp::Error>(())
        };

        match select(read_fut, read_timeout).await {
            embassy_futures::select::Either::First(Ok(())) => {
                if !found_end {
                    info!("Request header too large or incomplete, closing");
                    continue;
                }
            }
            embassy_futures::select::Either::First(Err(e)) => {
                info!("Read error: {:?}", e);
                continue;
            }
            embassy_futures::select::Either::Second(_) => {
                info!("Request read timeout");
                continue;
            }
        }

        let request_str = core::str::from_utf8(&request_buf[..total_read]).unwrap_or("");
        let mut lines = request_str.lines();
        let first_line = lines.next().unwrap_or("");
        let mut parts = first_line.split_whitespace();
        let method = parts.next().unwrap_or("GET");
        let path = parts.next().unwrap_or("/");

        info!("Request for {} {}", method, path);

        let response = ROUTER.lock().await.route(method, path).unwrap_or_else(|| {
            info!("No route found");
            Response::not_found()
        });

        if let Err(e) = send_response(&mut socket, response).await {
            info!("Send error: {:?}", e);
        } else {
            info!("Response sent");
        }

        Timer::after(Duration::from_millis(50)).await;
    }
}
