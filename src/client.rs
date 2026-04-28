// client.rs – HTTP GET/POST client for tinyapi (caller-provided buffer)
use embassy_net::{Stack, tcp::TcpSocket};
use embassy_time::Duration;

/// Response from an HTTP request. The body is a slice into the caller’s buffer.
pub struct HttpResponse<'a> {
    pub status: u16,
    pub body: &'a [u8],
}

/// Write all bytes to the socket.
async fn write_all(socket: &mut TcpSocket<'_>, buf: &[u8]) -> Result<(), embassy_net::tcp::Error> {
    let mut written = 0;
    while written < buf.len() {
        let n = socket.write(&buf[written..]).await?;
        if n == 0 {
            return Err(embassy_net::tcp::Error::ConnectionReset);
        }
        written += n;
    }
    Ok(())
}

/// Send an HTTP GET request.
/// `buf` is a scratch buffer used for both the raw response and the returned body slice.
/// It must be large enough for headers + body, otherwise the body will be truncated.
pub async fn http_get<'a>(
    stack: Stack<'a>,
    url: &str,
    buf: &'a mut [u8],
) -> Result<HttpResponse<'a>, ()> {
    let (host, port, path) = parse_url(url)?;
    let ip = parse_ip(host)?;
    let endpoint = (ip, port);

    let mut rx_buf = [0u8; 1024];   // TCP stack buffer
    let mut tx_buf = [0u8; 512];
    let mut socket = TcpSocket::new(stack, &mut rx_buf, &mut tx_buf);
    socket.set_timeout(Some(Duration::from_secs(5)));
    socket.connect(endpoint).await.map_err(|_| ())?;

    // Format and send request
    let mut req_buf = [0u8; 256];
    let req_len = format_request(&mut req_buf, "GET", host, path);
    write_all(&mut socket, &req_buf[..req_len]).await.map_err(|_| ())?;

    // Read into caller’s buffer
    let total = read_response(&mut socket, buf).await;

    parse_response(&buf[..total])
}

/// Send an HTTP POST request.
/// `body` is the payload, `content_type` is the MIME type (e.g., "application/json").
/// `buf` works exactly as in `http_get`.
pub async fn http_post<'a>(
    stack: Stack<'a>,
    url: &str,
    body: &[u8],
    content_type: &str,
    buf: &'a mut [u8],
) -> Result<HttpResponse<'a>, ()> {
    let (host, port, path) = parse_url(url)?;
    let ip = parse_ip(host)?;
    let endpoint = (ip, port);

    let mut rx_buf = [0u8; 1024];
    let mut tx_buf = [0u8; 512];
    let mut socket = TcpSocket::new(stack, &mut rx_buf, &mut tx_buf);
    socket.set_timeout(Some(Duration::from_secs(5)));
    socket.connect(endpoint).await.map_err(|_| ())?;

    // Format POST request
    let mut req_buf = [0u8; 512]; // may need a bit more space for headers + body
    let req_len = format_post_request(&mut req_buf, host, path, content_type, body);
    write_all(&mut socket, &req_buf[..req_len]).await.map_err(|_| ())?;

    let total = read_response(&mut socket, buf).await;

    parse_response(&buf[..total])
}

/// Read everything from the socket into `buf` (until close or buffer full).
async fn read_response(socket: &mut TcpSocket<'_>, buf: &mut [u8]) -> usize {
    let mut total = 0;
    loop {
        match socket.read(&mut buf[total..]).await {
            Ok(0) => break,
            Ok(n) => {
                total += n;
                if total >= buf.len() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    total
}

// ---------- helpers ----------

fn parse_url(url: &str) -> Result<(&str, u16, &str), ()> {
    let url = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = match url.find('/') {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, "/"),
    };
    let (host, port) = match host_port.find(':') {
        Some(i) => (&host_port[..i], host_port[i+1..].parse::<u16>().unwrap_or(80)),
        None => (host_port, 80),
    };
    Ok((host, port, path))
}

fn parse_ip(host: &str) -> Result<embassy_net::Ipv4Address, ()> {
    let mut parts = [0u8; 4];
    let mut idx = 0;
    for segment in host.split('.') {
        if idx >= 4 { return Err(()); }
        parts[idx] = segment.parse::<u8>().map_err(|_| ())?;
        idx += 1;
    }
    if idx != 4 { return Err(()); }
    Ok(embassy_net::Ipv4Address::new(parts[0], parts[1], parts[2], parts[3]))
}

fn format_request(buf: &mut [u8], method: &str, host: &str, path: &str) -> usize {
    let mut pos = 0;
    for &b in method.as_bytes() { if pos < buf.len() { buf[pos] = b; pos += 1; } }
    if pos < buf.len() { buf[pos] = b' '; pos += 1; }
    for &b in path.as_bytes() { if pos < buf.len() { buf[pos] = b; pos += 1; } }
    for &b in b" HTTP/1.1\r\nHost: " { if pos < buf.len() { buf[pos] = b; pos += 1; } }
    for &b in host.as_bytes() { if pos < buf.len() { buf[pos] = b; pos += 1; } }
    for &b in b"\r\nConnection: close\r\n\r\n" { if pos < buf.len() { buf[pos] = b; pos += 1; } }
    pos
}

fn format_post_request(buf: &mut [u8], host: &str, path: &str, content_type: &str, body: &[u8]) -> usize {
    let mut pos = 0;
    for &b in b"POST " { if pos < buf.len() { buf[pos] = b; pos += 1; } }
    for &b in path.as_bytes() { if pos < buf.len() { buf[pos] = b; pos += 1; } }
    for &b in b" HTTP/1.1\r\nHost: " { if pos < buf.len() { buf[pos] = b; pos += 1; } }
    for &b in host.as_bytes() { if pos < buf.len() { buf[pos] = b; pos += 1; } }
    for &b in b"\r\nContent-Type: " { if pos < buf.len() { buf[pos] = b; pos += 1; } }
    for &b in content_type.as_bytes() { if pos < buf.len() { buf[pos] = b; pos += 1; } }
    for &b in b"\r\nContent-Length: " { if pos < buf.len() { buf[pos] = b; pos += 1; } }
    // convert body length to ASCII (at most 3 digits – safe for esp32 small posts)
    let len = body.len();
    if len >= 100 { if pos < buf.len() { buf[pos] = b'0' + (len / 100) as u8; pos += 1; } }
    if len >= 10  { if pos < buf.len() { buf[pos] = b'0' + ((len / 10) % 10) as u8; pos += 1; } }
    if pos < buf.len() { buf[pos] = b'0' + (len % 10) as u8; pos += 1; }
    for &b in b"\r\n\r\n" { if pos < buf.len() { buf[pos] = b; pos += 1; } }
    for &b in body { if pos < buf.len() { buf[pos] = b; pos += 1; } }
    pos
}

fn parse_response<'a>(data: &'a [u8]) -> Result<HttpResponse<'a>, ()> {
    if data.len() < 12 { return Err(()); }
    let status_str = core::str::from_utf8(&data[9..12]).map_err(|_| ())?;
    let status = status_str.parse::<u16>().unwrap_or(0);

    // Find body start after \r\n\r\n
    let mut body_start = data.len();
    for i in 0..data.len().saturating_sub(3) {
        if &data[i..i+4] == b"\r\n\r\n" {
            body_start = i + 4;
            break;
        }
    }

    Ok(HttpResponse {
        status,
        body: &data[body_start..],
    })
}
