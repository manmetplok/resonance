//! Background worker thread for the Tone3000 browser.
//!
//! The editor UI never touches the network directly. It pushes commands
//! into a bounded `mpsc::Sender`, the worker drains them one at a time
//! on its own thread, and publishes the results into `Arc<Mutex<State>>`
//! which the UI polls each frame. That keeps the egui thread responsive
//! and, critically, keeps the audio thread completely out of the picture.
//!
//! The worker also owns the "after download, auto-load" handshake: when
//! a download finishes, it rescans the models directory, updates the
//! plugin's `file_list`, and fires `load_request` so the existing amp
//! loader thread picks the new .nam up and primes it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;

use super::auth;
use super::client::{ArchitectureFilter, ClientError, SearchPage, Tone3000Client};
use super::types::{Model, StoredTokens, Tone};

/// Commands accepted by the worker. Each corresponds to one UI action.
pub enum Command {
    /// Try to load persisted tokens from disk. Silently sets
    /// `State::status = Connected` if valid tokens exist.
    TryRestore,
    /// Run the full PKCE / browser dance.
    Authenticate,
    /// Drop in-memory tokens and delete the config file.
    Disconnect,
    /// Search tones from page 1, replacing `State::tones`.
    Search {
        query: String,
        /// `sort` query value (e.g. `trending`, `downloads-all-time`).
        sort: String,
        architecture: ArchitectureFilter,
    },
    /// Fetch the next page of the last search and append it. No-op if
    /// the last page has already been reached.
    LoadMore,
    /// Load the model list for a tone; results go to `State::models`.
    ListModels(i64),
    /// Download a model to disk and queue it for loading.
    Download(Model),
    /// Gracefully stop the worker thread.
    Shutdown,
}

/// The search the worker is currently showing, kept so
/// [`Command::LoadMore`] can repeat it for the next page and
/// [`Command::ListModels`] can use the same architecture filter.
#[derive(Debug, Clone)]
struct ActiveSearch {
    query: String,
    sort: String,
    architecture: ArchitectureFilter,
}

impl Default for ActiveSearch {
    fn default() -> Self {
        Self {
            query: String::new(),
            sort: "trending".to_string(),
            architecture: ArchitectureFilter::default(),
        }
    }
}

/// Connection / activity status. Drives the header of the browser panel
/// so the user always knows what the worker is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Disconnected,
    Authenticating,
    Connected,
    Searching,
    LoadingModels,
    Downloading(String),
    Error(String),
}

impl Status {
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Status::Authenticating
                | Status::Searching
                | Status::LoadingModels
                | Status::Downloading(_)
        )
    }
}

/// State visible to the UI. Everything the egui panel renders from lives
/// behind this one mutex; a frame that picks it up sees a consistent
/// snapshot.
pub struct State {
    pub status: Status,
    pub tones: Vec<Tone>,
    pub selected_tone: Option<i64>,
    pub models: Vec<Model>,
    pub last_error: Option<String>,
    pub last_downloaded: Option<PathBuf>,
    /// Total matching tones the server reports, so the panel can say
    /// "25 of 1 284" instead of implying 25 is all there is.
    pub total_tones: Option<u32>,
    /// Highest page fetched so far (1-based); 0 before the first search.
    pub page: u32,
    /// Whether another page exists — drives the "Load more" button.
    pub has_more: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            status: Status::Disconnected,
            tones: Vec::new(),
            selected_tone: None,
            models: Vec::new(),
            last_error: None,
            last_downloaded: None,
            total_tones: None,
            page: 0,
            has_more: false,
        }
    }
}

/// Fold a fetched page into the browser state.
///
/// `append` is false for a fresh search (replace everything) and true
/// for "Load more" (extend, skipping ids already on screen — the
/// architecture sub-queries and the server's own ranking can both
/// re-serve a row).
///
/// Pure and public so the pagination rules are testable without a
/// network.
pub fn apply_search_page(state: &mut State, page: SearchPage, append: bool) {
    if !append {
        state.tones.clear();
        state.models.clear();
        state.selected_tone = None;
    }
    let fetched = page.tones.len();
    for tone in page.tones {
        if state.tones.iter().any(|t| t.id == tone.id) {
            continue;
        }
        state.tones.push(tone);
    }
    state.page = page.page;
    state.total_tones = page.total;
    state.has_more = match page.total_pages {
        // The server told us how far the result set goes.
        Some(total_pages) => page.page < total_pages,
        // No `total_pages`: fall back on the running count against the
        // reported total, and failing that assume a page that came back
        // empty is the end.
        None => match page.total {
            Some(total) => (state.tones.len() as u32) < total,
            None => fetched > 0,
        },
    };
}

/// Shared handles the worker needs to reach into to hand off a freshly
/// downloaded model. These come from the plugin's own state so the
/// download → load path reuses the existing loader thread wholesale.
pub struct PluginHooks {
    pub file_list: Arc<Mutex<Vec<String>>>,
    pub model_path: Arc<Mutex<String>>,
    pub load_request: Arc<AtomicI32>,
    pub file_select_setter: Arc<dyn Fn(i32) + Send + Sync>,
}

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

pub fn spawn(hooks: PluginHooks) -> WorkerHandle {
    let (tx, rx) = mpsc::channel();
    let state = Arc::new(Mutex::new(State::default()));
    let state_for_thread = state.clone();

    let join = std::thread::Builder::new()
        .name("amp-tone3000".into())
        .spawn(move || worker_loop(rx, state_for_thread, hooks))
        .expect("spawn amp-tone3000 worker");

    // Kick off a token-restore attempt immediately so a returning user
    // sees "Connected" without having to click anything.
    let _ = tx.send(Command::TryRestore);

    WorkerHandle {
        tx,
        state,
        join: Some(join),
    }
}

fn worker_loop(rx: Receiver<Command>, state: Arc<Mutex<State>>, hooks: PluginHooks) {
    let client = Tone3000Client::new();
    let mut tokens: Option<StoredTokens> = None;
    let mut active = ActiveSearch::default();

    loop {
        let cmd = match rx.recv() {
            Ok(c) => c,
            Err(_) => return,
        };
        match cmd {
            Command::Shutdown => return,
            Command::TryRestore => {
                if let Some(t) = auth::load_tokens() {
                    tokens = Some(t);
                    state.lock().status = Status::Connected;
                }
            }
            Command::Authenticate => {
                state.lock().status = Status::Authenticating;
                match auth::run_authorization_flow(Duration::from_secs(180)) {
                    Ok(t) => {
                        tokens = Some(t);
                        let mut s = state.lock();
                        s.status = Status::Connected;
                        s.last_error = None;
                    }
                    Err(e) => {
                        let mut s = state.lock();
                        s.status = Status::Error(e.clone());
                        s.last_error = Some(e);
                    }
                }
            }
            Command::Disconnect => {
                auth::clear_tokens();
                tokens = None;
                let mut s = state.lock();
                s.status = Status::Disconnected;
                s.tones.clear();
                s.models.clear();
                s.selected_tone = None;
                s.total_tones = None;
                s.page = 0;
                s.has_more = false;
            }
            Command::Search {
                query,
                sort,
                architecture,
            } => {
                active = ActiveSearch {
                    query,
                    sort,
                    architecture,
                };
                let Some(tok) = ensure_valid_token(&mut tokens, &state) else {
                    continue;
                };
                state.lock().status = Status::Searching;
                fetch_page(&client, &tok, &active, 1, false, &mut tokens, &state);
            }
            Command::LoadMore => {
                let next_page = {
                    let s = state.lock();
                    if !s.has_more {
                        continue;
                    }
                    s.page + 1
                };
                let Some(tok) = ensure_valid_token(&mut tokens, &state) else {
                    continue;
                };
                state.lock().status = Status::Searching;
                fetch_page(&client, &tok, &active, next_page, true, &mut tokens, &state);
            }
            Command::ListModels(tone_id) => {
                let Some(tok) = ensure_valid_token(&mut tokens, &state) else {
                    continue;
                };
                state.lock().status = Status::LoadingModels;
                match client.list_models(&tok, tone_id, active.architecture) {
                    Ok(models) => {
                        let mut s = state.lock();
                        s.models = models;
                        s.selected_tone = Some(tone_id);
                        s.status = Status::Connected;
                    }
                    Err(ClientError::Unauthorized) => handle_unauthorized(&mut tokens, &state),
                    Err(e) => set_error(&state, &format!("list models: {e}")),
                }
            }
            Command::Download(model) => {
                let Some(tok) = ensure_valid_token(&mut tokens, &state) else {
                    continue;
                };
                let Some(url) = model.model_url.clone() else {
                    set_error(&state, "model has no download URL");
                    continue;
                };
                state.lock().status = Status::Downloading(model.display_label());

                match client.download_model(&tok, &url) {
                    Ok(bytes) => {
                        if let Err(e) = finalize_download(&model, &bytes, &hooks, &state) {
                            set_error(&state, &e);
                        } else {
                            state.lock().status = Status::Connected;
                        }
                    }
                    Err(ClientError::Unauthorized) => handle_unauthorized(&mut tokens, &state),
                    Err(e) => set_error(&state, &format!("download: {e}")),
                }
            }
        }
    }
}

/// Run one search request and fold the result into the shared state.
/// Shared by the initial search and by "Load more" — the only
/// difference between them is `append` and the page number.
fn fetch_page(
    client: &Tone3000Client,
    token: &str,
    active: &ActiveSearch,
    page: u32,
    append: bool,
    tokens: &mut Option<StoredTokens>,
    state: &Arc<Mutex<State>>,
) {
    match client.search_tones(
        token,
        &active.query,
        &active.sort,
        active.architecture,
        page,
    ) {
        Ok(result) => {
            let mut s = state.lock();
            apply_search_page(&mut s, result, append);
            s.status = Status::Connected;
        }
        Err(ClientError::Unauthorized) => handle_unauthorized(tokens, state),
        Err(e) => set_error(state, &format!("search failed: {e}")),
    }
}

/// Ensure we have a non-expired access token, refreshing if needed.
/// Returns the bearer string, or `None` and sets an error if we can't
/// produce one (in which case the command should be skipped).
fn ensure_valid_token(
    tokens: &mut Option<StoredTokens>,
    state: &Arc<Mutex<State>>,
) -> Option<String> {
    let current = tokens.clone()?;
    if !current.is_expired() {
        return Some(current.access_token);
    }
    let Some(rt) = current.refresh_token.as_deref() else {
        set_error(state, "token expired and no refresh token — reconnect");
        state.lock().status = Status::Disconnected;
        return None;
    };
    match auth::refresh_tokens(rt) {
        Ok(fresh) => {
            let access = fresh.access_token.clone();
            *tokens = Some(fresh);
            Some(access)
        }
        Err(e) => {
            set_error(state, &format!("refresh failed: {e}"));
            state.lock().status = Status::Disconnected;
            None
        }
    }
}

fn handle_unauthorized(tokens: &mut Option<StoredTokens>, state: &Arc<Mutex<State>>) {
    // Treat 401 as "stale token". If we have a refresh token, retrying
    // through `ensure_valid_token` will pick it up next time; if not,
    // drop to disconnected so the user knows to reauth.
    if let Some(t) = tokens {
        // Force expiry so the next `ensure_valid_token` call refreshes.
        t.expires_in = Some(0);
        t.acquired_at = 0;
    }
    let mut s = state.lock();
    s.status = Status::Error("session expired — try again".to_string());
}

fn set_error(state: &Arc<Mutex<State>>, msg: &str) {
    let mut s = state.lock();
    s.last_error = Some(msg.to_string());
    s.status = Status::Error(msg.to_string());
}

/// Write the downloaded bytes to the models dir and trigger the amp
/// loader to swap the new file in.
fn finalize_download(
    model: &Model,
    bytes: &[u8],
    hooks: &PluginHooks,
    state: &Arc<Mutex<State>>,
) -> Result<(), String> {
    let dir = super::models_dir().ok_or_else(|| "no data dir".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;

    let filename = sanitize_filename(&model.display_label(), model.id);
    let dest = dir.join(filename);
    auth::write_all_to(&dest, bytes)?;

    // Rescan the directory so file_list reflects the new file, then
    // point file_select + load_request at it so the existing loader
    // thread picks it up and primes it like any manual load would.
    let files = resonance_common::scan_directory(&dir, "nam");
    let dest_str = dest.to_string_lossy().into_owned();
    let idx = files.iter().position(|f| f == &dest_str).unwrap_or(0) as i32;

    *hooks.file_list.lock() = files;
    *hooks.model_path.lock() = dest_str.clone();
    (hooks.file_select_setter)(idx);
    hooks.load_request.store(idx, Ordering::Release);

    state.lock().last_downloaded = Some(dest);
    Ok(())
}

fn sanitize_filename(label: &str, id: i64) -> String {
    // Conservative: keep ASCII alphanumerics, `-`, `_`, `.`; replace the
    // rest with `_`. Always append the model id so downloads with the
    // same label don't collide.
    let mut out = String::with_capacity(label.len() + 12);
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("model");
    }
    format!("{out}_{id}.nam")
}
