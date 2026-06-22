//! MIDI Import modal state: the shared shell plus the per-track UI rows
//! the user reviews before importing. Mirrors the export/bounce modal
//! family (see [`crate::view::bounce_dialog`]) so the dialog reads as one
//! family of overlays.
//!
//! The flow walks through stages — drop a file, parse it, review the
//! detected tracks, reconcile any tempo difference, then confirm. Only
//! the state model lives here; the per-stage view bodies and the
//! parse/import orchestration land in the follow-up todos (doc #158).
//!
//! Every row/summary type is **app-level on purpose**: the orchestration
//! layer maps the parser's `ImportedSmf` onto these, so the view and
//! update layers never touch `resonance-audio`'s import internals.

use resonance_audio::types::TrackId;
use std::path::PathBuf;

/// Which step of the import flow the modal is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStage {
    /// Initial state: prompt the user to drop or choose a MIDI file.
    Drop,
    /// A file was chosen and is being parsed in the background.
    Parsing,
    /// Parsing succeeded; the user reviews the detected tracks.
    Review,
    /// The file's tempo differs from the project's and the user must
    /// choose how to reconcile them.
    TempoConflict,
    /// Parsing or import failed; [`ImportDialogState::error`] holds why.
    Error,
    /// The import completed; [`ImportDialogState::result`] holds a summary.
    Imported,
}

/// How to treat the imported file's tempo against the project's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempoChoice {
    /// Keep the project tempo; imported notes are time-warped onto it.
    KeepProject,
    /// Adopt the file's tempo map for the project.
    AdoptFile,
}

/// Where imported clips are anchored on the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementStart {
    /// Anchor at the very start of the timeline (bar 1).
    Bar1,
    /// Anchor at the current playhead position.
    Playhead,
}

/// Whether imported tracks create fresh timeline tracks or merge into an
/// existing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementMode {
    /// Create a new track per selected import row.
    NewTracks,
    /// Merge all selected rows into one already-existing track.
    MergeIntoSelected,
}

/// How to align a tempo-conflicted import against the project grid when
/// the user keeps the project tempo. Used by the `TempoConflict` stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempoAlignment {
    /// Preserve musical bar/beat positions (stretch onto project tempo).
    MatchBars,
    /// Preserve absolute timing (notes land at the same wall-clock time).
    MatchTime,
}

/// Where imported clips land: the timeline anchor, the new-vs-merge mode
/// and — when merging — the target track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub start: PlacementStart,
    pub mode: PlacementMode,
    /// Target track for [`PlacementMode::MergeIntoSelected`]; ignored for
    /// [`PlacementMode::NewTracks`].
    pub merge_target: Option<TrackId>,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            start: PlacementStart::Bar1,
            mode: PlacementMode::NewTracks,
            merge_target: None,
        }
    }
}

/// A single note in a row's preview strip. App-level mirror of a parsed
/// MIDI note carrying only what the review preview renders, kept
/// independent of `resonance-audio`'s `MidiNote`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviewNote {
    pub pitch: u8,
    pub start_tick: u64,
    pub duration_ticks: u64,
    pub velocity: f32,
}

/// One detected source track in the import, plus the user's per-track
/// choices in the Review stage.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackImportRow {
    /// Whether this track is included in the import.
    pub selected: bool,
    /// Editable destination name, seeded from the file's track meta.
    pub name: String,
    /// Source MIDI channel (0-15).
    pub channel: u8,
    /// Number of notes detected on the track.
    pub note_count: usize,
    /// Lowest pitch present, or `None` when the track carries no notes.
    pub pitch_min: Option<u8>,
    /// Highest pitch present, or `None` when the track carries no notes.
    pub pitch_max: Option<u8>,
    /// True for a conductor/tempo track (no notes, carries tempo + meta).
    pub is_conductor: bool,
    /// A small sample of notes for the review preview strip.
    pub preview: Vec<PreviewNote>,
}

/// High-level summary of a parsed MIDI file, shown above the per-track
/// rows in the Review stage.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportSummary {
    /// Display name of the source file.
    pub file_name: String,
    /// Total number of source tracks detected.
    pub track_count: usize,
    /// Total notes across every track.
    pub total_notes: usize,
    /// Tempo (BPM) from the file's first tempo event, if any.
    pub file_tempo_bpm: Option<f32>,
    /// True when the file's tempo differs from the project's, routing the
    /// flow through the `TempoConflict` stage.
    pub tempo_conflict: bool,
}

/// Successful parse payload handed to the dialog by the parse task: the
/// file summary plus the per-track rows the user reviews.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedImport {
    pub summary: ImportSummary,
    pub rows: Vec<TrackImportRow>,
}

/// Outcome summary shown on the `Imported` stage once the import lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportResultSummary {
    pub tracks_created: usize,
    pub clips_added: usize,
    pub notes_imported: usize,
}

/// Transient state for the MIDI Import modal. Lives on
/// `Resonance::import_dialog` while the overlay is open and is `None` when
/// closed. Opened to the [`ImportStage::Drop`] stage; closed on
/// Cancel / done.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportDialogState {
    /// Current step of the flow.
    pub stage: ImportStage,
    /// Source file path once chosen or dropped.
    pub source_path: Option<PathBuf>,
    /// Parsed file summary, set when parsing completes.
    pub summary: Option<ImportSummary>,
    /// Per-track UI rows the user reviews and edits.
    pub rows: Vec<TrackImportRow>,
    /// Tempo reconciliation choice.
    pub tempo_choice: TempoChoice,
    /// Tempo-conflict alignment (meaningful only on the `TempoConflict`
    /// stage when [`TempoChoice::KeepProject`] is selected).
    pub tempo_alignment: TempoAlignment,
    /// Where imported clips land on the timeline.
    pub placement: Placement,
    /// Set on the `Error` stage with a user-facing explanation.
    pub error: Option<String>,
    /// Set on the `Imported` stage with the import outcome.
    pub result: Option<ImportResultSummary>,
}

impl ImportDialogState {
    /// Open a fresh dialog at the [`ImportStage::Drop`] stage with the
    /// default tempo / placement choices.
    pub fn new() -> Self {
        Self {
            stage: ImportStage::Drop,
            source_path: None,
            summary: None,
            rows: Vec::new(),
            tempo_choice: TempoChoice::KeepProject,
            tempo_alignment: TempoAlignment::MatchBars,
            placement: Placement::default(),
            error: None,
            result: None,
        }
    }
}

impl Default for ImportDialogState {
    fn default() -> Self {
        Self::new()
    }
}
