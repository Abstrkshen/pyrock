// net.rs: HTTP/1.1 server + Ollama HTTP client + JSON helpers, std + serde_json.
// pyrock: dep serde_json = "1"
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static LISTENER: Mutex<Option<TcpListener>> = Mutex::new(None);
static CONNS: Mutex<Option<HashMap<i64, TcpStream>>> = Mutex::new(None);
static NEXT_ID: Mutex<i64> = Mutex::new(1);

/// Bind 127.0.0.1:port (0 = any free port) and return the bound port.
pub fn listen(port: i64) -> Result<i64, String> {
    let l = TcpListener::bind(("127.0.0.1", port as u16)).map_err(|e| e.to_string())?;
    let p = l.local_addr().map_err(|e| e.to_string())?.port();
    *LISTENER.lock().unwrap() = Some(l);
    Ok(p as i64)
}

fn read_request(stream: &TcpStream) -> Result<(String, String, String), String> {
    let mut rd = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    rd.read_line(&mut line).map_err(|e| e.to_string())?;
    let mut parts = line.split_whitespace();
    let method = parts.next().ok_or("empty request")?.to_string();
    let path = parts.next().ok_or("no path")?.to_string();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        let n = rd.read_line(&mut h).map_err(|e| e.to_string())?;
        if n == 0 || h == "\r\n" || h == "\n" { break; }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; len];
    rd.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok((method, path, String::from_utf8_lossy(&body).to_string()))
}

/// Block until a request arrives: (connection id, method, path, body).
pub fn accept() -> Result<(i64, String, String, String), String> {
    let l = {
        let g = LISTENER.lock().unwrap();
        g.as_ref().ok_or("not listening")?.try_clone().map_err(|e| e.to_string())?
    };
    loop {
        let (stream, _) = l.accept().map_err(|e| e.to_string())?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        match read_request(&stream) {
            Ok((m, p, b)) => {
                let mut idg = NEXT_ID.lock().unwrap();
                let id = *idg;
                *idg += 1;
                CONNS.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, stream);
                return Ok((id, m, p, b));
            }
            Err(_) => continue, // bad or empty connection: drop it, wait for the next one
        }
    }
}

/// Send a JSON response with Content-Length and close the connection.
pub fn respond(conn: i64, status: i64, body: &str) {
    let s = CONNS.lock().unwrap().as_mut().and_then(|m| m.remove(&conn));
    if let Some(mut s) = s {
        let reason = match status { 200 => "OK", 201 => "Created", 400 => "Bad Request", 404 => "Not Found", 409 => "Conflict", _ => "Error" };
        let head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            status, reason, body.len());
        let _ = s.write_all(head.as_bytes());
        let _ = s.write_all(body.as_bytes());
        let _ = s.flush();
    }
}

fn dechunk(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let mut j = i;
        while j + 1 < raw.len() && !(raw[j] == b'\r' && raw[j + 1] == b'\n') { j += 1; }
        let size_s = String::from_utf8_lossy(&raw[i..j]).to_string();
        let size = usize::from_str_radix(size_s.split(';').next().unwrap_or("0").trim(), 16).unwrap_or(0);
        if size == 0 { break; }
        let start = j + 2;
        let end = (start + size).min(raw.len());
        out.extend_from_slice(&raw[start..end]);
        i = end + 2;
    }
    out
}

/// POST a JSON body to an http://host:port/path URL; returns the response body.
pub fn post(url: &str, body: &str) -> Result<String, String> {
    let rest = url.strip_prefix("http://").ok_or("only http:// urls")?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let mut s = TcpStream::connect(hostport).map_err(|e| format!("connect {}: {}", hostport, e))?;
    s.set_read_timeout(Some(Duration::from_secs(600))).ok();
    let req = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        path, hostport, body.len(), body);
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("malformed response")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let payload = &raw[split + 4..];
    let status: i64 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let chunked = head.to_ascii_lowercase().contains("transfer-encoding: chunked");
    let bytes = if chunked { dechunk(payload) } else { payload.to_vec() };
    let text = String::from_utf8_lossy(&bytes).to_string();
    if status != 200 { return Err(format!("HTTP {}: {}", status, text)); }
    Ok(text)
}

/// JSON Pointer lookup: strings come back unquoted, everything else as raw JSON text.
pub fn json_get(doc: &str, pointer: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(doc).ok()?;
    match v.pointer(pointer)? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// Length of the array at the pointer (0 when missing or not an array).
pub fn json_len(doc: &str, pointer: &str) -> i64 {
    serde_json::from_str::<serde_json::Value>(doc).ok()
        .and_then(|v| v.pointer(pointer).and_then(|a| a.as_array().map(|a| a.len() as i64)))
        .unwrap_or(0)
}

/// Escape a string as a JSON string literal (with quotes).
pub fn json_quote(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

pub fn args() -> Vec<String> { std::env::args().collect() }

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Like json_get but never None: missing/null gives "" (keeps Python code free of Option narrowing).
pub fn json_str(doc: &str, pointer: &str) -> String {
    json_get(doc, pointer).unwrap_or_default()
}

/// True when the pointer resolves to a non-null value.
pub fn json_has(doc: &str, pointer: &str) -> bool {
    json_get(doc, pointer).is_some()
}
