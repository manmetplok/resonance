//! Kit download worker against a local HTTP server
//! (drums-plugin-rework.md §1.1, §9).
//!
//! `WorkerHandle::drop` used to join the worker thread, which only read
//! its Shutdown command between downloads: closing a project during a
//! 5 GiB download blocked until the download finished. A failed or
//! abandoned download also left its `.part` file behind.
//!
//! One `#[test]` on purpose: the download directory comes from
//! `$XDG_DATA_HOME`, which is process-global, so this binary points it
//! at a temp dir once and runs every phase in sequence.

#![cfg(feature = "editor")]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use resonance_drums::download::{self, Command, ServerKit, Status};

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
        _ => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        }
    }
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

#[test]
fn download_worker_cancels_cleans_up_and_never_blocks_drop() {
    let data_home =
        std::env::temp_dir().join(format!("resonance-drums-download-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_home);
    std::fs::create_dir_all(&data_home).unwrap();
    // Before anything resolves a data dir, so the real one is never
    // touched. Process-global — hence the single test in this binary.
    std::env::set_var("XDG_DATA_HOME", &data_home);
    let kits_dir = download::drumkits_dir().expect("drumkits dir");
    assert!(
        kits_dir.starts_with(&data_home),
        "download dir {} escaped the test's data home",
        kits_dir.display()
    );

    let index_url = start_server();

    // --- A transfer the server cuts short fails and leaves no .part ---
    let worker = download::spawn_with_index(index_url.clone());
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
    let worker = download::spawn_with_index(index_url);
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

    let _ = std::fs::remove_dir_all(&data_home);
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
