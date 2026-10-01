//! The process-wide plok.org download worker against a local HTTP server
//! (drums-plugin-rework.md §4.1, §9).
//!
//! Every test builds its own kit library at a temp root
//! (`SharedKitLibrary::open`) whose worker fetches from an in-test
//! `TcpListener` server, so the real data dir and the real server are
//! never touched and the tests run side by side.
#![cfg(feature = "editor")]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use resonance_common::drumkit_library::{read_sidecar, Source, SOURCE_PLOK};
use resonance_drums::download::{
    self, Command, ExtractHook, FreeSpaceFn, ServerKit, Status, WorkerConfig,
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

/// One request the server saw: its path and its `Range` header.
#[derive(Debug, Clone)]
struct Seen {
    path: String,
    range: Option<String>,
    /// Body bytes the server sent back.
    sent: usize,
}

/// Path → body.
type Files = Vec<(String, Vec<u8>)>;

#[derive(Clone, Default)]
struct Server {
    /// Path → zip bytes; served honouring `Range` unless the path starts
    /// with `/norange`.
    files: Arc<Mutex<Files>>,
    log: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    fn start(files: Vec<(&str, Vec<u8>)>) -> (Self, String) {
        let server = Server {
            files: Arc::new(Mutex::new(
                files.into_iter().map(|(p, b)| (p.to_string(), b)).collect(),
            )),
            log: Arc::default(),
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

    fn requests(&self, path: &str) -> Vec<Seen> {
        self.log
            .lock()
            .iter()
            .filter(|s| s.path == path)
            .cloned()
            .collect()
    }

    fn serve(&self, mut stream: TcpStream) {
        let Some((path, range)) = read_request(&stream) else {
            return;
        };
        let mut sent = 0;
        match path.as_str() {
            // Trickles a huge body until the client goes away.
            "/slow.zip" => {
                let head = "HTTP/1.1 200 OK\r\nContent-Length: 1073741824\r\n\
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
                let head = "HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\n\r\n";
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&[0u8; 4096]);
                let _ = stream.flush();
                std::thread::sleep(Duration::from_secs(30));
            }
            _ => {
                let body = self
                    .files
                    .lock()
                    .iter()
                    .find(|(p, _)| *p == path)
                    .map(|(_, b)| b.clone());
                match body {
                    None => {
                        let _ = stream
                            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                    }
                    Some(body) => {
                        let start = range
                            .as_deref()
                            .filter(|_| !path.starts_with("/norange"))
                            .and_then(|r| r.strip_prefix("bytes="))
                            .and_then(|r| r.strip_suffix('-'))
                            .and_then(|n| n.parse::<usize>().ok());
                        sent = match start {
                            Some(s) if s < body.len() => {
                                let head = format!(
                                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\n\
                                     Content-Range: bytes {s}-{}/{}\r\n\r\n",
                                    body.len() - s,
                                    body.len() - 1,
                                    body.len()
                                );
                                let _ = stream.write_all(head.as_bytes());
                                let _ = stream.write_all(&body[s..]);
                                body.len() - s
                            }
                            _ => {
                                let head = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                                    body.len()
                                );
                                let _ = stream.write_all(head.as_bytes());
                                let _ = stream.write_all(&body);
                                body.len()
                            }
                        };
                    }
                }
            }
        }
        self.log.lock().push(Seen { path, range, sent });
    }
}

/// Read the request line and headers; return the path and `Range`.
fn read_request(stream: &TcpStream) -> Option<(String, Option<String>)> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let path = line.split_whitespace().nth(1)?.to_string();
    let mut range = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 || header == "\r\n" {
            break;
        }
        if let Some((k, v)) = header.split_once(':') {
            if k.trim().eq_ignore_ascii_case("range") {
                range = Some(v.trim().to_string());
            }
        }
    }
    Some((path, range))
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
        SharedKitLibrary::open(Roots {
            root: Some(self.root()),
            marks_dir: Some(self.0.join("library")),
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

/// Wait for the worker to finish whatever it is doing, and return how.
fn settle(lib: &SharedKitLibrary) -> Status {
    let mut last = Status::Idle;
    wait_for("the worker to finish", Duration::from_secs(15), || {
        last = status(lib);
        matches!(
            last,
            Status::Done(_) | Status::Error(_) | Status::Cancelled(_)
        )
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
    std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".zip.part"))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// One worker per process, not per instance: two plugin instances whose
/// editors both open the Library share the library and its one thread.
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
    assert!(shared.download().is_running());
    assert_eq!(
        shared.download().threads_started(),
        1,
        "two instances started two download threads"
    );
    wait_for("the refused fetch to fail", Duration::from_secs(10), || {
        matches!(status(&shared), Status::Error(_))
    });
}

/// Cancel acts on the running transfer at once (not queued behind it),
/// removes its `.part`, and drops a queued download before it starts.
/// Dropping the last library handle mid-transfer never blocks.
#[test]
fn cancel_removes_the_part_and_drop_never_blocks() {
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
    assert_eq!(
        parts(&home.root()).len(),
        1,
        "the transfer streams into a .part"
    );

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
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        status(&lib),
        Status::Cancelled("Slow Kit".into()),
        "the cancelled queued download ran anyway"
    );

    // Dropping the last handle mid-transfer returns at once.
    start(
        &lib,
        Command::Download(ServerKit::new("Slow Kit", "slow.zip")),
    );
    wait_for(
        "the second transfer to stream",
        Duration::from_secs(10),
        || streaming(&lib),
    );
    let started = Instant::now();
    drop(lib);
    let took = started.elapsed();
    assert!(
        took < Duration::from_secs(1),
        "dropping the library blocked for {took:?} on an in-flight download"
    );
    wait_for("the abandoned .part to go", Duration::from_secs(5), || {
        parts(&home.root()).is_empty()
    });
}

/// A finished download lands through `Library::install_zip`: a library
/// entry whose sidecar carries what the index said, the zip and `.part`
/// gone, nothing half-extracted left in the root.
#[test]
fn a_download_installs_through_the_library_with_its_sidecar() {
    let home = Home::new("install");
    let zip = kit_zip("Good Kit", "goodkit", 3);
    let sha = sha256_hex(&zip);
    let (_server, index) = Server::start(vec![("/good.zip", zip.clone())]);
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
    assert_eq!(kit_dirs(&home.root()), vec!["Good Kit".to_string()]);
    let staging = home.root().join(".staging");
    assert!(
        kit_dirs(&staging).is_empty(),
        "staging leftovers: {:?}",
        kit_dirs(&staging)
    );
}

/// An own `.part` is resumed with `Range` when the server answers 206 —
/// only the rest is transferred — and the result still verifies.
#[test]
fn a_part_is_resumed_over_206() {
    let home = Home::new("resume");
    let zip = kit_zip("Resume Kit", "resumekit", 4);
    let (server, index) = Server::start(vec![("/resume.zip", zip.clone())]);
    let lib = home.library(config(&index));

    let half = zip.len() / 2;
    std::fs::create_dir_all(home.root()).unwrap();
    std::fs::write(
        download::part_path(&home.root(), "Resume Kit"),
        &zip[..half],
    )
    .unwrap();

    let kit = ServerKit {
        sha256: Some(sha256_hex(&zip)),
        ..ServerKit::new("Resume Kit", "resume.zip")
    };
    start(&lib, Command::Download(kit));
    assert_eq!(settle(&lib), Status::Done("Resume Kit".into()));

    let seen = server.requests("/resume.zip");
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(
        seen[0].range.as_deref(),
        Some(format!("bytes={half}-").as_str())
    );
    assert_eq!(seen[0].sent, zip.len() - half, "the whole zip was re-sent");
    assert_eq!(kit_dirs(&home.root()), vec!["Resume Kit".to_string()]);
}

/// A server that ignores `Range` answers 200 with the whole file: the
/// download starts over (the stale `.part` is truncated, not appended to).
#[test]
fn a_part_restarts_when_the_server_answers_200() {
    let home = Home::new("restart");
    let zip = kit_zip("Restart Kit", "restartkit", 2);
    let (server, index) = Server::start(vec![("/norange.zip", zip.clone())]);
    let lib = home.library(config(&index));

    std::fs::create_dir_all(home.root()).unwrap();
    std::fs::write(
        download::part_path(&home.root(), "Restart Kit"),
        vec![0xEEu8; 777],
    )
    .unwrap();
    let kit = ServerKit {
        sha256: Some(sha256_hex(&zip)),
        ..ServerKit::new("Restart Kit", "norange.zip")
    };
    start(&lib, Command::Download(kit));
    assert_eq!(settle(&lib), Status::Done("Restart Kit".into()));
    let seen = server.requests("/norange.zip");
    assert_eq!(seen[0].range.as_deref(), Some("bytes=777-"));
    assert_eq!(seen[0].sent, zip.len());
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
    start(
        &lib,
        Command::Download(ServerKit::new("Stall Kit", "stall.zip")),
    );
    assert!(matches!(settle(&lib), Status::Error(_)));
    let took = started.elapsed();
    assert!(
        took < limit * 10,
        "a {limit:?} read timeout took {took:?} to fire"
    );
    assert_eq!(
        parts(&home.root()),
        vec![download::part_path(&home.root(), "Stall Kit")
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()],
    );
}

/// A worker starting up removes download leftovers nobody can own: those
/// of a process that is gone (Linux can tell) and any untouched for an
/// hour — never this process's own, nor a fresh one of unknown owner.
#[test]
fn a_starting_worker_sweeps_stale_download_leftovers() {
    let home = Home::new("sweep");
    let root = home.root();
    std::fs::create_dir_all(&root).unwrap();
    let me = std::process::id();
    // No pid ever reaches 999999999 (Linux caps pid_max at 2^22).
    let dead_part = root.join(".Dead.999999999.zip.part");
    let dead_legacy = root.join(".Dead.999999999-0.zip.part");
    let dead_staging = root.join(".Dead.999999999-1.extracting");
    let mine = root.join(format!(".Mine.{me}.zip.part"));
    let old = root.join(".Old.zip.part");
    let fresh = root.join(".Fresh.zip.part");
    let unrelated = root.join("notes.txt");
    for f in [&dead_part, &dead_legacy, &mine, &old, &fresh, &unrelated] {
        std::fs::write(f, b"x").unwrap();
    }
    std::fs::create_dir_all(dead_staging.join("inner")).unwrap();
    let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(2 * 3600);
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(two_hours_ago)
        .unwrap();

    let (_server, index) = Server::start(vec![]);
    let lib = home.library(config(&index));
    // Any command starts the worker; the index fetch 404s.
    start(&lib, Command::FetchIndex);
    assert!(matches!(settle(&lib), Status::Error(_)));

    assert!(!old.exists(), "an hour-old leftover survived");
    assert!(
        mine.exists(),
        "this process's own .part (resumable) was swept"
    );
    assert!(
        fresh.exists(),
        "a fresh leftover of unknown owner was swept"
    );
    assert!(unrelated.exists());
    if cfg!(target_os = "linux") {
        assert!(!dead_part.exists(), "a dead process's .part survived");
        assert!(
            !dead_legacy.exists(),
            "a dead process's old-style .part survived"
        );
        assert!(
            !dead_staging.exists(),
            "a dead process's staging dir survived"
        );
    }
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
