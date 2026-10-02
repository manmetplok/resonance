//! The process-wide plok.org download worker against a local HTTP server
//! (drums-plugin-rework.md §4.1, §9).
//!
//! Every test builds its own kit library at a temp root
//! (`SharedKitLibrary::open`) whose worker fetches from an in-test
//! `TcpListener` server, so the real data dir and the real server are
//! never touched and the tests run side by side.
#![cfg(feature = "editor")]

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use resonance_common::drumkit_library::{read_sidecar, Source, SOURCE_PLOK};
use resonance_drums::download::{
    self, Command, ExtractHook, FreeSpaceFn, KeepAliveFn, ServerKit, SpawnHook, State, Status,
    WorkerConfig,
};
use resonance_drums::library::{Roots, SharedKitLibrary};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A kit manifest with `pieces` pieces, one mic setup, one take each.
fn manifest(name: &str, pieces: usize) -> String {
    let mut obj = serde_json::Map::new();
    for p in 0..pieces {
        obj.insert(
            format!("Piece {p}"),
            serde_json::json!({
                "01_KickIn_e901": {
                    "brand": "Sennheiser", "channel": "01", "mic": "e901",
                    "position": "KickIn",
                    "rounds": { "RR01": { "Vel01": format!("piece{p}.wav") } }
                }
            }),
        );
    }
    obj.insert("_meta".into(), serde_json::json!({ "name": name }));
    serde_json::to_string_pretty(&obj).unwrap()
}

/// A zip of a kit: `<dir>/drum_samples.json` plus `files` sample files.
fn kit_zip(name: &str, dir: &str, files: usize) -> Vec<u8> {
    let mut entries = vec![(
        format!("{dir}/drum_samples.json"),
        manifest(name, files).into_bytes(),
    )];
    for p in 0..files {
        entries.push((format!("{dir}/piece{p}.wav"), vec![p as u8; 3000]));
    }
    zip_of(&entries)
}

/// An in-memory zip of `entries`, stored uncompressed.
fn zip_of(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut out);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            zip.start_file(name.as_str(), opts).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }
    out.into_inner()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// One request the server saw.
#[derive(Debug, Clone)]
struct Seen {
    path: String,
    range: Option<String>,
    if_range: Option<String>,
    accept_encoding: Option<String>,
    status: u16,
    /// Body bytes the server sent back.
    sent: usize,
}

/// How the server answers the next request for a path.
#[derive(Debug, Clone)]
enum Act {
    /// Serve the file, honouring `Range` and `If-Range`.
    Normal,
    /// Answer this status with no body.
    Status(u16),
    /// Start a normal answer but close the connection after this many
    /// body bytes.
    CutAfter(usize),
}

/// Path → body.
type Files = Vec<(String, Vec<u8>)>;

#[derive(Clone, Default)]
struct Server {
    /// Path → zip bytes; served honouring `Range` unless the path starts
    /// with `/norange`. Each has an `ETag` derived from its bytes.
    files: Arc<Mutex<Files>>,
    /// Path → what to do for its next requests (then `Normal`).
    script: Arc<Mutex<Vec<(String, VecDeque<Act>)>>>,
    log: Arc<Mutex<Vec<Seen>>>,
}

fn etag_of(body: &[u8]) -> String {
    format!("\"{}\"", &sha256_hex(body)[..16])
}

impl Server {
    fn start(files: Vec<(&str, Vec<u8>)>) -> (Self, String) {
        let server = Server {
            files: Arc::new(Mutex::new(
                files.into_iter().map(|(p, b)| (p.to_string(), b)).collect(),
            )),
            ..Server::default()
        };
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().unwrap();
        let s = server.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let s = s.clone();
                std::thread::spawn(move || s.serve(stream));
            }
        });
        (server, format!("http://{addr}/index.json"))
    }

    /// Answer the next requests for `path` with `acts`, in order.
    fn script(&self, path: &str, acts: Vec<Act>) {
        self.script.lock().push((path.to_string(), acts.into()));
    }

    /// Serve `body` at `path` from now on (a new version of the file).
    fn replace(&self, path: &str, body: Vec<u8>) {
        let mut files = self.files.lock();
        files.retain(|(p, _)| p != path);
        files.push((path.to_string(), body));
    }

    fn requests(&self, path: &str) -> Vec<Seen> {
        self.log
            .lock()
            .iter()
            .filter(|s| s.path == path)
            .cloned()
            .collect()
    }

    fn next_act(&self, path: &str) -> Act {
        let mut script = self.script.lock();
        script
            .iter_mut()
            .find(|(p, acts)| p == path && !acts.is_empty())
            .and_then(|(_, acts)| acts.pop_front())
            .unwrap_or(Act::Normal)
    }

    fn serve(&self, mut stream: TcpStream) {
        let Some(req) = read_request(&stream) else {
            return;
        };
        let mut seen = Seen {
            path: req.path.clone(),
            range: req.range.clone(),
            if_range: req.if_range.clone(),
            accept_encoding: req.accept_encoding.clone(),
            status: 200,
            sent: 0,
        };
        match req.path.as_str() {
            // Trickles a huge body until the client goes away.
            "/slow.zip" => {
                let head = "HTTP/1.1 200 OK\r\nContent-Length: 1073741824\r\nETag: \"slow\"\r\n\
                            Content-Type: application/zip\r\n\r\n";
                if stream.write_all(head.as_bytes()).is_ok() {
                    let chunk = vec![0u8; 64 * 1024];
                    while stream.write_all(&chunk).is_ok() {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
            }
            // Headers and a first chunk, then silence with the connection
            // held open.
            "/stall.zip" => {
                let head = "HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\nETag: \"stall\"\r\n\r\n";
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&[0u8; 4096]);
                let _ = stream.flush();
                std::thread::sleep(Duration::from_secs(30));
            }
            path => {
                let body = self
                    .files
                    .lock()
                    .iter()
                    .find(|(p, _)| p == path)
                    .map(|(_, b)| b.clone());
                let act = self.next_act(path);
                match (body, act) {
                    (_, Act::Status(code)) => {
                        seen.status = code;
                        let _ = stream.write_all(
                            format!(
                                "HTTP/1.1 {code} Nope\r\nContent-Length: 0\r\n\
                                 Connection: close\r\n\r\n"
                            )
                            .as_bytes(),
                        );
                    }
                    (None, _) => {
                        seen.status = 404;
                        let _ = stream.write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                    }
                    (Some(body), act) => {
                        let (status, sent) = serve_file(&mut stream, path, &req, &body, act);
                        seen.status = status;
                        seen.sent = sent;
                    }
                }
            }
        }
        let _ = stream.shutdown(Shutdown::Both);
        self.log.lock().push(seen);
    }
}

/// A file answer: 206 for a range the `If-Range` allows, 416 past its end,
/// else 200 with the whole file. Returns the status and the bytes sent.
fn serve_file(
    stream: &mut TcpStream,
    path: &str,
    req: &Request,
    body: &[u8],
    act: Act,
) -> (u16, usize) {
    let etag = etag_of(body);
    let validator_ok = req.if_range.as_deref().is_none_or(|v| v == etag);
    let start = req
        .range
        .as_deref()
        .filter(|_| !path.starts_with("/norange") && validator_ok)
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.strip_suffix('-'))
        .and_then(|n| n.parse::<usize>().ok());
    let (status, head, from) = match start {
        Some(s) if s < body.len() => (
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
        Some(_) => {
            let head = format!(
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nETag: {etag}\r\n\
                 Content-Range: bytes */{}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            return (416, 0);
        }
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
    let n = match act {
        Act::CutAfter(n) => n.min(rest.len()),
        _ => rest.len(),
    };
    let _ = stream.write_all(&rest[..n]);
    let _ = stream.flush();
    (status, n)
}

struct Request {
    path: String,
    range: Option<String>,
    if_range: Option<String>,
    accept_encoding: Option<String>,
}

/// Read the request line and headers.
fn read_request(stream: &TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let path = line.split_whitespace().nth(1)?.to_string();
    let mut req = Request {
        path,
        range: None,
        if_range: None,
        accept_encoding: None,
    };
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 || header == "\r\n" {
            break;
        }
        if let Some((k, v)) = header.split_once(':') {
            let v = Some(v.trim().to_string());
            match k.trim().to_ascii_lowercase().as_str() {
                "range" => req.range = v,
                "if-range" => req.if_range = v,
                "accept-encoding" => req.accept_encoding = v,
                _ => {}
            }
        }
    }
    Some(req)
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A temp base with a library root and a marks dir, removed on drop.
struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "resonance-drums-download-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn root(&self) -> PathBuf {
        self.0.join("drumkits")
    }

    fn library(&self, worker: WorkerConfig) -> Arc<SharedKitLibrary> {
        self.library_marked(worker, "library")
    }

    /// A library with its own marks dir (a second library at the root).
    fn library_marked(&self, worker: WorkerConfig, marks: &str) -> Arc<SharedKitLibrary> {
        SharedKitLibrary::open(Roots {
            root: Some(self.root()),
            marks_dir: Some(self.0.join(marks)),
            installed_json: None,
            worker,
        })
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn config(index_url: &str) -> WorkerConfig {
    WorkerConfig {
        index_url: index_url.to_string(),
        https_only: false,
        ..WorkerConfig::default()
    }
}

fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Send `cmd` with the status reset first, so [`settle`] waits for this
/// command's outcome rather than reading the previous one.
fn start(lib: &SharedKitLibrary, cmd: Command) {
    lib.download().state.lock().status = Status::Idle;
    lib.download().send(cmd);
}

fn status(lib: &SharedKitLibrary) -> Status {
    lib.download().state.lock().status.clone()
}

fn is_final(s: &Status) -> bool {
    matches!(s, Status::Done(_) | Status::Error(_) | Status::Cancelled(_))
}

/// Wait for the worker to finish whatever it is doing, and return how.
fn settle(lib: &SharedKitLibrary) -> Status {
    settle_state(&lib.download().state)
}

fn settle_state(state: &Mutex<State>) -> Status {
    let mut last = Status::Idle;
    wait_for("the worker to finish", Duration::from_secs(15), || {
        last = state.lock().status.clone();
        is_final(&last)
    });
    last
}

fn streaming(lib: &SharedKitLibrary) -> bool {
    matches!(
        status(lib),
        Status::Downloading { downloaded_bytes, .. } if downloaded_bytes > 0
    )
}

/// Every non-hidden name directly in `dir`: the kits.
fn kit_dirs(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.') && !n.ends_with(".json") && !n.ends_with(".lock"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// The `.part` files in `dir`.
fn parts(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".zip.part"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// The `.part.meta` records in `dir`.
fn metas(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".zip.part.meta"))
                .collect()
        })
        .unwrap_or_default()
}

fn part_len(dir: &Path) -> u64 {
    let p = parts(dir);
    assert_eq!(p.len(), 1, "expected one .part: {p:?}");
    std::fs::metadata(dir.join(&p[0])).unwrap().len()
}

// ---------------------------------------------------------------------------
// Tests: threads and lifetime
// ---------------------------------------------------------------------------

/// One worker per process, not per instance: two plugin instances whose
/// editors both open the Library share the library and its one thread.
/// Opening the plok.org tab fetches the index on a thread of its own and
/// starts no download worker.
#[test]
fn two_plugin_instances_share_one_worker_thread() {
    use resonance_drums::{library, ResonanceDrums, TestEditor};
    use resonance_plugin::ResonancePlugin;

    library::isolate_for_tests();
    let shared = library::shared();
    assert!(Arc::ptr_eq(&shared, &library::shared()));
    assert!(
        !shared.download().is_running(),
        "opening the library must not start a download thread"
    );

    let (a, b) = (ResonanceDrums::new(), ResonanceDrums::new());
    let mut ea = TestEditor::new(&a, library::shared(), (960.0, 640.0));
    let mut eb = TestEditor::new(&b, library::shared(), (960.0, 640.0));
    // An empty library opens on plok.org, which fetches the index (from a
    // closed loopback port here).
    ea.open_library();
    eb.open_library();
    ea.frame(Vec::new());
    eb.frame(Vec::new());
    assert_eq!(ea.library_tab(), "plok.org");
    assert!(shared.download().fetches_started() >= 1);
    assert!(
        !shared.download().is_running(),
        "an index fetch started the download worker"
    );
    wait_for("the refused fetch to fail", Duration::from_secs(10), || {
        let s = shared.download().state.lock();
        !s.fetching_index && s.index_error.is_some()
    });

    // Two downloads from the two instances: one worker.
    shared
        .download()
        .send(Command::Download(ServerKit::new("A", "a.zip")));
    shared
        .download()
        .send(Command::Download(ServerKit::new("B", "b.zip")));
    assert_eq!(
        shared.download().threads_started(),
        1,
        "two instances started two download threads"
    );
}

/// Cancel acts on the running transfer at once (not queued behind it),
/// removes its `.part` and record, and drops a queued download before it
/// starts.
#[test]
fn cancel_removes_the_part_and_drops_a_queued_download() {
    let home = Home::new("cancel");
    let (_server, index) = Server::start(vec![]);
    let lib = home.library(config(&index));

    start(
        &lib,
        Command::Download(ServerKit::new("Slow Kit", "slow.zip")),
    );
    start(
        &lib,
        Command::Download(ServerKit::new("Next Kit", "next.zip")),
    );
    wait_for(
        "the slow download to stream",
        Duration::from_secs(10),
        || streaming(&lib),
    );
    assert_eq!(
        lib.download().state.lock().queued,
        vec!["Next Kit".to_string()]
    );
    assert_eq!(parts(&home.root()).len(), 1, "it streams into a .part");
    assert_eq!(metas(&home.root()).len(), 1, "the .part has its record");

    // The queued one first: it never starts.
    lib.download().send(Command::Cancel("Next Kit".into()));
    assert!(lib.download().state.lock().queued.is_empty());
    lib.download().send(Command::Cancel("Slow Kit".into()));
    let s = settle(&lib);
    assert_eq!(s, Status::Cancelled("Slow Kit".into()));
    assert!(
        parts(&home.root()).is_empty(),
        "a cancelled download kept its .part"
    );
    assert!(metas(&home.root()).is_empty());
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        status(&lib),
        Status::Cancelled("Slow Kit".into()),
        "the cancelled queued download ran anyway"
    );
}

/// Dropping the last library handle mid-transfer returns at once; with
/// nothing keeping the library alive the transfer is abandoned — not
/// cancelled: its `.part` stays for a resume, unlocked.
#[test]
fn dropping_the_last_handle_abandons_the_transfer_and_keeps_its_part() {
    let home = Home::new("drop");
    let (_server, index) = Server::start(vec![]);
    let lib = home.library(config(&index));
    let state = lib.download().state.clone();
    start(
        &lib,
        Command::Download(ServerKit::new("Slow Kit", "slow.zip")),
    );
    wait_for("the transfer to stream", Duration::from_secs(10), || {
        streaming(&lib)
    });
    let started = Instant::now();
    drop(lib);
    let took = started.elapsed();
    assert!(
        took < Duration::from_secs(1),
        "dropping the library blocked for {took:?} on an in-flight download"
    );
    assert_eq!(settle_state(&state), Status::Cancelled("Slow Kit".into()));
    let part = home.root().join(&parts(&home.root())[0]);
    // The lock goes with the abandoned job.
    let mut unlocked = false;
    wait_for("the .part to be unlocked", Duration::from_secs(5), || {
        let f = std::fs::File::open(&part).unwrap();
        unlocked = f.try_lock().is_ok();
        unlocked
    });
    assert_eq!(metas(&home.root()).len(), 1, "the record went");
}

/// While something asks to keep the library alive (the process-wide one:
/// "while any drum instance lives"), closing the last editor does not
/// stop a download; once nothing does, it is abandoned at the next chunk.
#[test]
fn a_download_outlives_its_editor_while_kept_alive() {
    let home = Home::new("keepalive");
    let (_server, index) = Server::start(vec![]);
    let alive = Arc::new(AtomicBool::new(true));
    let flag = alive.clone();
    let lib = home.library(WorkerConfig {
        keep_alive: Some(KeepAliveFn(Arc::new(move || flag.load(Ordering::SeqCst)))),
        ..config(&index)
    });
    let state = lib.download().state.clone();
    start(
        &lib,
        Command::Download(ServerKit::new("Slow Kit", "slow.zip")),
    );
    wait_for("the transfer to stream", Duration::from_secs(10), || {
        streaming(&lib)
    });
    drop(lib);
    let at = |s: &Status| match s {
        Status::Downloading {
            downloaded_bytes, ..
        } => *downloaded_bytes,
        _ => 0,
    };
    let before = at(&state.lock().status);
    std::thread::sleep(Duration::from_millis(300));
    let now = state.lock().status.clone();
    assert!(
        at(&now) > before,
        "the download stopped with its editor: {now:?}"
    );
    alive.store(false, Ordering::SeqCst);
    assert_eq!(settle_state(&state), Status::Cancelled("Slow Kit".into()));
    assert_eq!(parts(&home.root()).len(), 1, "an abandon keeps the .part");
}

/// Two handles at one root (a detached worker and a re-opened library)
/// never write one `.part`: each has its own nonce, and a locked part is
/// never adopted.
#[test]
fn two_handles_never_share_a_part() {
    let home = Home::new("twohandles");
    let (_server, index) = Server::start(vec![]);
    let a = home.library_marked(config(&index), "marks-a");
    let b = home.library_marked(config(&index), "marks-b");
    let kit = ServerKit::new("Slow Kit", "slow.zip");
    start(&a, Command::Download(kit.clone()));
    start(&b, Command::Download(kit.clone()));
    wait_for("both to stream", Duration::from_secs(10), || {
        streaming(&a) && streaming(&b)
    });
    assert_ne!(a.download().part_path(&kit), b.download().part_path(&kit));
    assert_eq!(parts(&home.root()).len(), 2, "{:?}", parts(&home.root()));
    a.download().send(Command::Cancel("Slow Kit".into()));
    b.download().send(Command::Cancel("Slow Kit".into()));
    settle(&a);
    settle(&b);
    assert!(parts(&home.root()).is_empty());
}

/// The index fetch never waits behind a download: it runs on its own
/// thread while a transfer streams, and leaves the download's status alone.
#[test]
fn an_index_fetch_is_not_blocked_by_an_active_download() {
    let home = Home::new("fetch-beside");
    let index_doc = br#"{"drumkits":[{"name":"K","file":"k.zip"}]}"#.to_vec();
    let (_server, index) = Server::start(vec![("/index.json", index_doc)]);
    let lib = home.library(config(&index));
    start(
        &lib,
        Command::Download(ServerKit::new("Slow Kit", "slow.zip")),
    );
    wait_for("the transfer to stream", Duration::from_secs(10), || {
        streaming(&lib)
    });
    lib.download().send(Command::FetchIndex);
    wait_for("the index", Duration::from_secs(5), || {
        lib.download().state.lock().index.is_some()
    });
    let s = lib.download().state.lock().clone();
    assert!(!s.fetching_index);
    assert_eq!(s.index.unwrap().drumkits.len(), 1);
    assert!(
        matches!(s.status, Status::Downloading { .. }),
        "the fetch took over the download's status: {:?}",
        s.status
    );
    lib.download().send(Command::Cancel("Slow Kit".into()));
    settle(&lib);
}

/// An index fetch that succeeds does not clear the last download's error,
/// and one that fails reports itself in `index_error`.
#[test]
fn an_index_fetch_keeps_the_download_error() {
    let home = Home::new("fetch-error");
    let (server, index) = Server::start(vec![]);
    let lib = home.library(config(&index));
    start(&lib, Command::Download(ServerKit::new("Gone", "gone.zip")));
    assert!(matches!(settle(&lib), Status::Error(_)));
    let error = lib.download().state.lock().last_error.clone();
    assert!(error.is_some());

    start(&lib, Command::FetchIndex);
    wait_for("the fetch", Duration::from_secs(5), || {
        !lib.download().state.lock().fetching_index
    });
    assert!(lib.download().state.lock().index_error.is_some());

    server.replace("/index.json", br#"{"drumkits":[]}"#.to_vec());
    start(&lib, Command::FetchIndex);
    wait_for("the fetch", Duration::from_secs(5), || {
        !lib.download().state.lock().fetching_index
    });
    let s = lib.download().state.lock().clone();
    assert!(s.index.is_some());
    assert_eq!(s.index_error, None);
    assert_eq!(
        s.last_error, error,
        "the fetch cleared the download's error"
    );
}

/// A thread that cannot be spawned leaves nothing stuck: the download is
/// taken off the queue and the fetch flag goes down, each with an error.
#[test]
fn a_spawn_failure_rolls_back() {
    let home = Home::new("spawn");
    let (_server, index) = Server::start(vec![]);
    let lib = home.library(WorkerConfig {
        spawn_hook: Some(SpawnHook(Arc::new(|name| {
            Err(std::io::Error::other(format!("no thread for {name}")))
        }))),
        ..config(&index)
    });
    start(&lib, Command::Download(ServerKit::new("K", "k.zip")));
    let s = lib.download().state.lock().clone();
    assert!(
        matches!(&s.status, Status::Error(e) if e.contains("no thread")),
        "{:?}",
        s.status
    );
    assert!(s.queued.is_empty(), "the download stayed queued");
    assert!(!s.is_working_on("K"));
    assert!(!lib.download().is_running());

    lib.download().send(Command::FetchIndex);
    let s = lib.download().state.lock().clone();
    assert!(!s.fetching_index, "the fetch flag stayed up");
    assert!(s.index_is_stale(), "the tab would never fetch again");
    assert!(s.index_error.unwrap().contains("no thread"));
}

/// A panic in the middle of a job ends that job with an error — not a
/// status stuck on Extracting — and the worker goes on with the next.
#[test]
fn a_worker_panic_is_an_error_and_the_worker_survives() {
    let home = Home::new("panic");
    let (_server, index) = Server::start(vec![
        ("/a.zip", kit_zip("Kit A", "kita", 2)),
        ("/b.zip", kit_zip("Kit B", "kitb", 2)),
    ]);
    let once = Arc::new(AtomicBool::new(true));
    let hook = ExtractHook(Arc::new(move |_| {
        if once.swap(false, Ordering::SeqCst) {
            panic!("boom in the extraction");
        }
    }));
    let lib = home.library(WorkerConfig {
        on_extract_entry: Some(hook),
        ..config(&index)
    });
    start(&lib, Command::Download(ServerKit::new("Kit A", "a.zip")));
    match settle(&lib) {
        Status::Error(e) => assert!(e.contains("boom"), "{e}"),
        other => panic!("not an error: {other:?}"),
    }
    start(&lib, Command::Download(ServerKit::new("Kit B", "b.zip")));
    assert_eq!(settle(&lib), Status::Done("Kit B".into()));
    assert_eq!(lib.download().threads_started(), 1);
}

// ---------------------------------------------------------------------------
// Tests: the queue
// ---------------------------------------------------------------------------

/// A Download right after a Cancel of the same running kit is not lost: it
/// queues behind the cancelled transfer and runs.
#[test]
fn cancel_then_download_the_same_kit_downloads_it() {
    let home = Home::new("cancel-again");
    let (_server, index) = Server::start(vec![]);
    let lib = home.library(config(&index));
    let kit = ServerKit::new("Slow Kit", "slow.zip");
    start(&lib, Command::Download(kit.clone()));
    wait_for("the transfer to stream", Duration::from_secs(10), || {
        streaming(&lib)
    });
    lib.download().send(Command::Cancel(kit.name.clone()));
    lib.download().send(Command::Download(kit.clone()));
    assert!(lib.download().state.lock().is_working_on("Slow Kit"));
    wait_for("the cancel", Duration::from_secs(10), || {
        status(&lib) == Status::Cancelled("Slow Kit".into())
            || lib.download().state.lock().queued.is_empty()
    });
    wait_for("the new transfer", Duration::from_secs(10), || {
        streaming(&lib)
    });
    lib.download().send(Command::Cancel(kit.name.clone()));
    assert_eq!(settle(&lib), Status::Cancelled("Slow Kit".into()));
}

/// A plain Download queued after a Re-download of the same kit (or the
/// other way round) does not lose the Re-download's target: the kit is
/// replaced in place, not installed a second time.
#[test]
fn a_queued_redownload_keeps_its_target() {
    let home = Home::new("merge");
    let zip = kit_zip("Same Kit", "samekit", 2);
    let (_server, index) = Server::start(vec![("/same.zip", zip)]);
    let lib = home.library(config(&index));
    let kit = ServerKit::new("Same Kit", "same.zip");
    start(&lib, Command::Download(kit.clone()));
    assert_eq!(settle(&lib), Status::Done("Same Kit".into()));
    let dir = lib
        .download()
        .state
        .lock()
        .last_installed
        .clone()
        .unwrap()
        .dir;

    // Hold the worker on another kit while the two queue.
    start(
        &lib,
        Command::Download(ServerKit::new("Slow Kit", "slow.zip")),
    );
    wait_for("the transfer to stream", Duration::from_secs(10), || {
        streaming(&lib)
    });
    lib.download().send(Command::Redownload {
        kit: kit.clone(),
        existing_dir: dir.clone(),
    });
    lib.download().send(Command::Download(kit.clone()));
    assert_eq!(
        lib.download().state.lock().queued,
        vec!["Same Kit".to_string()]
    );
    lib.download().send(Command::Cancel("Slow Kit".into()));
    wait_for("the re-download", Duration::from_secs(15), || {
        status(&lib) == Status::Done("Same Kit".into())
    });
    assert_eq!(lib.read().len(), 1, "installed a second copy");
    assert_eq!(kit_dirs(&home.root()), vec!["Same Kit".to_string()]);
}

/// Two installs that finish between two editor frames are both reported.
#[test]
fn every_install_is_reported_even_two_between_frames() {
    let home = Home::new("notices");
    let (_server, index) = Server::start(vec![
        ("/a.zip", kit_zip("Kit A", "kita", 1)),
        ("/b.zip", kit_zip("Kit B", "kitb", 1)),
    ]);
    let lib = home.library(config(&index));
    let seen = lib.download().state.lock().installs;
    lib.download()
        .send(Command::Download(ServerKit::new("Kit A", "a.zip")));
    lib.download()
        .send(Command::Download(ServerKit::new("Kit B", "b.zip")));
    wait_for("both installs", Duration::from_secs(15), || {
        lib.download().state.lock().installs == seen + 2
    });
    let s = lib.download().state.lock();
    let names: Vec<String> = s.installs_since(seen).into_iter().map(|i| i.name).collect();
    assert_eq!(names, vec!["Kit A".to_string(), "Kit B".to_string()]);
    assert_eq!(s.installs_since(seen + 1).len(), 1);
    assert!(s.installs_since(s.installs).is_empty());
}

/// The running editor itself — not just `State::installs_since` in
/// isolation — handles two installs that land before its next poll: both
/// are dropped from `my_downloads`, and both show up in the library view
/// (ba review: `poll_downloads` used to read `last_installed` alone, so
/// of two installs finishing between two frames only the second was ever
/// recognised as "mine").
#[test]
fn the_editor_handles_two_installs_landing_between_two_frames() {
    use resonance_drums::{ResonanceDrums, TestEditor};
    use resonance_plugin::ResonancePlugin;

    let home = Home::new("editor-two-installs");
    let (_server, index) = Server::start(vec![
        ("/a.zip", kit_zip("Kit A", "kita", 1)),
        ("/b.zip", kit_zip("Kit B", "kitb", 1)),
    ]);
    let lib = home.library(config(&index));
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib.clone(), (960.0, 640.0));
    editor.finish_jobs();

    // As `plok_panel` does on Download: record both as this editor's own
    // before asking the worker for them.
    editor.add_my_download("Kit A");
    editor.add_my_download("Kit B");
    let seen = lib.download().state.lock().installs;
    lib.download()
        .send(Command::Download(ServerKit::new("Kit A", "a.zip")));
    lib.download()
        .send(Command::Download(ServerKit::new("Kit B", "b.zip")));

    // Let both finish before the editor polls even once — the "two
    // installs between two frames" case.
    wait_for("both installs", Duration::from_secs(15), || {
        lib.download().state.lock().installs == seen + 2
    });

    editor.frame(Vec::new());

    assert!(
        editor.my_downloads().is_empty(),
        "both downloads should be consumed, got {:?}",
        editor.my_downloads()
    );
    let names = editor.view_names();
    assert!(names.contains(&"Kit A".to_string()), "{names:?}");
    assert!(names.contains(&"Kit B".to_string()), "{names:?}");
    // The jobs run strictly one at a time (one worker thread), so "Kit A"
    // always installs first and "Kit B" second: an editor that only reads
    // `last_installed` sees "Kit B" alone, and — having never dropped "Kit
    // A" from `my_downloads` there — then has `poll_failed_downloads`
    // mistake its own already-installed "Kit A" for a stalled download and
    // report it as "stopped", clobbering the correct notice. Both must be
    // handled as installs, not one as an install and one as a false
    // failure.
    assert_eq!(
        editor.notice().as_deref(),
        Some("downloaded \"Kit B\" — Load plays it in this instance"),
        "the second install's notice should win, not a false report about the first"
    );
}

// ---------------------------------------------------------------------------
// Tests: download, verify, install
// ---------------------------------------------------------------------------

/// A finished download lands through `Library::install_zip`: a library
/// entry whose sidecar carries what the index said, the zip and `.part`
/// gone, nothing half-extracted left in the root. The zip is asked for
/// uncompressed in transit.
#[test]
fn a_download_installs_through_the_library_with_its_sidecar() {
    let home = Home::new("install");
    let zip = kit_zip("Good Kit", "goodkit", 3);
    let sha = sha256_hex(&zip);
    let (server, index) = Server::start(vec![("/good.zip", zip.clone())]);
    let lib = home.library(config(&index));

    let kit = ServerKit {
        sha256: Some(sha.clone()),
        description: Some("A test kit".into()),
        tags: vec!["test".into(), "acoustic".into()],
        bytes: Some(zip.len() as u64),
        ..ServerKit::new("Good Kit", "good.zip")
    };
    start(&lib, Command::Download(kit));
    assert_eq!(settle(&lib), Status::Done("Good Kit".into()));
    assert_eq!(
        server.requests("/good.zip")[0].accept_encoding.as_deref(),
        Some("identity")
    );

    let installed = lib
        .download()
        .state
        .lock()
        .last_installed
        .clone()
        .expect("the install is recorded");
    let entry = lib.read().entry(&installed.id).cloned().expect("indexed");
    assert_eq!(entry.dir, installed.dir);
    assert_eq!(entry.source, Source::Plok);
    assert_eq!(entry.name, "Good Kit");
    assert_eq!(entry.pieces.len(), 3);
    assert!(entry.slot.is_some());

    let sc = read_sidecar(&entry.dir).expect("a sidecar");
    assert_eq!(sc.source, SOURCE_PLOK);
    assert_eq!(sc.index_name.as_deref(), Some("Good Kit"));
    assert_eq!(sc.index_file.as_deref(), Some("good.zip"));
    assert_eq!(sc.sha256.as_deref(), Some(sha.as_str()));
    assert_eq!(sc.description.as_deref(), Some("A test kit"));
    assert_eq!(
        sc.index_tags,
        vec!["test".to_string(), "acoustic".to_string()]
    );
    assert!(sc.downloaded_at.is_some());
    assert!(sc.size_bytes.is_some());

    assert!(parts(&home.root()).is_empty(), "the zip was not deleted");
    assert!(metas(&home.root()).is_empty(), "the record was not deleted");
    assert_eq!(kit_dirs(&home.root()), vec!["Good Kit".to_string()]);
    let staging = home.root().join(".staging");
    assert!(
        kit_dirs(&staging).is_empty(),
        "staging leftovers: {:?}",
        kit_dirs(&staging)
    );
}

/// A kept `.part` is resumed with `Range` and `If-Range` when the server
/// answers 206 — only the rest is transferred — and the result, hashed as
/// it streamed (seeded from the kept bytes), still verifies.
#[test]
fn a_part_is_resumed_over_206() {
    let home = Home::new("resume");
    let zip = kit_zip("Resume Kit", "resumekit", 4);
    let (server, index) = Server::start(vec![("/resume.zip", zip.clone())]);
    let lib = home.library(config(&index));
    let half = zip.len() / 2;
    server.script("/resume.zip", vec![Act::CutAfter(half)]);
    let kit = ServerKit {
        sha256: Some(sha256_hex(&zip)),
        ..ServerKit::new("Resume Kit", "resume.zip")
    };
    start(&lib, Command::Download(kit.clone()));
    assert!(matches!(settle(&lib), Status::Error(_)));
    assert_eq!(part_len(&home.root()), half as u64);

    start(&lib, Command::Download(kit));
    assert_eq!(settle(&lib), Status::Done("Resume Kit".into()));
    let seen = server.requests("/resume.zip");
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(
        seen[1].range.as_deref(),
        Some(format!("bytes={half}-").as_str())
    );
    assert_eq!(seen[1].if_range.as_deref(), Some(etag_of(&zip).as_str()));
    assert_eq!(seen[1].status, 206);
    assert_eq!(seen[1].sent, zip.len() - half, "the whole zip was re-sent");
    assert_eq!(kit_dirs(&home.root()), vec!["Resume Kit".to_string()]);
    assert!(parts(&home.root()).is_empty());
}

/// A server that ignores `Range` answers 200 with the whole file: the
/// download starts over (the kept `.part` is truncated, not appended to).
#[test]
fn a_part_restarts_when_the_server_answers_200() {
    let home = Home::new("restart");
    let zip = kit_zip("Restart Kit", "restartkit", 2);
    let (server, index) = Server::start(vec![("/norange.zip", zip.clone())]);
    let lib = home.library(config(&index));
    server.script("/norange.zip", vec![Act::CutAfter(777)]);
    let kit = ServerKit {
        sha256: Some(sha256_hex(&zip)),
        ..ServerKit::new("Restart Kit", "norange.zip")
    };
    start(&lib, Command::Download(kit.clone()));
    assert!(matches!(settle(&lib), Status::Error(_)));
    start(&lib, Command::Download(kit));
    assert_eq!(settle(&lib), Status::Done("Restart Kit".into()));
    let seen = server.requests("/norange.zip");
    assert_eq!(seen[1].range.as_deref(), Some("bytes=777-"));
    assert_eq!(seen[1].status, 200);
    assert_eq!(seen[1].sent, zip.len());
    assert!(parts(&home.root()).is_empty());
}

/// The file changed on the server since the part was kept: `If-Range`
/// fails, the server sends the whole new file, and that is what installs —
/// never the old bytes with the new ones appended.
#[test]
fn an_if_range_mismatch_restarts_the_download() {
    let home = Home::new("ifrange");
    let old = kit_zip("Old Kit", "changing", 3);
    let new = kit_zip("New Kit", "changing", 4);
    let (server, index) = Server::start(vec![("/changing.zip", old.clone())]);
    let lib = home.library(config(&index));
    // No sha256 or size in the index: only the validator can tell.
    let kit = ServerKit::new("Changing", "changing.zip");
    server.script("/changing.zip", vec![Act::CutAfter(old.len() / 2)]);
    start(&lib, Command::Download(kit.clone()));
    assert!(matches!(settle(&lib), Status::Error(_)));

    server.replace("/changing.zip", new.clone());
    start(&lib, Command::Download(kit));
    assert_eq!(settle(&lib), Status::Done("Changing".into()));
    let seen = server.requests("/changing.zip");
    assert_eq!(seen[1].if_range.as_deref(), Some(etag_of(&old).as_str()));
    assert_eq!(seen[1].status, 200);
    assert_eq!(seen[1].sent, new.len());
    let id = lib
        .download()
        .state
        .lock()
        .last_installed
        .clone()
        .unwrap()
        .id;
    assert_eq!(
        lib.read().entry(&id).unwrap().pieces.len(),
        4,
        "the old bytes installed"
    );
}

/// A server error (503) is not the end of a resumable download: the
/// `.part` stays and the next try resumes it. A definite 404 removes it.
#[test]
fn a_5xx_keeps_the_part_and_a_404_removes_it() {
    let home = Home::new("5xx");
    let zip = kit_zip("Flaky Kit", "flakykit", 4);
    let (server, index) = Server::start(vec![("/flaky.zip", zip.clone())]);
    let lib = home.library(config(&index));
    let half = zip.len() / 2;
    let kit = ServerKit::new("Flaky Kit", "flaky.zip");
    server.script(
        "/flaky.zip",
        vec![Act::CutAfter(half), Act::Status(503), Act::Status(429)],
    );
    start(&lib, Command::Download(kit.clone()));
    assert!(matches!(settle(&lib), Status::Error(_)));
    for code in ["503", "429"] {
        start(&lib, Command::Download(kit.clone()));
        match settle(&lib) {
            Status::Error(e) => assert!(e.contains(code), "{e}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            part_len(&home.root()),
            half as u64,
            "a {code} lost the .part"
        );
    }
    start(&lib, Command::Download(kit.clone()));
    assert_eq!(settle(&lib), Status::Done("Flaky Kit".into()));
    let seen = server.requests("/flaky.zip");
    assert_eq!(
        seen[3].range.as_deref(),
        Some(format!("bytes={half}-").as_str())
    );
    assert_eq!(seen[3].sent, zip.len() - half);

    // A 404 is a definite no.
    let other = ServerKit::new("Gone Kit", "gone.zip");
    server.replace("/gone.zip", zip.clone());
    server.script("/gone.zip", vec![Act::CutAfter(half), Act::Status(404)]);
    start(&lib, Command::Download(other.clone()));
    assert!(matches!(settle(&lib), Status::Error(_)));
    assert_eq!(parts(&home.root()).len(), 1);
    start(&lib, Command::Download(other));
    assert!(matches!(settle(&lib), Status::Error(_)));
    assert!(parts(&home.root()).is_empty(), "a 404 kept the .part");
}

/// Dropping the library while the zip is being installed abandons the
/// install (nothing lands, no staging is left) but keeps the complete
/// zip: the next download of the kit asks for the rest, gets a 416 naming
/// exactly the size it has, and installs without transferring a byte.
#[test]
fn a_drop_during_install_keeps_the_zip_and_the_next_try_needs_no_transfer() {
    let home = Home::new("drop-install");
    let zip = kit_zip("Whole Kit", "wholekit", 6);
    let (server, index) = Server::start(vec![("/whole.zip", zip.clone())]);
    let (at_tx, at_rx) = crossbeam_channel::bounded::<()>(1);
    let (go_tx, go_rx) = crossbeam_channel::bounded::<()>(1);
    let hook = ExtractHook(Arc::new(move |i| {
        if i == 1 {
            let _ = at_tx.send(());
            let _ = go_rx.recv_timeout(Duration::from_secs(10));
        }
    }));
    let lib = home.library(WorkerConfig {
        on_extract_entry: Some(hook),
        ..config(&index)
    });
    let state = lib.download().state.clone();
    let kit = ServerKit {
        sha256: Some(sha256_hex(&zip)),
        bytes: Some(zip.len() as u64),
        ..ServerKit::new("Whole Kit", "whole.zip")
    };
    start(&lib, Command::Download(kit.clone()));
    at_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the extraction never started");
    drop(lib);
    let _ = go_tx.send(());
    assert_eq!(settle_state(&state), Status::Cancelled("Whole Kit".into()));
    assert!(
        kit_dirs(&home.root()).is_empty(),
        "{:?}",
        kit_dirs(&home.root())
    );
    assert!(kit_dirs(&home.root().join(".staging")).is_empty());
    assert_eq!(part_len(&home.root()), zip.len() as u64);

    let lib = home.library(config(&index));
    start(&lib, Command::Download(kit));
    assert_eq!(settle(&lib), Status::Done("Whole Kit".into()));
    let seen = server.requests("/whole.zip");
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(seen[1].status, 416);
    assert_eq!(seen[1].sent, 0);
    assert_eq!(kit_dirs(&home.root()), vec!["Whole Kit".to_string()]);
    assert!(parts(&home.root()).is_empty());
}

/// A zip whose sha256 is not the index's is refused before extraction:
/// nothing installed, nothing left behind.
#[test]
fn a_sha_mismatch_is_refused() {
    let home = Home::new("sha");
    let zip = kit_zip("Bad Sha", "badsha", 2);
    let (_server, index) = Server::start(vec![("/badsha.zip", zip)]);
    let lib = home.library(config(&index));

    let kit = ServerKit {
        sha256: Some("0".repeat(64)),
        ..ServerKit::new("Bad Sha", "badsha.zip")
    };
    start(&lib, Command::Download(kit));
    match settle(&lib) {
        Status::Error(e) => assert!(e.contains("verification"), "{e}"),
        other => panic!("a corrupt download installed: {other:?}"),
    }
    assert!(lib.read().is_empty());
    assert!(kit_dirs(&home.root()).is_empty());
    assert!(parts(&home.root()).is_empty());
    assert!(metas(&home.root()).is_empty());
}

/// With less free space than `zip × 2.1`, the download is refused before
/// a byte moves, and the message names the shortfall.
#[test]
fn a_full_disk_refuses_the_download_naming_the_shortfall() {
    let home = Home::new("space");
    let zip = kit_zip("Big Kit", "bigkit", 2);
    let (server, index) = Server::start(vec![("/big.zip", zip.clone())]);
    let lib = home.library(WorkerConfig {
        free_space: Some(FreeSpaceFn(Arc::new(|_| Some(1000)))),
        ..config(&index)
    });

    // The index gives the size: refused without a request.
    let kit = ServerKit {
        bytes: Some(zip.len() as u64),
        ..ServerKit::new("Big Kit", "big.zip")
    };
    start(&lib, Command::Download(kit));
    match settle(&lib) {
        Status::Error(e) => assert!(e.contains("short") && e.contains("free space"), "{e}"),
        other => panic!("not refused: {other:?}"),
    }
    assert!(
        server.requests("/big.zip").is_empty(),
        "the zip was requested anyway"
    );

    // Without `bytes`, the Content-Length decides — still before writing.
    start(
        &lib,
        Command::Download(ServerKit::new("Big Kit", "big.zip")),
    );
    match settle(&lib) {
        Status::Error(e) => assert!(e.contains("short"), "{e}"),
        other => panic!("not refused: {other:?}"),
    }
    assert!(parts(&home.root()).is_empty());
    assert!(lib.read().is_empty());
}

/// Update / Re-download goes through `install_zip_replacing`: the kit is
/// replaced in place, and with an unchanged manifest it keeps its id, its
/// slot and its marks.
#[test]
fn an_update_replaces_in_place_and_keeps_id_slot_and_marks() {
    let home = Home::new("update");
    let zip = kit_zip("Same Kit", "samekit", 2);
    let (server, index) = Server::start(vec![("/same.zip", zip)]);
    let lib = home.library(config(&index));

    start(
        &lib,
        Command::Download(ServerKit::new("Same Kit", "same.zip")),
    );
    assert_eq!(settle(&lib), Status::Done("Same Kit".into()));
    let first = lib.download().state.lock().last_installed.clone().unwrap();
    let before = lib.read().entry(&first.id).cloned().unwrap();
    lib.toggle_favorite(&before.id).unwrap();
    // Damage it: a re-download repairs it.
    std::fs::remove_file(before.manifest_path.with_file_name("piece0.wav")).unwrap();

    start(
        &lib,
        Command::Redownload {
            kit: ServerKit {
                description: Some("fresh".into()),
                ..ServerKit::new("Same Kit", "same.zip")
            },
            existing_dir: before.dir.clone(),
        },
    );
    assert_eq!(settle(&lib), Status::Done("Same Kit".into()));
    assert_eq!(server.requests("/same.zip").len(), 2);

    let after = lib
        .read()
        .by_dir(&before.dir)
        .cloned()
        .expect("same directory");
    assert_eq!(after.id, before.id);
    assert_eq!(after.slot, before.slot);
    assert_eq!(after.description(), Some("fresh"));
    assert!(lib.marks_of(&after.id).favorite, "the favourite was lost");
    assert!(before.manifest_path.with_file_name("piece0.wav").exists());
    assert_eq!(lib.read().len(), 1);
}

/// An Update to a new version of the kit (its manifest changed, so its id
/// did) carries the favourite, the tags and the recents to the new id, and
/// leaves nothing under the old one.
#[test]
fn an_update_to_a_new_manifest_keeps_the_marks() {
    let home = Home::new("update-marks");
    let v1 = kit_zip("Versioned Kit", "versioned", 2);
    let v2 = kit_zip("Versioned Kit", "versioned", 3);
    let (server, index) = Server::start(vec![("/versioned.zip", v1)]);
    let lib = home.library(config(&index));
    let kit = ServerKit::new("Versioned Kit", "versioned.zip");
    start(&lib, Command::Download(kit.clone()));
    assert_eq!(settle(&lib), Status::Done("Versioned Kit".into()));
    let old = lib.download().state.lock().last_installed.clone().unwrap();
    lib.toggle_favorite(&old.id).unwrap();
    lib.add_tag(&old.id, "loud").unwrap();
    lib.record_use(&old.id).unwrap();

    server.replace("/versioned.zip", v2);
    start(
        &lib,
        Command::Redownload {
            kit,
            existing_dir: old.dir.clone(),
        },
    );
    assert_eq!(settle(&lib), Status::Done("Versioned Kit".into()));
    let new = lib.download().state.lock().last_installed.clone().unwrap();
    assert_ne!(new.id, old.id, "the manifest changed, so must the id");
    assert_eq!(new.dir, old.dir);
    let marks = lib.marks_of(&new.id);
    assert!(marks.favorite, "the favourite was lost");
    assert!(marks.has_tag("loud"), "the tag was lost");
    assert!(
        marks.last_used.is_some() && marks.use_count == 1,
        "{marks:?}"
    );
    assert!(
        lib.marks_of(&old.id).is_default(),
        "the old id kept its marks"
    );
}

/// A cancel in the middle of extraction stops it and leaves no kit, no
/// zip and no staging leftovers.
#[test]
fn a_cancel_mid_extraction_leaves_nothing() {
    let home = Home::new("extract-cancel");
    let zip = kit_zip("Many Kit", "manykit", 8);
    let (_server, index) = Server::start(vec![("/many.zip", zip)]);
    let (at_tx, at_rx) = crossbeam_channel::bounded::<()>(1);
    let (go_tx, go_rx) = crossbeam_channel::bounded::<()>(1);
    let hook = ExtractHook(Arc::new(move |i| {
        if i == 2 {
            let _ = at_tx.send(());
            let _ = go_rx.recv_timeout(Duration::from_secs(10));
        }
    }));
    let lib = home.library(WorkerConfig {
        on_extract_entry: Some(hook),
        ..config(&index)
    });
    start(
        &lib,
        Command::Download(ServerKit::new("Many Kit", "many.zip")),
    );
    at_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the extraction never reached entry 2");
    assert!(matches!(status(&lib), Status::Extracting { .. }));
    lib.download().send(Command::Cancel("Many Kit".into()));
    let _ = go_tx.send(());
    assert_eq!(settle(&lib), Status::Cancelled("Many Kit".into()));
    assert!(lib.read().is_empty());
    assert!(kit_dirs(&home.root()).is_empty());
    assert!(parts(&home.root()).is_empty());
    assert!(kit_dirs(&home.root().join(".staging")).is_empty());
}

/// The per-read timeout: a server that sends headers and then stalls
/// fails the download once one read has waited that long. The `.part`
/// is kept: the next try resumes it.
#[test]
fn a_stalled_body_times_out_and_keeps_its_part_for_a_resume() {
    let home = Home::new("stall");
    let (_server, index) = Server::start(vec![]);
    let limit = Duration::from_millis(300);
    let lib = home.library(WorkerConfig {
        read_timeout: limit,
        ..config(&index)
    });
    let started = Instant::now();
    let kit = ServerKit::new("Stall Kit", "stall.zip");
    start(&lib, Command::Download(kit.clone()));
    assert!(matches!(settle(&lib), Status::Error(_)));
    let took = started.elapsed();
    assert!(
        took < limit * 10,
        "a {limit:?} read timeout took {took:?} to fire"
    );
    let own = lib.download().part_path(&kit).unwrap();
    assert_eq!(
        parts(&home.root()),
        vec![own.file_name().unwrap().to_string_lossy().into_owned()],
    );
}

/// A worker starting up removes download leftovers nobody can own — an
/// unlocked part with no record (it cannot be resumed), an unlocked part
/// untouched for an hour, a record whose part is gone, the old worker's
/// staging dirs — and never a part some live download holds locked, nor a
/// fresh, resumable one.
#[test]
fn a_starting_worker_sweeps_stale_download_leftovers() {
    let home = Home::new("sweep");
    let root = home.root();
    std::fs::create_dir_all(&root).unwrap();
    let locked = root.join(".locked-0123456789ab.aaaa.zip.part");
    let resumable = root.join(".resumable-0123456789ab.bbbb.zip.part");
    let old = root.join(".old-0123456789ab.cccc.zip.part");
    let no_record = root.join(".Legacy.12345.zip.part");
    let orphan_meta = root.join(".gone-0123456789ab.dddd.zip.part.meta");
    let staging = root.join(".Dead.999999999-1.extracting");
    let unrelated = root.join("notes.txt");
    for f in [&locked, &resumable, &old, &no_record, &unrelated] {
        std::fs::write(f, b"x").unwrap();
    }
    for p in [&locked, &resumable, &old] {
        let mut meta = p.as_os_str().to_owned();
        meta.push(".meta");
        std::fs::write(PathBuf::from(meta), b"{}").unwrap();
    }
    std::fs::write(&orphan_meta, b"{}").unwrap();
    std::fs::create_dir_all(staging.join("inner")).unwrap();
    let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(2 * 3600);
    for p in [&old, &orphan_meta] {
        std::fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(two_hours_ago)
            .unwrap();
    }
    let holder = std::fs::File::options().write(true).open(&locked).unwrap();
    holder.lock().unwrap();
    std::fs::File::options()
        .write(true)
        .open(&locked)
        .unwrap()
        .set_modified(two_hours_ago)
        .unwrap();

    let (_server, index) = Server::start(vec![]);
    let lib = home.library(config(&index));
    // A download starts the worker; it 404s.
    start(&lib, Command::Download(ServerKit::new("X", "x.zip")));
    assert!(matches!(settle(&lib), Status::Error(_)));

    assert!(locked.exists(), "a locked (live) .part was swept");
    assert!(resumable.exists(), "a fresh resumable .part was swept");
    assert!(!old.exists(), "an hour-old unlocked .part survived");
    assert!(!no_record.exists(), "a .part without a record survived");
    assert!(!orphan_meta.exists(), "a record without a part survived");
    assert!(!staging.exists(), "an old staging dir survived");
    assert!(unrelated.exists());
    drop(holder);
}

/// The §4.2 index: every new field optional, the current file (a `size`
/// string, no `bytes`) still parses, and one field of the wrong type does
/// not cost the whole index.
#[test]
fn the_index_parser_takes_the_new_fields_and_tolerates_the_old_file() {
    let full = br#"{ "drumkits": [ {
        "name": "Drummica", "file": "drummica.zip",
        "bytes": 5690000000, "sha256": "ABC",
        "pieces": 35, "mic_setups": 14,
        "description": "Acoustic studio kit.",
        "tags": ["acoustic", "rock"], "added": "2026-04-12",
        "manifest_sha256": "def"
    } ] }"#;
    let index = download::parse_index(full).unwrap();
    let k = &index.drumkits[0];
    assert_eq!(k.bytes, Some(5_690_000_000));
    assert_eq!(k.sha256.as_deref(), Some("ABC"));
    assert_eq!(k.pieces, Some(35));
    assert_eq!(k.mic_setups, Some(14));
    assert_eq!(k.manifest_sha256.as_deref(), Some("def"));
    assert_eq!(k.tags, vec!["acoustic".to_string(), "rock".to_string()]);
    assert_eq!(
        k.size_text(),
        Some(resonance_common::drumkit_library::format_bytes(
            5_690_000_000
        )),
        "bytes wins over the display string"
    );

    let current = br#"{"drumkits":[{"name":"Drummica","file":"drummica.zip","size":"5.3 GiB",
        "description":"x","tags":["a"],"added":"2026-04-12"}]}"#;
    let index = download::parse_index(current).unwrap();
    assert_eq!(index.drumkits[0].bytes, None);
    assert_eq!(index.drumkits[0].size_text().as_deref(), Some("5.3 GiB"));
    assert!(index.find("drummica").is_some(), "find is case-insensitive");

    let odd =
        br#"{"drumkits":[{"name":"K","file":"k.zip","bytes":"lots","pieces":-3,"tags":"x"}]}"#;
    let index = download::parse_index(odd).unwrap();
    assert_eq!(index.drumkits[0].bytes, None);
    assert_eq!(index.drumkits[0].pieces, None);
    assert!(index.drumkits[0].tags.is_empty());
}

/// The production config refuses plain HTTP.
#[test]
fn the_default_config_is_https_only() {
    let home = Home::new("https");
    let (server, index) = Server::start(vec![("/k.zip", kit_zip("K", "k", 1))]);
    let lib = home.library(WorkerConfig {
        index_url: index,
        ..WorkerConfig::default()
    });
    start(&lib, Command::Download(ServerKit::new("K", "k.zip")));
    assert!(matches!(settle(&lib), Status::Error(_)));
    assert!(server.requests("/k.zip").is_empty());
}
