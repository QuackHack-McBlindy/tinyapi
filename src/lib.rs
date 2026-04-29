//! tinyAPI – a minimal, no_std HTTP API framework for Embassy
//!
//! # Example (on ESP32‑S3)
//! ```no_run
//! use tinyapi::{register_route, Response, web_server_task, log};
//!
//! register_route("/", |_req| Response::html(include_str!("index.html"))).await;
//! register_route("/led/{state}", |req| {
//!     let state = req.param("state").unwrap_or("?");
//!     log!("LED set to {}", state);
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
mod client;
pub use client::{http_get, HttpResponse};

// Conditional logging
#[cfg(feature = "defmt")]
use defmt::info;
#[cfg(not(feature = "defmt"))]
macro_rules! info {
    ($($arg:tt)*) => {};
}



/// HTTP REQUEST DATA – LIVES ONLY AS LONG AS THE REQUEST BUFFER
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

/// HTTP RESPONSE (OWNED DATA)
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

    pub fn script(body: &str) -> Self {
        Self {
            status: "200 OK",
            content_type: "application/javascript; charset=utf-8",
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

// HANDLER TRAIT
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

// ROUTER
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

static ROUTER: Mutex<CriticalSectionRawMutex, Router> = Mutex::new(Router::new());

/// REGISTER A GET  with a path pattern (e.g., `/led/{state}`).
pub async fn register_route(pattern: &str, handler: impl Handler + 'static) {
    let mut router = ROUTER.lock().await;
    router.get(pattern, handler);
}


// LOG (test optional)
#[cfg(feature = "log")]
mod log {
    use alloc::string::String;
    use heapless::Vec;
    use critical_section::Mutex as CSMutex;
    use core::cell::RefCell;
    use crate::{Request, Response};

    const MAX_LOGS: usize = 100;

    struct LogState {
        logs: Vec<String, MAX_LOGS>,
    }

    static LOG_STATE: CSMutex<RefCell<LogState>> = CSMutex::new(RefCell::new(LogState { logs: Vec::new() }));

    /// PUSH A LOG MESSAGE (synchronous)
    pub fn push_log(msg: String) {
        critical_section::with(|cs| {
            let state = LOG_STATE.borrow(cs);
            let mut state = state.borrow_mut();
            if state.logs.len() == MAX_LOGS {
                state.logs.remove(0);
            }
            state.logs.push(msg).ok();
        });
    }

    /// GET ALL LOGS AS HTML STRINGS (synchronous)
    fn get_logs_html() -> String {
        critical_section::with(|cs| {
            let state = LOG_STATE.borrow(cs);
            let state = state.borrow();
            let mut html = String::from("<html><head><meta http-equiv='refresh' content='2'><title>Logs</title></head><body><pre>");
            for entry in state.logs.iter() {
                html.push_str(entry);
                html.push('\n');
            }
            html.push_str("</pre></body></html>");
            html
        })
    }

    // A NAMED FUNCTION THAT ACTS AS THE HANDLER - WORKS WITH ANY LIFETIME
    fn logs_handler(_req: Request<'_>) -> Response {
        let has_logs = critical_section::with(|cs| {
            let state = LOG_STATE.borrow(cs);
            let state = state.borrow();
            !state.logs.is_empty()
        });

        if !has_logs {
            let default = r#"<html><head><meta http-equiv='refresh' content='2'><title>Logs</title></head><body><pre>No logs yet. Use tinyapi::log!() to record events.</pre></body></html>"#;
            return Response::html(default);
        }

        let body = get_logs_html();
        Response::html(&body)
    }

    /// REGISTER THE `/logs` ROUTE (CALLED AUTOMATICALLY BY `web_server_task`)
    pub async fn register_logs_route() {
        crate::register_route("/logs", logs_handler).await;
    }
}
#[cfg(feature = "log")]
pub use log::push_log as _push_log;

/// LOG A MESSAGE TO THE BUILT-IN LOG BUFFER (VISIBLE AT `/logs`)
/// Example: `tinyapi::log!("Temperature: {}°C", temp).await;`
/// NOTE ASYNC
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        #[cfg(feature = "log")]
        {
            let msg = alloc::format!($($arg)*);
            let timestamp = embassy_time::Instant::now().as_millis();
            let full = alloc::format!("[{} ms] {}", timestamp, msg);
            $crate::_push_log(full);
        }
    }};
}


// HTTP SERVER TASK
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

/// THE MAIN HTTP SERVER TASK. 
/// SPAWN THIS AFTER REGESTERING ROUTES.
#[embassy_executor::task]
pub async fn web_server_task(stack: &'static Stack<'static>) -> ! {
    const PORT: u16 = 80;
    info!("📡 ☑️ 🌐 HTTP server online on port: {}", PORT);

    // REGISTER LOG ROUTE IF FEATURE ENABLED
    #[cfg(feature = "log")]
    log::register_logs_route().await;

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

        // READ HEADERS
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
