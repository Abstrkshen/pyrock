//! Tiny HTTP server using only the Rust standard library.
//! pyrock exposes each `pub fn` to Python. `std::thread` keeps the listener
//! alive after `start` returns.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

pub fn start(port: i64) -> i64 {
    let bind_port: u16 = if port < 0 || port > 65535 {
        0
    } else {
        port as u16
    };
    let listener = match TcpListener::bind(("127.0.0.1", bind_port)) {
        Ok(listener) => listener,
        Err(_) => return -1,
    };
    let actual = match listener.local_addr() {
        Ok(addr) => addr.port() as i64,
        Err(_) => return -1,
    };
    thread::spawn(move || {
        for conn in listener.incoming() {
            if let Ok(stream) = conn {
                let _ = respond(stream);
            }
        }
    });
    actual
}

fn respond(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut buf = [0u8; 2048];
    let n = stream.read(&mut buf)?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = request_path(&req);
    let body = format!("pyrock rust http server\npath: {path}\n");
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    Ok(())
}

fn request_path(req: &str) -> &str {
    let line = req.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let _method = parts.next();
    parts.next().unwrap_or("/")
}
