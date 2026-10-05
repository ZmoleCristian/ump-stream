//! Canned HTTP/1.1 chunked server + fixture loaders shared by the integration tests.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use ump_stream::Part;

pub struct Hit {
    pub path: String,
    pub body: Vec<u8>,
}

/// Serves one canned body per connection (7-byte chunks so parts straddle
/// chunk boundaries), records each request, closes the connection.
pub fn spawn(l: TcpListener, bodies: Vec<Vec<u8>>) -> Arc<Mutex<Vec<Hit>>> {
    let hits = Arc::new(Mutex::new(Vec::new()));
    let h = hits.clone();
    tokio::spawn(async move {
        for body in bodies {
            let (mut s, _) = l.accept().await.unwrap();
            let mut req = Vec::new();
            let mut tmp = [0u8; 4096];
            let head_end = loop {
                let n = s.read(&mut tmp).await.unwrap();
                assert!(n > 0, "client hung up");
                req.extend_from_slice(&tmp[..n]);
                if let Some(i) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let head = String::from_utf8_lossy(&req[..head_end]).to_ascii_lowercase();
            let path = head.split_whitespace().nth(1).unwrap().to_owned();
            let len: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .map_or(0, |v| v.trim().parse().unwrap());
            while req.len() < head_end + len {
                let n = s.read(&mut tmp).await.unwrap();
                req.extend_from_slice(&tmp[..n]);
            }
            h.lock().unwrap().push(Hit { path, body: req[head_end..head_end + len].to_vec() });

            let mut out = b"HTTP/1.1 200 OK\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
            for c in body.chunks(7) {
                out.extend(format!("{:x}\r\n", c.len()).into_bytes());
                out.extend(c);
                out.extend(b"\r\n");
            }
            out.extend(b"0\r\n\r\n");
            s.write_all(&out).await.unwrap();
            s.shutdown().await.unwrap();
        }
    });
    hits
}

pub fn concat(parts: &[Part]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in parts {
        p.encode_into(&mut out).unwrap();
    }
    out
}

/// `tests/fixtures/real/<prefix>_*.bin`, sorted, as (file name, bytes).
/// The captures are not published (they carry session tokens and the client
/// IP); without the directory this returns empty and the caller skips.
pub fn real(prefix: &str) -> Vec<(String, Vec<u8>)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/real");
    if !dir.is_dir() {
        eprintln!("skipped: no captures in {}", dir.display());
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_str().unwrap().starts_with(prefix))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no {prefix}_* in {}", dir.display());
    files
        .into_iter()
        .map(|p| (p.file_name().unwrap().to_str().unwrap().to_owned(), fs::read(&p).unwrap()))
        .collect()
}
