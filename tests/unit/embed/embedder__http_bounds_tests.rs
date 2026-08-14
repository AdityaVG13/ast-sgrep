#![cfg(feature = "cloud")]

use super::*;
use std::io::{Cursor, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

fn drain_http_request(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .ok();
    let mut buf = [0u8; 8192];
    let mut collected = Vec::new();
    loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                collected.extend_from_slice(&buf[..n]);
                if collected.windows(4).any(|w| w == b"\r\n\r\n") && n < buf.len() {
                    break;
                }
            }
        }
    }
}

fn spawn_loopback(handle: impl FnOnce(TcpStream) + Send + 'static) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            handle(stream);
        }
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn write_http(stream: &mut TcpStream, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.shutdown(Shutdown::Write);
}

fn ollama_cfg(api_url: String) -> OllamaEmbeddingConfig {
    OllamaEmbeddingConfig {
        api_url,
        model: "nomic-embed-text".into(),
    }
}

#[test]
fn read_capped_embed_json_rejects_oversize_cursor() {
    let oversized = vec![b'x'; (MAX_EMBED_HTTP_BODY_BYTES as usize) + 1];
    let result: Result<serde_json::Value, _> = read_capped_embed_json(Cursor::new(oversized));
    let err = result.expect_err("oversize cursor must fail");
    assert!(
        err.contains("exceeds") && err.contains("byte cap"),
        "expected body-cap error, got {err}"
    );
}

#[test]
fn embed_via_ollama_parses_loopback_json() {
    let url = spawn_loopback(|mut stream| {
        drain_http_request(&mut stream);
        write_http(&mut stream, br#"{"embedding":[0.25,0.5,0.75]}"#);
    });
    let vector = embed_via_ollama("hello", &ollama_cfg(url)).unwrap();
    assert_eq!(vector, vec![0.25, 0.5, 0.75]);
}

#[test]
fn embed_via_ollama_rejects_hostile_oversize_loopback() {
    let url = spawn_loopback(|mut stream| {
        drain_http_request(&mut stream);
        let header =
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n";
        let _ = stream.write_all(header.as_bytes());
        let chunk = vec![b'x'; 8192];
        let mut left = (MAX_EMBED_HTTP_BODY_BYTES as usize) + 1;
        while left > 0 {
            let n = chunk.len().min(left);
            if stream.write_all(&chunk[..n]).is_err() {
                break;
            }
            left -= n;
        }
        let _ = stream.shutdown(Shutdown::Write);
    });
    let err = embed_via_ollama("hello", &ollama_cfg(url)).unwrap_err();
    assert!(
        err.contains("exceeds")
            || err.contains("reset")
            || err.contains("timed out")
            || err.contains("decode"),
        "hostile oversize body must fail closed, got {err}"
    );
}

#[test]
fn embed_via_ollama_times_out_on_hung_loopback() {
    let url = spawn_loopback(|mut stream| {
        drain_http_request(&mut stream);
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .ok();
        let mut buf = [0u8; 64];
        loop {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });
    let (tx, rx) = mpsc::channel();
    let started = Instant::now();
    thread::spawn(move || {
        let _ = tx.send(embed_via_ollama("hello", &ollama_cfg(url)));
    });
    let result = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("embed HTTP hung past the watchdog; timeout was not applied");
    assert!(result.is_err(), "hung server must not return Ok: {result:?}");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "timeout must fire before the 20s watchdog"
    );
}
