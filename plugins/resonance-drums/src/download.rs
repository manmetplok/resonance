//! Drumkit download worker: fetches the server index, downloads kit zips
//! with progress, and extracts them to `$XDG_DATA_HOME/resonance/drumkits/`.
//!
//! Follows the same architecture as the amp plugin's Tone3000 worker:
//! - The editor pushes [`Command`]s via an `mpsc::Sender`.
//! - The worker drains them on a dedicated thread.
//! - Shared [`State`] behind `Arc<Mutex<…>>` is polled by the UI each frame.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

use parking_lot::Mutex;
use serde::Deserialize;

use resonance_common::registry::{self, ContentType, InstalledItem};

// ---------------------------------------------------------------------------
// Public constants
// ---------------------------------------------------------------------------

/// Base URL of the drumkit distribution server.
const INDEX_URL: &str = "https://resonance.plok.org/index.json";

/// Subdirectory under the user's data dir where extracted kits live.
const DRUMKITS_SUBDIR: &str = "resonance/drumkits";

/// Resolve the per-user directory where drumkits are stored.
pub fn drumkits_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(DRUMKITS_SUBDIR))
}

// ---------------------------------------------------------------------------
// Server index types
// ---------------------------------------------------------------------------

/// Top-level response from the index endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerIndex {
    #[serde(default)]
    pub drumkits: Vec<ServerKit>,
}

/// One kit available for download.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct ServerKit {
    pub name: String,
    pub file: String,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub added: Option<String>,
}

// ---------------------------------------------------------------------------
// Worker protocol
// ---------------------------------------------------------------------------

/// Commands the UI pushes to the worker.
pub enum Command {
    /// Fetch the server index.
    FetchIndex,
    /// Download and extract a kit.
    Download(ServerKit),
    /// Gracefully stop the worker thread.
    Shutdown,
}

/// Current activity, surfaced in the UI.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Status {
    Idle,
    FetchingIndex,
    Downloading {
        name: String,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Extracting(String),
    Done(String),
    Error(String),
}

impl Status {
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Status::FetchingIndex | Status::Downloading { .. } | Status::Extracting(_)
        )
    }
}

/// Shared state the UI reads each frame.
pub struct State {
    pub status: Status,
    pub index: Option<ServerIndex>,
    pub last_error: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            status: Status::Idle,
            index: None,
            last_error: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Worker handle
// ---------------------------------------------------------------------------

pub struct WorkerHandle {
    tx: Sender<Command>,
    pub state: Arc<Mutex<State>>,
    join: Option<JoinHandle<()>>,
}

impl WorkerHandle {
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

pub fn spawn() -> WorkerHandle {
    let (tx, rx) = mpsc::channel();
    let state = Arc::new(Mutex::new(State::default()));
    let state_for_thread = state.clone();

    let join = std::thread::Builder::new()
        .name("drums-download".into())
        .spawn(move || worker_loop(rx, state_for_thread))
        .expect("spawn drums-download worker");

    WorkerHandle {
        tx,
        state,
        join: Some(join),
    }
}

// ---------------------------------------------------------------------------
// Worker loop
// ---------------------------------------------------------------------------

fn worker_loop(rx: Receiver<Command>, state: Arc<Mutex<State>>) {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        // No global read timeout — large downloads need more time.
        .build();

    loop {
        let cmd = match rx.recv() {
            Ok(c) => c,
            Err(_) => return,
        };
        match cmd {
            Command::Shutdown => return,
            Command::FetchIndex => {
                state.lock().status = Status::FetchingIndex;
                match fetch_index(&agent) {
                    Ok(index) => {
                        let mut s = state.lock();
                        s.index = Some(index);
                        s.status = Status::Idle;
                        s.last_error = None;
                    }
                    Err(e) => set_error(&state, &e),
                }
            }
            Command::Download(kit) => {
                state.lock().status = Status::Downloading {
                    name: kit.name.clone(),
                    downloaded_bytes: 0,
                    total_bytes: 0,
                };
                match download_and_extract(&agent, &kit, &state) {
                    Ok(dest) => {
                        // Mark in the shared registry.
                        let _ = registry::mark_installed(InstalledItem {
                            name: kit.name.clone(),
                            content_type: ContentType::Drumkit,
                            path: dest.to_string_lossy().into_owned(),
                            installed_at: registry::today_iso(),
                        });
                        let mut s = state.lock();
                        s.status = Status::Done(kit.name.clone());
                        s.last_error = None;
                    }
                    Err(e) => set_error(&state, &e),
                }
            }
        }
    }
}

fn set_error(state: &Arc<Mutex<State>>, msg: &str) {
    let mut s = state.lock();
    s.last_error = Some(msg.to_string());
    s.status = Status::Error(msg.to_string());
}

// ---------------------------------------------------------------------------
// Network helpers
// ---------------------------------------------------------------------------

fn fetch_index(agent: &ureq::Agent) -> Result<ServerIndex, String> {
    let resp = agent
        .get(INDEX_URL)
        .call()
        .map_err(|e| format!("fetch index: {e}"))?;
    let index: ServerIndex = resp.into_json().map_err(|e| format!("parse index: {e}"))?;
    Ok(index)
}

/// Stream-download the kit zip, then extract it.
fn download_and_extract(
    agent: &ureq::Agent,
    kit: &ServerKit,
    state: &Arc<Mutex<State>>,
) -> Result<PathBuf, String> {
    // Build the download URL relative to the index URL base.
    let base = INDEX_URL
        .rsplit_once('/')
        .map(|(base, _)| base)
        .unwrap_or(INDEX_URL);
    let url = format!("{base}/{}", kit.file);

    let resp = agent
        .get(&url)
        .call()
        .map_err(|e| format!("download {}: {e}", kit.name))?;

    let total: u64 = resp
        .header("Content-Length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Stream to a temporary file so we don't hold GiBs in RAM.
    let dir = drumkits_dir().ok_or_else(|| "no data dir".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;

    let tmp_path = dir.join(format!(".{}.zip.part", sanitize(&kit.name)));
    let mut tmp_file =
        std::fs::File::create(&tmp_path).map_err(|e| format!("create temp file: {e}"))?;

    // Read in 256 KiB chunks, updating progress.
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut downloaded: u64 = 0;

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("read body: {e}"))?;
        if n == 0 {
            break;
        }
        tmp_file
            .write_all(&buf[..n])
            .map_err(|e| format!("write temp file: {e}"))?;
        downloaded += n as u64;

        state.lock().status = Status::Downloading {
            name: kit.name.clone(),
            downloaded_bytes: downloaded,
            total_bytes: total,
        };
    }
    drop(tmp_file);

    // Extract the zip.
    state.lock().status = Status::Extracting(kit.name.clone());

    let dest = dir.join(sanitize(&kit.name));
    extract_zip(&tmp_path, &dest)?;

    // Clean up the temp file.
    let _ = std::fs::remove_file(&tmp_path);

    Ok(dest)
}

fn extract_zip(zip_path: &PathBuf, dest: &PathBuf) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("open zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("read zip: {e}"))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip entry {i}: {e}"))?;

        let out_path = match entry.enclosed_name() {
            Some(name) => dest.join(name),
            None => continue, // skip entries with suspicious paths
        };

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("mkdir {}: {e}", out_path.display()))?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
            }
            let mut out_file = std::fs::File::create(&out_path)
                .map_err(|e| format!("create {}: {e}", out_path.display()))?;
            std::io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("extract {}: {e}", out_path.display()))?;
        }
    }
    Ok(())
}

/// Conservative filename sanitizer: keep ASCII alphanumerics, `-`, `_`, `.`;
/// replace whitespace with `_`, drop everything else.
fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("kit");
    }
    out
}
