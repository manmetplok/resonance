//! State model for the Export modal (design doc #155).
//!
//! The Export modal is a single overlay with two **mode tabs** — Audio
//! stems and MIDI — sharing one source selection, destination, and
//! overwrite policy. This module holds only the shared shell's data; the
//! per-tab body widgets (source checklist, range/format controls, MIDI
//! layout) and the render orchestration land in follow-up todos
//! (#326/#327 for the bodies, #328 for progress/done/error, #330/#331 for
//! the actual stem/MIDI render). Lives on `Resonance::export_dialog` while
//! the overlay is open.

use std::collections::BTreeSet;
use std::path::PathBuf;

use resonance_audio::types::{BusId, TrackId};

/// Which mode the Export modal is showing. Selection state is shared
/// across modes; the tab only changes which sources are eligible and
/// which format controls apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportMode {
    /// Render per-track / per-bus / master audio stems to WAV.
    AudioStems,
    /// Export instrument / vocal tracks to Standard MIDI Files.
    Midi,
}

/// A renderable source the user can tick in the sources checklist. Master
/// has no id; tracks and busses carry theirs. `Ord` so the selection set
/// has a deterministic iteration order (stable filename previews / tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExportSource {
    Track(TrackId),
    Bus(BusId),
    Master,
}

/// Render range shared by both modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportRange {
    /// The whole timeline from the project origin.
    WholeProject,
    /// The current loop region / selection only.
    LoopOrSelection,
}

/// Bit depth for stem WAVs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportBitDepth {
    Int16,
    Int24,
    Float32,
}

/// What to do when a target filename already exists on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportOverwrite {
    /// Append `_1`, `_2`, … so nothing is clobbered.
    Suffix,
    /// Overwrite the existing file in place.
    Overwrite,
    /// Prompt per collision at write time.
    Ask,
}

/// MIDI file layout for the MIDI tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiLayout {
    /// One `.mid` per source track.
    FilePerTrack,
    /// A single multi-track `.mid`.
    SingleMultiTrack,
}

/// Format options for both modes. The audio fields apply in
/// [`ExportMode::AudioStems`]; the MIDI fields apply in
/// [`ExportMode::Midi`]. Kept in one struct so the shared shell can hold a
/// single value regardless of the active tab.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExportFormat {
    /// Output sample rate for stems, in Hz.
    pub sample_rate: u32,
    pub bit_depth: ExportBitDepth,
    /// Let plugin FX tails ring out past the range end vs hard-cut at it.
    pub include_fx_tail: bool,
    pub midi_layout: MidiLayout,
    /// Embed the project tempo & time-signature map as meta events.
    pub midi_embed_tempo: bool,
}

impl Default for ExportFormat {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            bit_depth: ExportBitDepth::Int24,
            include_fx_tail: true,
            midi_layout: MidiLayout::FilePerTrack,
            midi_embed_tempo: true,
        }
    }
}

/// Lifecycle phase of an open Export modal. The body the view renders is
/// driven by this: `Setup` shows the mode tabs + per-tab body, while the
/// render phases reuse the bounce-progress modal pattern. The render
/// phases are filled in by todo #328; the scaffold only ever sits in
/// `Setup`.
#[derive(Debug, Clone, PartialEq)]
pub enum ExportPhase {
    /// Picking sources, range, format, destination — the editable state.
    Setup,
    /// A render is in flight; carries how many targets are done out of the
    /// total so the progress modal can show "Stem 3 of 6".
    Rendering { done: usize, total: usize },
    /// All targets finished; carries the written file paths.
    Done(Vec<PathBuf>),
    /// A target failed. Already-written stems stay valid; `remaining` is
    /// how many targets still need rendering for the Retry action.
    Error { written: Vec<PathBuf>, message: String, remaining: usize },
    /// The user cancelled between targets; the stems written before the
    /// cancel stay on disk and are listed here.
    Cancelled(Vec<PathBuf>),
}

/// Transient state for the Export modal. Held on `Resonance::export_dialog`
/// while the overlay is open; `None` when closed.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportDialogState {
    pub mode: ExportMode,
    pub phase: ExportPhase,
    /// Sources the user has ticked. Shared across both modes; the MIDI tab
    /// only renders the subset that accepts MIDI (filtered in the view).
    pub selected_sources: BTreeSet<ExportSource>,
    pub range: ExportRange,
    pub format: ExportFormat,
    /// Output folder. Defaults to `<project>/stems` for a saved project;
    /// `None` for an untitled one until the user picks a folder.
    pub destination: Option<PathBuf>,
    pub overwrite: ExportOverwrite,
    /// Per-target failures of the running export
    /// (`StemExportTargetError`): the queue keeps going past them, so they
    /// are collected and shown once it finishes.
    pub target_errors: Vec<String>,
    /// A `CancelStemExport` was sent for the running export; the footer's
    /// cancel button is spent until the engine answers.
    pub cancel_requested: bool,
}

impl ExportDialogState {
    /// A freshly-opened dialog: Audio-stems mode, nothing selected,
    /// whole-project range, default format, suffix-on-collision.
    pub fn new() -> Self {
        Self {
            mode: ExportMode::AudioStems,
            phase: ExportPhase::Setup,
            selected_sources: BTreeSet::new(),
            range: ExportRange::WholeProject,
            format: ExportFormat::default(),
            destination: None,
            overwrite: ExportOverwrite::Suffix,
            target_errors: Vec::new(),
            cancel_requested: false,
        }
    }

    /// Number of currently-selected sources — drives the footer's live
    /// count and the primary button label.
    pub fn selected_count(&self) -> usize {
        self.selected_sources.len()
    }

    /// Whether the primary action is enabled: the Audio-stems tab, at
    /// least one source selected, a destination folder, and the editable
    /// `Setup` phase. The MIDI tab has no exporter yet (its engine side
    /// does not exist), so it never enables the action.
    pub fn can_export(&self) -> bool {
        matches!(self.phase, ExportPhase::Setup)
            && self.mode == ExportMode::AudioStems
            && !self.selected_sources.is_empty()
            && self.destination.is_some()
    }

    /// A stem render is in flight: the offline renderer owns the plugin
    /// instances, so the app gates edits exactly as for a mixdown.
    pub fn is_rendering(&self) -> bool {
        matches!(self.phase, ExportPhase::Rendering { .. })
    }
}

impl Default for ExportDialogState {
    fn default() -> Self {
        Self::new()
    }
}
