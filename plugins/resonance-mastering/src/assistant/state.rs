//! The [`Assistant`] facade: owns the capture ring and the latest
//! analysis/suggestion/reference state behind mutexes, and exposes the
//! audio-thread feed plus the UI-thread analyze/apply entry points.
//!
//! It also owns the user's **target choice** — genre or reference, which
//! genre, which reference file — and persists it with the plugin's state
//! under [`STATE_KEY`] ([`AssistantStateSaver`]), so reopening a project
//! brings back the target the user was mastering against
//! (warmth-width-depth.md §7.4; plugin-audit finding). The assistant
//! processes no audio, so this state never changes what the plugin
//! renders.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use super::analyze::{self, AnalysisResult};
use super::capture::CaptureBuffer;
use super::decide::{self, Suggestions, Target};
use super::reference::{self, ReferenceTrack};
use super::targets::Genre;
use crate::viz::MasteringViz;

/// Top-level key of the assistant's entry in the plugin state JSON.
pub const STATE_KEY: &str = "assistant";

/// What the assistant compares against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetMode {
    /// A built-in genre band (the default).
    #[default]
    Genre,
    /// The loaded reference track.
    Reference,
}

impl TargetMode {
    fn id(self) -> &'static str {
        match self {
            TargetMode::Genre => "genre",
            TargetMode::Reference => "reference",
        }
    }

    fn from_id(s: &str) -> Option<Self> {
        match s {
            "genre" => Some(TargetMode::Genre),
            "reference" => Some(TargetMode::Reference),
            _ => None,
        }
    }
}

/// The persisted part of the assistant: the user's target choice.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AssistantSettings {
    pub mode: TargetMode,
    pub genre: Genre,
    /// Path of the reference file, as the user gave it. Empty for none.
    pub reference_path: String,
}

/// Capture duration in seconds. The research brief explicitly calls
/// for "play ~10 seconds, run analysis".
pub const CAPTURE_SECONDS: f32 = 10.0;

/// Background reference decodes of one assistant: at most one runs, and
/// only the newest request waits behind it (see [`AssistantStateSaver`]).
#[derive(Default)]
struct ReloadQueue {
    /// The newest path waiting for the decode thread.
    pending: Option<String>,
    /// True while a decode thread is alive.
    running: bool,
}

/// Lock order: `settings` before `reference` / `reference_error`; the
/// reference slots are only written with `settings` held, so the path
/// check and the install are one step.
pub struct Assistant {
    capture: Mutex<CaptureBuffer>,
    last_analysis: Mutex<Option<AnalysisResult>>,
    last_suggestions: Mutex<Option<Suggestions>>,
    reference: Mutex<Option<ReferenceTrack>>,
    reference_error: Mutex<Option<String>>,
    settings: Mutex<AssistantSettings>,
    reload: Mutex<ReloadQueue>,
    /// Decode threads alive now, and the most ever alive at once
    /// (diagnostics: the latter must never pass 1).
    decode_threads: AtomicUsize,
    decode_threads_peak: AtomicUsize,
}

impl Assistant {
    pub fn new(sample_rate: f32) -> Self {
        let capacity = (CAPTURE_SECONDS * sample_rate) as usize;
        Self {
            capture: Mutex::new(CaptureBuffer::new(capacity, sample_rate)),
            last_analysis: Mutex::new(None),
            last_suggestions: Mutex::new(None),
            reference: Mutex::new(None),
            reference_error: Mutex::new(None),
            settings: Mutex::new(AssistantSettings::default()),
            reload: Mutex::new(ReloadQueue::default()),
            decode_threads: AtomicUsize::new(0),
            decode_threads_peak: AtomicUsize::new(0),
        }
    }

    /// The user's current target choice.
    pub fn settings(&self) -> AssistantSettings {
        self.settings.lock().clone()
    }

    pub fn set_mode(&self, mode: TargetMode) {
        self.settings.lock().mode = mode;
    }

    pub fn set_genre(&self, genre: Genre) {
        self.settings.lock().genre = genre;
    }

    /// Remember `path` as the reference file (without loading it).
    pub fn set_reference_path(&self, path: &str) {
        self.settings.lock().reference_path = path.to_string();
    }

    /// The target the current settings name: the chosen genre, or the
    /// loaded reference in reference mode (falling back to the genre while
    /// no reference is loaded).
    pub fn current_target(&self) -> Target {
        let settings = self.settings();
        match (settings.mode, self.reference()) {
            (TargetMode::Reference, Some(r)) => Target::Reference(r),
            _ => Target::Genre(settings.genre),
        }
    }

    /// The settings as their state-JSON entry, or `None` when they are
    /// all at their defaults — an untouched assistant adds no key, so a
    /// state blob saved without one reads back exactly the same.
    pub fn save_state(&self) -> Option<serde_json::Value> {
        let settings = self.settings();
        if settings == AssistantSettings::default() {
            return None;
        }
        let mut entry = serde_json::Map::new();
        entry.insert("mode".into(), settings.mode.id().into());
        entry.insert("genre".into(), settings.genre.id().into());
        if !settings.reference_path.is_empty() {
            entry.insert("reference_path".into(), settings.reference_path.into());
        }
        Some(serde_json::Value::Object(entry))
    }

    /// Restore the settings from a whole state JSON object. A missing
    /// entry, or a missing or unknown field in it, restores that setting's
    /// default — so state from before this existed loads as a fresh
    /// assistant. Returns the reference path to re-load, if any; the
    /// decode itself is the caller's (see [`AssistantStateSaver`]).
    pub fn load_state(&self, state: &serde_json::Value) -> Option<String> {
        let entry = state.get(STATE_KEY);
        let field = |k: &str| entry.and_then(|e| e.get(k)).and_then(|v| v.as_str());
        let restored = AssistantSettings {
            mode: field("mode").and_then(TargetMode::from_id).unwrap_or_default(),
            genre: field("genre").and_then(Genre::from_id).unwrap_or_default(),
            reference_path: field("reference_path").unwrap_or_default().to_string(),
        };
        let path = restored.reference_path.clone();
        // A decode still waiting belongs to the previous state.
        self.reload.lock().pending = None;
        let mut settings = self.settings.lock();
        *settings = restored;
        // Whatever reference was loaded belonged to the previous state.
        *self.reference.lock() = None;
        *self.reference_error.lock() = None;
        drop(settings);
        (!path.is_empty()).then_some(path)
    }

    /// Decode `path` as the reference, but install it only if it is
    /// still the configured reference path once the decode finishes — a
    /// restore that was superseded while decoding changes nothing.
    pub fn reload_reference_if_current(&self, path: &str) {
        let result = reference::load_from_path(path);
        self.install_if_current(path, result);
    }

    /// Install a decode of `path`, unless the configured path has moved
    /// on meanwhile. The check and the install happen under the settings
    /// lock, so a restore or a Load that changes the path lands either
    /// before (and this is dropped) or after (and clears or replaces it).
    fn install_if_current(&self, path: &str, result: Result<ReferenceTrack, String>) -> bool {
        let settings = self.settings.lock();
        if settings.reference_path != path {
            return false;
        }
        match result {
            Ok(track) => {
                *self.reference.lock() = Some(track);
                *self.reference_error.lock() = None;
            }
            Err(e) => {
                *self.reference.lock() = None;
                *self.reference_error.lock() = Some(e);
            }
        }
        true
    }

    /// Queue `path` for the background decode. Returns true when the
    /// caller must start the decode thread (none is running); otherwise
    /// the running one picks it up next, and it replaces any path that
    /// was still waiting.
    fn queue_reload(&self, path: String) -> bool {
        let mut q = self.reload.lock();
        q.pending = Some(path);
        !std::mem::replace(&mut q.running, true)
    }

    /// The decode thread's next path, or `None` (and the thread is
    /// marked gone, under the same lock) when nothing waits.
    fn next_reload(&self) -> Option<String> {
        let mut q = self.reload.lock();
        let next = q.pending.take();
        if next.is_none() {
            q.running = false;
        }
        next
    }

    /// The decode thread's loop: decode what is queued until nothing is.
    fn run_reloads(&self) {
        let alive = self.decode_threads.fetch_add(1, Ordering::SeqCst) + 1;
        self.decode_threads_peak.fetch_max(alive, Ordering::SeqCst);
        while let Some(path) = self.next_reload() {
            self.reload_reference_if_current(&path);
        }
        self.decode_threads.fetch_sub(1, Ordering::SeqCst);
    }

    /// The most background reference decodes that ever ran at once
    /// (diagnostics).
    pub fn peak_decode_threads(&self) -> usize {
        self.decode_threads_peak.load(Ordering::SeqCst)
    }

    /// True while a background reference decode is running or queued.
    pub fn reload_in_flight(&self) -> bool {
        self.reload.lock().running
    }

    pub fn set_sample_rate(&self, sample_rate: f32) {
        let expected = (CAPTURE_SECONDS * sample_rate) as usize;
        let mut cap = self.capture.lock();
        if cap.capacity() != expected {
            *cap = CaptureBuffer::new(expected, sample_rate);
        } else {
            cap.set_sample_rate(sample_rate);
        }
    }

    /// Audio-thread hot path: append a stereo block to the capture ring.
    ///
    /// Uses `try_lock` to guarantee the audio thread never blocks waiting
    /// on the UI's `snapshot_chrono` (which clones two ~480 k-sample
    /// vecs and can take milliseconds — longer than one audio block).
    /// When the lock is held by the UI, the block is dropped; the ring
    /// loses at most a few hundred milliseconds across a snapshot call,
    /// which is acceptable for a 10-second offline-analysis window.
    pub fn feed(&self, left: &[f32], right: &[f32]) {
        if let Some(mut cap) = self.capture.try_lock() {
            cap.push(left, right);
        }
    }

    /// How much of the capture ring currently holds real audio,
    /// as a fraction from 0.0 (empty) to 1.0 (full).
    pub fn capture_fraction(&self) -> f32 {
        let c = self.capture.lock();
        c.filled() as f32 / c.capacity().max(1) as f32
    }

    /// Run analysis on the current ring contents against the given
    /// target. Returns `None` if the ring holds less than 2 seconds
    /// of audio (not enough for a stable LUFS-I gated measurement).
    pub fn analyze(&self, target: Target) -> Option<Suggestions> {
        let (l, r, sr) = {
            let cap = self.capture.lock();
            let min_samples = (cap.sample_rate() * 2.0) as usize;
            if cap.filled() < min_samples {
                return None;
            }
            let sr = cap.sample_rate();
            let (l, r) = cap.snapshot_chrono();
            (l, r, sr)
        };

        let analysis = analyze::run(sr, &l, &r);
        let suggestions = decide::build(&analysis, &target);
        *self.last_analysis.lock() = Some(analysis);
        *self.last_suggestions.lock() = Some(suggestions.clone());
        Some(suggestions)
    }

    /// Load a reference track from disk. On success, stores the
    /// decoded track so the next `analyze` call can target it. On
    /// failure, stores the error for the UI to display.
    ///
    /// A restore that changes the path while this decodes wins: the
    /// decode is then dropped (and reported as `Ok`, as nothing failed).
    pub fn load_reference(&self, path: &str) -> Result<(), String> {
        self.set_reference_path(path);
        let result = reference::load_from_path(path);
        let error = result.as_ref().err().cloned();
        match (self.install_if_current(path, result), error) {
            (true, Some(e)) => Err(e),
            _ => Ok(()),
        }
    }

    pub fn reference(&self) -> Option<ReferenceTrack> {
        self.reference.lock().clone()
    }

    pub fn reference_error(&self) -> Option<String> {
        self.reference_error.lock().clone()
    }

    /// Unload the reference. The configured path is kept, so a Load
    /// brings the same file back; [`Self::set_reference_path`] with `""`
    /// forgets it.
    pub fn clear_reference(&self) {
        let _settings = self.settings.lock();
        *self.reference.lock() = None;
        *self.reference_error.lock() = None;
    }

    pub fn last_analysis(&self) -> Option<AnalysisResult> {
        self.last_analysis.lock().clone()
    }

    pub fn last_suggestions(&self) -> Option<Suggestions> {
        self.last_suggestions.lock().clone()
    }

    pub fn clear(&self) {
        self.capture.lock().clear();
        *self.last_analysis.lock() = None;
        *self.last_suggestions.lock() = None;
    }
}

impl Assistant {
    /// Install a pre-constructed reference track directly, without
    /// hitting the filesystem. Used by integration tests that want to
    /// exercise the reference-based analysis path with a synthetic
    /// track.
    pub fn set_reference_for_testing(&self, track: ReferenceTrack) {
        let _settings = self.settings.lock();
        *self.reference.lock() = Some(track);
    }
}

/// Persists the assistant's [`AssistantSettings`] with the plugin state
/// (the plugin chains it into its preset session's extra state).
///
/// The bridge calls this from the main thread, possibly while the plugin
/// processes audio; it only touches the assistant's mutexes. A restored
/// reference path is decoded on a short-lived worker thread, so a project
/// load never waits on decoding a reference file, and a file that has
/// gone missing just shows its error in the assistant panel. One thread
/// per plugin instance at most: restores arriving while it decodes queue
/// behind it, the newest replacing any older one still waiting.
pub struct AssistantStateSaver {
    viz: Arc<MasteringViz>,
}

impl AssistantStateSaver {
    pub fn new(viz: Arc<MasteringViz>) -> Arc<Self> {
        Arc::new(Self { viz })
    }
}

impl resonance_plugin::ExtraStateSaver for AssistantStateSaver {
    fn save(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        if let Some(entry) = self.viz.assistant.save_state() {
            map.insert(STATE_KEY.to_string(), entry);
        }
        map
    }

    fn load(&self, state: &serde_json::Value) {
        let Some(path) = self.viz.assistant.load_state(state) else {
            return;
        };
        if !self.viz.assistant.queue_reload(path.clone()) {
            return;
        }
        let viz = self.viz.clone();
        let spawned = std::thread::Builder::new()
            .name("mastering-reference".into())
            .spawn(move || viz.assistant.run_reloads());
        if let Err(e) = spawned {
            let assistant = &self.viz.assistant;
            // Nothing will drain the queue: empty it, so the next restore
            // tries a thread again.
            while assistant.next_reload().is_some() {}
            let error = Err(format!("could not start the reference decode: {e}"));
            assistant.install_if_current(&path, error);
        }
    }
}
