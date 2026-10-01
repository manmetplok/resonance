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
        // Several entries, so an extraction can be stopped half-way.
        "/many.zip" => send_body(&mut stream, &zip_of(&many_entries())),
        // Downloads fine; its second entry fails its CRC on extraction.
        "/corrupt.zip" => send_body(&mut stream, &corrupt_zip()),
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

fn many_entries() -> Vec<(String, Vec<u8>)> {
    (0..6)
        .map(|i| (format!("many/file{i}.wav"), vec![i as u8; 2000]))
        .collect()
}

/// A zip whose first entry is fine and whose second has one byte of its
/// (stored) data flipped, so reading it fails the CRC check.
fn corrupt_zip() -> Vec<u8> {
    let marker = vec![0xABu8; 2000];
    let mut bytes = zip_of(&[
        ("bad/first.wav".to_string(), vec![1u8; 2000]),
        ("bad/second.wav".to_string(), marker.clone()),
        ("bad/third.wav".to_string(), vec![3u8; 2000]),
    ]);
    let at = bytes
        .windows(marker.len())
        .position(|w| w == marker.as_slice())
        .expect("stored data is in the archive verbatim");
    bytes[at + 1000] ^= 0xFF;
    bytes
}

/// Every name directly in `dir`.
fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
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

/// The `.part` files downloads of a kit named `sanitized` are streaming
/// into: `.<sanitized>.<pid>-<n>.zip.part`.
fn part_files(dir: &Path, sanitized: &str) -> Vec<PathBuf> {
    let prefix = format!(".{sanitized}.");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with(&prefix) && name.ends_with(".zip.part")
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    out
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
        part_files(&kits_dir, "Broken_Kit").is_empty(),
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
    let parts = part_files(&kits_dir, "Slow_Kit");
    assert_eq!(parts.len(), 1, "the transfer should be streaming into a .part");
    let part = parts[0].clone();

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

/// Two workers (two plugin instances) downloading the same kit into the
/// same directory used to share `.<Kit>.zip.part`: both wrote into one
/// file, and the first to finish or fail deleted it under the other.
#[test]
fn two_downloads_of_one_kit_stream_into_separate_part_files() {
    let home = DataHome::new("twins");
    let index_url = start_server();
    let a = home.worker(&index_url);
    let b = home.worker(&index_url);
    a.send(Command::Download(kit("Slow Kit", "slow.zip")));
    b.send(Command::Download(kit("Slow Kit", "slow.zip")));
    let streaming = |w: &WorkerHandle| {
        matches!(
            w.state.lock().status,
            Status::Downloading { downloaded_bytes, .. } if downloaded_bytes > 0
        )
    };
    wait_for("both downloads to stream", Duration::from_secs(10), || {
        streaming(&a) && streaming(&b)
    });
    let parts = part_files(&home.kits_dir(), "Slow_Kit");
    assert_eq!(parts.len(), 2, "both downloads share one file: {parts:?}");

    // Cancelling one leaves the other's file alone.
    drop(a);
    wait_for("a's .part to go", Duration::from_secs(5), || {
        part_files(&home.kits_dir(), "Slow_Kit").len() == 1
    });
    assert!(streaming(&b), "b must still be downloading");
    drop(b);
    wait_for("b's .part to go", Duration::from_secs(5), || {
        part_files(&home.kits_dir(), "Slow_Kit").is_empty()
    });
}

/// Dropping the handle in the middle of an extraction stops it at the
/// next entry and leaves neither a half-extracted kit nor the `.part`.
/// The extraction used to run to the end whatever the cancel flag said —
/// a detached worker then finished installing a kit the user had walked
/// away from.
#[test]
fn a_cancel_mid_extraction_stops_and_leaves_nothing() {
    use std::sync::Arc;

    let home = DataHome::new("extract-cancel");
    let (at_tx, at_rx) = crossbeam_channel::bounded::<()>(1);
    let (go_tx, go_rx) = crossbeam_channel::bounded::<()>(1);
    let hook = download::ExtractHook(Arc::new(move |i| {
        if i == 2 {
            let _ = at_tx.send(());
            let _ = go_rx.recv_timeout(Duration::from_secs(10));
        }
    }));
    let worker = download::spawn_with(WorkerConfig {
        on_extract_entry: Some(hook),
        ..home.config(&start_server())
    });
    let state = worker.state.clone();
    worker.send(Command::Download(kit("Many Kit", "many.zip")));
    at_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the extraction never reached entry 2");
    // Two entries are out, in the staging directory.
    let kits_dir = home.kits_dir();
    assert!(
        listing(&kits_dir).iter().any(|n| n.ends_with(".extracting")),
        "extraction should be under way: {:?}",
        listing(&kits_dir)
    );

    drop(worker);
    let _ = go_tx.send(());
    wait_for("the cancelled extraction to clean up", Duration::from_secs(10), || {
        listing(&kits_dir).is_empty()
    });
    wait_for("the worker to report the cancel", Duration::from_secs(10), || {
        matches!(state.lock().status, Status::Error(_))
    });
    assert!(
        !home.0.join("installed.json").exists(),
        "a cancelled kit was recorded as installed"
    );
}

/// An extraction that fails part-way removes what it wrote, and a kit
/// already installed under that name survives the failed re-download.
#[test]
fn a_failed_extraction_leaves_no_partial_kit_and_keeps_the_installed_one() {
    let home = DataHome::new("extract-fail");
    let kits_dir = home.kits_dir();
    let installed = kits_dir.join("Bad_Kit");
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::write(installed.join("old.wav"), b"the kit that was there").unwrap();

    let worker = home.worker(&start_server());
    worker.send(Command::Download(kit("Bad Kit", "corrupt.zip")));
    wait_for("the extraction to fail", Duration::from_secs(10), || {
        matches!(worker.state.lock().status, Status::Error(_))
    });
    drop(worker);
    assert_eq!(
        listing(&kits_dir),
        vec!["Bad_Kit".to_string()],
        "a failed extraction left something behind"
    );
    assert_eq!(listing(&installed), vec!["old.wav".to_string()]);
}

/// A worker starting up removes download leftovers nobody can own: those
/// of a process that is gone (Linux can tell) and any untouched for an
/// hour — never this process's own, nor a fresh one of unknown owner.
#[test]
fn a_starting_worker_sweeps_stale_download_leftovers() {
    let home = DataHome::new("sweep");
    let kits_dir = home.kits_dir();
    std::fs::create_dir_all(&kits_dir).unwrap();
    let me = std::process::id();
    // No pid ever reaches 999999999 (Linux caps pid_max at 2^22).
    let dead_part = kits_dir.join(".Dead.999999999-0.zip.part");
    let dead_staging = kits_dir.join(".Dead.999999999-1.extracting");
    let mine = kits_dir.join(format!(".Mine.{me}-77.zip.part"));
    let old_legacy = kits_dir.join(".Old.zip.part");
    let fresh_legacy = kits_dir.join(".Fresh.zip.part");
    let unrelated = kits_dir.join("notes.txt");
    for f in [&dead_part, &mine, &old_legacy, &fresh_legacy, &unrelated] {
        std::fs::write(f, b"x").unwrap();
    }
    std::fs::create_dir_all(dead_staging.join("inner")).unwrap();
    let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(2 * 3600);
    std::fs::File::options()
        .write(true)
        .open(&old_legacy)
        .unwrap()
        .set_modified(two_hours_ago)
        .unwrap();

    // Any command starts the worker; the index fetch 404s.
    let worker = home.worker(&start_server());
    worker.send(Command::FetchIndex);
    wait_for("the fetch to finish", Duration::from_secs(10), || {
        matches!(worker.state.lock().status, Status::Error(_))
    });

    assert!(!old_legacy.exists(), "an hour-old leftover survived");
    assert!(mine.exists(), "this process's own download was swept");
    assert!(fresh_legacy.exists(), "a fresh leftover of unknown owner was swept");
    assert!(unrelated.exists());
    if cfg!(target_os = "linux") {
        assert!(!dead_part.exists(), "a dead process's .part survived");
        assert!(!dead_staging.exists(), "a dead process's staging dir survived");
    }
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
    assert!(part_files(&home.kits_dir(), "Good_Kit").is_empty());
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
