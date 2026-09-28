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

pub struct Assistant {
    capture: Mutex<CaptureBuffer>,
    last_analysis: Mutex<Option<AnalysisResult>>,
    last_suggestions: Mutex<Option<Suggestions>>,
    reference: Mutex<Option<ReferenceTrack>>,
    reference_error: Mutex<Option<String>>,
    settings: Mutex<AssistantSettings>,
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
        *self.settings.lock() = restored;
        // Whatever reference was loaded belonged to the previous state.
        self.clear_reference();
        (!path.is_empty()).then_some(path)
    }

    /// Decode `path` as the reference, but install it only if it is
    /// still the configured reference path once the decode finishes — a
    /// restore that was superseded while decoding changes nothing.
    pub fn reload_reference_if_current(&self, path: &str) {
        let result = reference::load_from_path(path);
        if self.settings.lock().reference_path != path {
            return;
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
    pub fn load_reference(&self, path: &str) -> Result<(), String> {
        self.set_reference_path(path);
        match reference::load_from_path(path) {
            Ok(track) => {
                *self.reference.lock() = Some(track);
                *self.reference_error.lock() = None;
                Ok(())
            }
            Err(e) => {
                *self.reference.lock() = None;
                *self.reference_error.lock() = Some(e.clone());
                Err(e)
            }
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
/// gone missing just shows its error in the assistant panel.
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
        let viz = self.viz.clone();
        let spawned = std::thread::Builder::new()
            .name("mastering-reference".into())
            .spawn(move || viz.assistant.reload_reference_if_current(&path));
        if let Err(e) = spawned {
            *self.viz.assistant.reference_error.lock() =
                Some(format!("could not start the reference decode: {e}"));
        }
    }
}
