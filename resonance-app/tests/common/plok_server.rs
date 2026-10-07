//! A local stand-in for plok.org and the upstream hosts its index points
//! at (vocal-mcp-control-and-quality.md §13, install tests): one loopback
//! HTTP server serving an `index.json` and zipped fixture banks and
//! vocoders, honouring `Range` + `If-Range` so a resume can be seen, and
//! able to cut a body short. Nothing here touches the network or the
//! user's folders.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

/// One request the server saw.
#[derive(Debug, Clone)]
pub struct Seen {
    pub path: String,
    pub range: Option<String>,
    pub status: u16,
}

/// How the server answers the next request for a path.
#[derive(Debug, Clone, Copy)]
pub enum Act {
    /// Close the connection after this many body bytes.
    CutAfter(usize),
}

/// Path → body.
type Files = Vec<(String, Vec<u8>)>;
/// Path → what to do for its next requests.
type Script = Vec<(String, VecDeque<Act>)>;

#[derive(Clone, Default)]
pub struct PlokServer {
    files: Arc<Mutex<Files>>,
    script: Arc<Mutex<Script>>,
    log: Arc<Mutex<Vec<Seen>>>,
    base: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn etag_of(body: &[u8]) -> String {
    format!("\"{}\"", &sha256_hex(body)[..16])
}

impl PlokServer {
    /// Start serving; `base()` is `http://127.0.0.1:<port>`.
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().unwrap();
        let server = PlokServer {
            base: format!("http://{addr}"),
            ..PlokServer::default()
        };
        let s = server.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let s = s.clone();
                std::thread::spawn(move || s.serve(stream));
            }
        });
        server
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// Serve `body` at `path` (replacing what was there).
    pub fn put(&self, path: &str, body: Vec<u8>) {
        let mut files = self.files.lock().unwrap();
        files.retain(|(p, _)| p != path);
        files.push((path.to_string(), body));
    }

    /// Answer the next requests for `path` with `acts`, in order.
    pub fn script(&self, path: &str, acts: Vec<Act>) {
        self.script
            .lock()
            .unwrap()
            .push((path.to_string(), acts.into()));
    }

    pub fn requests(&self, path: &str) -> Vec<Seen> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.path == path)
            .cloned()
            .collect()
    }

    fn next_act(&self, path: &str) -> Option<Act> {
        let mut script = self.script.lock().unwrap();
        script
            .iter_mut()
            .find(|(p, acts)| p == path && !acts.is_empty())
            .and_then(|(_, acts)| acts.pop_front())
    }

    fn serve(&self, mut stream: TcpStream) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return;
        }
        let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
        let (mut range, mut if_range) = (None, None);
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                match k.trim().to_ascii_lowercase().as_str() {
                    "range" => range = Some(v.trim().to_string()),
                    "if-range" => if_range = Some(v.trim().to_string()),
                    _ => {}
                }
            }
        }
        let body = self
            .files
            .lock()
            .unwrap()
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, b)| b.clone());
        let status = match body {
            None => {
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                404
            }
            Some(body) => {
                let act = self.next_act(&path);
                serve_file(&mut stream, range.as_deref(), if_range.as_deref(), &body, act)
            }
        };
        let _ = stream.shutdown(Shutdown::Both);
        self.log.lock().unwrap().push(Seen {
            path,
            range,
            status,
        });
    }
}

fn serve_file(
    stream: &mut TcpStream,
    range: Option<&str>,
    if_range: Option<&str>,
    body: &[u8],
    act: Option<Act>,
) -> u16 {
    let etag = etag_of(body);
    let validator_ok = if_range.is_none_or(|v| v == etag);
    let start = range
        .filter(|_| validator_ok)
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.strip_suffix('-'))
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|s| *s < body.len());
    let (status, head, from) = match start {
        Some(s) => (
            206,
            format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nETag: {etag}\r\n\
                 Content-Range: bytes {s}-{}/{}\r\nConnection: close\r\n\r\n",
                body.len() - s,
                body.len() - 1,
                body.len()
            ),
            s,
        ),
        None => (
            200,
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: {etag}\r\n\
                 Connection: close\r\n\r\n",
                body.len()
            ),
            0,
        ),
    };
    let _ = stream.write_all(head.as_bytes());
    let rest = &body[from..];
    let rest = match act {
        Some(Act::CutAfter(n)) => &rest[..n.min(rest.len())],
        None => rest,
    };
    let _ = stream.write_all(rest);
    let _ = stream.flush();
    status
}

/// An in-memory zip of `dir`'s files, each under `prefix` (a wrapper
/// directory, or "" for none). Stored uncompressed.
pub fn zip_dir(dir: &Path, prefix: &str) -> Vec<u8> {
    let mut files = Vec::new();
    collect(dir, dir, &mut files);
    files.sort();
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut out);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for rel in files {
            let name = if prefix.is_empty() {
                rel.clone()
            } else {
                format!("{prefix}/{rel}")
            };
            zip.start_file(name.as_str(), opts).unwrap();
            zip.write_all(&std::fs::read(dir.join(&rel)).unwrap())
                .unwrap();
        }
        // macOS litter, which an install must skip.
        zip.start_file("__MACOSX/._dsconfig.yaml", opts).unwrap();
        zip.write_all(b"junk").unwrap();
        zip.finish().unwrap();
    }
    out.into_inner()
}

fn collect(base: &Path, dir: &Path, out: &mut Vec<String>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(base, &p, out);
        } else {
            out.push(
                p.strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

/// A fixture vocoder package: `vocoder.yaml` naming `name` plus an empty
/// model, zipped under a wrapper directory.
pub fn vocoder_zip(name: &str) -> Vec<u8> {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("vocoder.yaml"),
        format!("name: {name}\nmodel: model.onnx\n"),
    )
    .unwrap();
    std::fs::write(tmp.path().join("model.onnx"), b"").unwrap();
    zip_dir(tmp.path(), "vocoder_pkg")
}
