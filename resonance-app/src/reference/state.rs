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
    /// Per-reference marker id, allocated by the engine.
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
#[derive(Debug, Clone, Default)]
pub struct ReferenceState {
    /// All loaded references, in load order.
    pub entries: Vec<ReferenceEntry>,
    /// Which reference the A/B monitor auditions, if any.
    pub active_id: Option<ReferenceId>,
    /// Whether the monitor is currently on the mix or the reference.
    pub ab_source: ABSource,
    /// Whether the active reference is loudness-matched to the mix.
    pub loudness_match: bool,
    /// Applied loudness-match gain offset (dB), reported by the engine.
    pub offset_db: f32,
    /// Manual level trim (dB) on top of any loudness match.
    pub trim_db: f32,
    /// Whether the reference cursor follows the mix transport.
    pub loop_to_mix: bool,
    /// Latest A/B meter snapshot (transient; repopulated each poll).
    pub ab_meter: Option<AbMeters>,
    /// Most recent load-failure reason, shown until dismissed. Load
    /// failures carry no id, so they live here rather than as an entry.
    pub last_error: Option<String>,
    /// Paths whose `LoadReferenceTrack` has been dispatched but whose
    /// engine-allocated id is not yet known. Drained FIFO when the first
    /// analysis event for a new id arrives, to recover its name / path.
    /// Runtime-only — never part of an undo snapshot.
    pub pending_loads: VecDeque<String>,
    /// The source to restore when a momentary-audition gesture ends.
    /// Runtime-only — never part of an undo snapshot.
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

    /// Capture the undo-relevant subset (the user-meaningful content) for
    /// an undo snapshot. Transient monitoring state — `ab_source`,
    /// `loop_to_mix`, the meter snapshot, in-flight loads and the
    /// momentary-restore target — is deliberately left live across an
    /// undo/redo so a history step doesn't yank the monitor around.
    pub fn undo_snapshot(&self) -> ReferenceUndo {
        ReferenceUndo {
            entries: self.entries.clone(),
            active_id: self.active_id,
            loudness_match: self.loudness_match,
            offset_db: self.offset_db,
            trim_db: self.trim_db,
        }
    }

    /// Restore the undo-relevant subset captured by [`Self::undo_snapshot`],
    /// leaving the live monitoring fields untouched.
    pub fn restore_undo(&mut self, snap: ReferenceUndo) {
        self.entries = snap.entries;
        self.active_id = snap.active_id;
        self.loudness_match = snap.loudness_match;
        self.offset_db = snap.offset_db;
        self.trim_db = snap.trim_db;
    }
}

/// The subset of [`ReferenceState`] carried in an undo snapshot. See
/// [`ReferenceState::undo_snapshot`] for what is and isn't captured.
#[derive(Debug, Clone, Default)]
pub struct ReferenceUndo {
    pub entries: Vec<ReferenceEntry>,
    pub active_id: Option<ReferenceId>,
    pub loudness_match: bool,
    pub offset_db: f32,
    pub trim_db: f32,
}
