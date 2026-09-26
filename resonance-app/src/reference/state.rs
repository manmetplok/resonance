//! GUI-side state for the reference-track (A/B) comparison feature.
//!
//! A *reference* is an external mastered track the user loads alongside
//! the project mix to A/B against. This module owns the view-facing
//! mirror of the engine's reference state: the loaded entries, which one
//! is active, the monitored source, loudness-match / trim settings, and
//! the latest A/B meter snapshot. The engine remains the source of truth
//! — handlers mutate this mirror optimistically and the engine echoes
//! authoritative values back through `engine_events::reference`.

use std::collections::VecDeque;

use resonance_audio::types::{ABSource, ReferenceAnalysisStage, ReferenceId};
use resonance_metering::MeterSnapshot;

/// Lifecycle of a single loaded reference, surfaced so the (later) view
/// can show an "analysing…" spinner, a ready waveform, or an error.
#[derive(Debug, Clone, PartialEq)]
pub enum ReferenceStatus {
    /// Offline analysis is in progress; carries the current stage so a
    /// determinate progress indicator can be shown.
    Analyzing(ReferenceAnalysisStage),
    /// Decoded, measured, and ready to audition.
    Loaded,
    /// The reference's source file could not be found (e.g. a project
    /// referencing a path that has since moved).
    Missing,
    /// Analysis failed; carries the reason for display.
    Error(String),
}

/// A user-placed comparison marker on a reference, mirroring
/// [`resonance_audio::types::ReferenceMarker`] in a form the view owns.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceMarkerState {
    /// Per-reference marker id, allocated by the app
    /// ([`ReferenceState::alloc_marker_id`]).
    pub id: u32,
    /// Position within the reference track, in sample frames.
    pub position_samples: u64,
    /// User-facing label.
    pub label: String,
}

/// One loaded reference track.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceEntry {
    pub id: ReferenceId,
    /// Display name (file stem unless the engine supplies one).
    pub name: String,
    /// Source path as reported by the engine.
    pub path: String,
    pub status: ReferenceStatus,
    /// Integrated loudness (LUFS) measured during analysis. `NEG_INFINITY`
    /// until [`ReferenceStatus::Loaded`].
    pub integrated_lufs: f32,
    /// Downsampled (min, max) waveform overview for drawing.
    pub waveform_peaks: Vec<(f32, f32)>,
    /// Comparison markers, ordered as the engine reports them.
    pub markers: Vec<ReferenceMarkerState>,
    /// The reference's own playback cursor, in sample frames.
    pub position_samples: u64,
    /// Total length of the reference, in sample frames. `0` until the
    /// engine reports it on [`ReferenceStatus::Loaded`]; used to map the
    /// playback cursor and markers onto the waveform overview.
    pub length_samples: u64,
}

impl ReferenceEntry {
    /// A freshly-registered entry whose analysis has just begun. Used when
    /// the first `ReferenceAnalysisProgress` arrives before the terminal
    /// `ReferenceLoaded` event has populated name / peaks / loudness.
    pub fn analyzing(id: ReferenceId, name: String, path: String, stage: ReferenceAnalysisStage) -> Self {
        Self {
            id,
            name,
            path,
            status: ReferenceStatus::Analyzing(stage),
            integrated_lufs: f32::NEG_INFINITY,
            waveform_peaks: Vec::new(),
            markers: Vec::new(),
            position_samples: 0,
            length_samples: 0,
        }
    }
}

/// The latest A/B meter snapshot from `AudioEvent::ABMeterSnapshot`.
#[derive(Debug, Clone, Copy)]
pub struct AbMeters {
    pub mix: MeterSnapshot,
    /// `None` when no reference is active.
    pub reference: Option<MeterSnapshot>,
}

/// GUI-side reference/A/B state. Hangs off [`crate::Resonance`].
///
/// Split three ways (ARCH-01 A-5):
///
/// - **Content** — `entries`, `active_id`, `loudness_match`, `trim_db`:
///   what the user compares against and how it is levelled. Persisted in
///   `ProjectFile::references` / `reference_settings` (an entry's path,
///   name, cached loudness and markers; the rest of an entry is engine
///   readback) and restored from there by a disk load and both undo
///   paths.
/// - **Monitor** — [`ReferenceMonitorState`]: what the user is listening
///   to right now. Never undone, so a history step does not yank the
///   monitor around.
/// - **Runtime bookkeeping** — `last_error`, `pending_loads`,
///   `next_engine_id`, `next_marker_id`. Neither saved nor undone.
#[derive(Debug, Clone, Default)]
pub struct ReferenceState {
    /// All loaded references, in load order.
    pub entries: Vec<ReferenceEntry>,
    /// Which reference the A/B monitor auditions, if any.
    pub active_id: Option<ReferenceId>,
    /// Whether the active reference is loudness-matched to the mix.
    pub loudness_match: bool,
    /// Manual level trim (dB) on top of any loudness match.
    pub trim_db: f32,
    /// Live monitoring state; see [`ReferenceMonitorState`].
    pub monitor: ReferenceMonitorState,
    /// Most recent load-failure reason, shown until dismissed. Load
    /// failures carry no id, so they live here rather than as an entry.
    pub last_error: Option<String>,
    /// Paths whose `LoadReferenceTrack` has been dispatched but whose
    /// engine-allocated id is not yet known. Drained FIFO when the first
    /// analysis event for a new id arrives, to recover its name / path.
    pub pending_loads: VecDeque<String>,
    /// The app's copy of the engine's reference-id allocator: past every
    /// id the engine has registered, or will register for a load already
    /// sent. A restore that brings a reference back hints its id from
    /// here ([`Self::alloc_engine_id`]), so it never collides with a live
    /// engine entry. `ClearAll` resets the engine's allocator; the replay
    /// after it resets this one (`restore_references`).
    pub next_engine_id: u32,
    /// Marker-id allocator, session-monotonic and shared by every
    /// reference (ids only need to be unique per reference). The app owns
    /// marker ids (FU-A5a): a restore brings saved markers back into the
    /// GUI only, so an engine-side allocator restarted at 1 under them.
    /// Survives every restore; [`Self::alloc_marker_id`] also stays past
    /// the markers an entry already holds.
    pub next_marker_id: u32,
}

/// The monitor half of [`ReferenceState`]: what the A/B switch listens
/// to and the live readback that goes with it. `ab_source` and
/// `loop_to_mix` are remembered per project
/// (`ProjectReferenceSettings::{ab_source_is_reference, loop_to_mix}`),
/// but none of this is undo state: the undo snapshot strips it and both
/// restore paths leave it as it is.
#[derive(Debug, Clone, Default)]
pub struct ReferenceMonitorState {
    /// Whether the monitor is currently on the mix or the reference.
    pub ab_source: ABSource,
    /// Whether the reference cursor follows the mix transport.
    pub loop_to_mix: bool,
    /// Applied loudness-match gain offset (dB), reported by the engine —
    /// readback derived from the active reference's analysis, re-reported
    /// whenever loudness matching is (re)set.
    pub offset_db: f32,
    /// Latest A/B meter snapshot (transient; repopulated each poll).
    pub ab_meter: Option<AbMeters>,
    /// The source to restore when a momentary-audition gesture ends.
    pub momentary_restore: Option<ABSource>,
}

impl ReferenceState {
    /// Index of the entry with `id`, if loaded.
    pub fn index_of(&self, id: ReferenceId) -> Option<usize> {
        self.entries.iter().position(|e| e.id == id)
    }

    /// Mutable handle to the entry with `id`, if loaded.
    pub fn entry_mut(&mut self, id: ReferenceId) -> Option<&mut ReferenceEntry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    /// Note that the engine has registered (or will register) `id`, so
    /// [`Self::alloc_engine_id`] stays past it. Ids of missing entries,
    /// which the engine never hears about, are ignored.
    pub fn saw_engine_id(&mut self, id: ReferenceId) {
        if id.0 < crate::state::ids::MISSING_REFERENCE_ID_BASE {
            self.next_engine_id = self.next_engine_id.max(id.0.saturating_add(1));
        }
    }

    /// Note a `LoadReferenceTrack` sent without a hint: the engine gives
    /// it the next id from its allocator.
    pub fn saw_unhinted_load(&mut self) {
        self.next_engine_id = self.next_engine_id.max(1) + 1;
    }

    /// A fresh marker id for the reference `ref_id`: past every id this
    /// session has handed out and every marker the reference holds.
    pub fn alloc_marker_id(&mut self, ref_id: ReferenceId) -> u32 {
        let held = self
            .entries
            .iter()
            .find(|e| e.id == ref_id)
            .and_then(|e| e.markers.iter().map(|m| m.id).max())
            .map_or(0, |id| id.saturating_add(1));
        let id = self.next_marker_id.max(held).max(1);
        self.next_marker_id = id + 1;
        id
    }

    /// An id no live or in-flight engine entry uses, for a hinted
    /// `LoadReferenceTrack`.
    pub fn alloc_engine_id(&mut self) -> ReferenceId {
        let id = self.next_engine_id.max(1);
        self.next_engine_id = id + 1;
        ReferenceId(id)
    }
}
