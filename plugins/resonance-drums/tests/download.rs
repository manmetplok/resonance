//! Kit download worker against a local HTTP server
//! (drums-plugin-rework.md §1.1, §9).
//!
//! `WorkerHandle::drop` used to join the worker thread, which only read
//! its Shutdown command between downloads: closing a project during a
//! 5 GiB download blocked until the download finished. A failed or
//! abandoned download also left its `.part` file behind.
//!
//! Every worker here gets its own data directory through
//! `WorkerConfig::data_dir` — not `$XDG_DATA_HOME`, which is
//! process-global and which `dirs::data_dir` ignores on macOS — so the
//! real one is never touched and the tests can run side by side.

#![cfg(feature = "editor")]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use resonance_drums::download::{self, Command, ServerKit, Status, WorkerConfig, WorkerHandle};

/// Body length the slow kit announces — far more than the test ever
/// lets it send.
const SLOW_LEN: u64 = 1 << 30;
const CHUNK: usize = 64 * 1024;

fn kit(name: &str, file: &str) -> ServerKit {
    ServerKit {
        name: name.to_string(),
        file: file.to_string(),
        size: None,
        description: None,
        tags: Vec::new(),
        added: None,
    }
}

/// Read the request line and headers; return the path.
fn read_request(stream: &TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let path = line.split_whitespace().nth(1)?.to_string();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 || header == "\r\n" {
            break;
        }
    }
    Some(path)
}

fn serve(mut stream: TcpStream) {
    let Some(path) = read_request(&stream) else {
        return;
    };
    match path.as_str() {
        // Trickles a huge body until the client goes away.
        "/slow.zip" => {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {SLOW_LEN}\r\n\
                 Content-Type: application/zip\r\n\r\n"
            );
            if stream.write_all(head.as_bytes()).is_err() {
                return;
            }
            let chunk = vec![0u8; CHUNK];
            loop {
                if stream.write_all(&chunk).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        // Announces 1 MiB, sends a tenth of it, hangs up.
        "/broken.zip" => {
            let head = "HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\n\
                        Content-Type: application/zip\r\n\r\n";
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&vec![0u8; 104_857]);
        }
        // Headers and a first chunk, then silence with the connection
        // held open — a server that has gone quiet mid-body.
        "/stall.zip" => {
            let head = "HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\n\
                        Content-Type: application/zip\r\n\r\n";
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&[0u8; 4096]);
            let _ = stream.flush();
            std::thread::sleep(Duration::from_secs(60));
        }
        // A small, valid kit.
        "/good.zip" => send_body(&mut stream, &zip_of(&good_kit_entries())),
        _ => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        }
    }
}

fn send_body(stream: &mut TcpStream, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/zip\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn good_kit_entries() -> Vec<(String, Vec<u8>)> {
    vec![
        ("good/drum_samples.json".to_string(), b"{}".to_vec()),
        ("good/kick.wav".to_string(), vec![7u8; 1000]),
    ]
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

fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || serve(stream));
        }
    });
    format!("http://{addr}/index.json")
}

fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn part_file(dir: &Path, sanitized: &str) -> PathBuf {
    dir.join(format!(".{sanitized}.zip.part"))
}

/// A fresh data directory for one test, removed on drop.
struct DataHome(PathBuf);

impl DataHome {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "resonance-drums-download-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn config(&self, index_url: &str) -> WorkerConfig {
        WorkerConfig {
            index_url: index_url.to_string(),
            data_dir: Some(self.0.clone()),
            ..WorkerConfig::default()
        }
    }

    fn worker(&self, index_url: &str) -> WorkerHandle {
        download::spawn_with(self.config(index_url))
    }

    fn kits_dir(&self) -> PathBuf {
        self.config("").drumkits_dir().expect("drumkits dir")
    }
}

impl Drop for DataHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn download_worker_cancels_cleans_up_and_never_blocks_drop() {
    let home = DataHome::new("cancel");
    let kits_dir = home.kits_dir();
    assert!(
        kits_dir.starts_with(&home.0),
        "download dir {} escaped the test's data home",
        kits_dir.display()
    );

    let index_url = start_server();

    // --- A transfer the server cuts short fails and leaves no .part ---
    let worker = home.worker(&index_url);
    worker.send(Command::Download(kit("Broken Kit", "broken.zip")));
    wait_for(
        "the broken download to fail",
        Duration::from_secs(10),
        || matches!(worker.state.lock().status, Status::Error(_)),
    );
    assert!(
        !part_file(&kits_dir, "Broken_Kit").exists(),
        "a failed download left its .part file behind"
    );
    drop(worker);

    // --- Dropping the handle mid-transfer returns at once ---
    let worker = home.worker(&index_url);
    worker.send(Command::Download(kit("Slow Kit", "slow.zip")));
    wait_for(
        "the slow download to start",
        Duration::from_secs(10),
        || {
            matches!(
                worker.state.lock().status,
                Status::Downloading { downloaded_bytes, .. } if downloaded_bytes > 0
            )
        },
    );
    let part = part_file(&kits_dir, "Slow_Kit");
    assert!(
        part.exists(),
        "the transfer should be streaming into {}",
        part.display()
    );

    let started = Instant::now();
    drop(worker);
    let took = started.elapsed();
    assert!(
        took < Duration::from_secs(1),
        "dropping the worker blocked for {took:?} on an in-flight download"
    );

    // The detached worker sees the cancel at its next chunk and removes
    // the partial file.
    wait_for(
        "the cancelled .part to be removed",
        Duration::from_secs(5),
        || !part.exists(),
    );
}

/// A completed download lands in the injected data directory — kit and
/// registry entry both — on every platform.
#[test]
fn a_download_installs_into_the_injected_data_dir() {
    let home = DataHome::new("install");
    let worker = home.worker(&start_server());
    worker.send(Command::Download(kit("Good Kit", "good.zip")));
    wait_for("the download to finish", Duration::from_secs(10), || {
        let status = worker.state.lock().status.clone();
        assert!(!matches!(status, Status::Error(_)), "download failed: {status:?}");
        matches!(status, Status::Done(_))
    });
    let dest = home.kits_dir().join("Good_Kit");
    assert_eq!(
        std::fs::read(dest.join("good/kick.wav")).unwrap(),
        vec![7u8; 1000]
    );
    let registry = std::fs::read_to_string(home.0.join("installed.json")).unwrap();
    assert!(
        registry.contains("Good Kit") && registry.contains(&*dest.to_string_lossy()),
        "registry entry missing: {registry}"
    );
    assert!(!part_file(&home.kits_dir(), "Good_Kit").exists());
}

/// The per-read timeout: a server that sends headers and then stalls
/// fails the download once one read has waited that long — not the 30 s
/// default here, a test-sized one — and leaves no `.part` behind.
#[test]
fn a_stalled_body_times_out_and_leaves_no_part() {
    let home = DataHome::new("stall");
    let limit = Duration::from_millis(300);
    let worker = download::spawn_with(WorkerConfig {
        read_timeout: limit,
        ..home.config(&start_server())
    });
    let started = Instant::now();
    worker.send(Command::Download(kit("Stall Kit", "stall.zip")));
    wait_for("the stalled download to fail", Duration::from_secs(10), || {
        matches!(worker.state.lock().status, Status::Error(_))
    });
    let took = started.elapsed();
    assert!(
        took < limit * 10,
        "a {limit:?} read timeout took {took:?} to fire"
    );
    let entries: Vec<_> = std::fs::read_dir(home.kits_dir())
        .map(|d| d.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(entries.is_empty(), "left behind: {entries:?}");
}

/// No thread until the first command: every plugin instance owns a
/// download handle, and most never download anything.
#[test]
fn the_worker_thread_starts_on_the_first_command() {
    use resonance_drums::ResonanceDrums;
    use resonance_plugin::ResonancePlugin;

    let plugin = ResonanceDrums::new();
    assert!(
        !plugin.download_worker_running(),
        "a fresh plugin instance started a download thread"
    );
    drop(plugin);

    // Nothing listens on port 1: the fetch fails at once.
    let worker = download::spawn_with_index("http://127.0.0.1:1/index.json".to_string());
    assert!(!worker.is_running());
    worker.send(Command::FetchIndex);
    assert!(worker.is_running(), "the first command must start the thread");
    wait_for(
        "the refused fetch to fail",
        Duration::from_secs(10),
        || matches!(worker.state.lock().status, Status::Error(_)),
    );
    drop(worker);

    // A handle that never started drops at once, and a lone Shutdown
    // does not start a thread just to stop it.
    let idle = download::spawn();
    idle.send(Command::Shutdown);
    assert!(!idle.is_running());
}
