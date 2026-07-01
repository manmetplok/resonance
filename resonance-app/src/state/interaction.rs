//! Transient interaction state (selection, drag/trim handles, the open
//! MIDI editor) and the MIDI editor's own viewport.

use std::collections::BTreeSet;

use resonance_audio::quantize::{Division, GridModifier, GridValue, QuantizeMode};
use resonance_audio::types::*;

use super::clips::{
    ClipDragState, ClipTrimState, FadeDragState, GainDragState, MidiClipDragState,
    MidiClipTrimState,
};
use super::global::SelectedGlobalEvent;

/// State for the MIDI piano roll editor.
#[derive(Debug, Clone)]
pub struct MidiEditorState {
    pub clip_id: ClipId,
    pub track_id: TrackId,
    pub scroll_y: f32,
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub snap_ticks: u64,
    /// Indices (into the clip's `notes`) of the currently selected notes.
    /// A `BTreeSet` keeps them sorted and deduplicated, which lets bulk
    /// ops (e.g. delete) walk them in a deterministic order. The piano
    /// roll drives the full multi-selection; the vocal roll still works
    /// one note at a time and reads [`MidiEditorState::primary_selected`].
    pub selected_notes: BTreeSet<usize>,
}

impl MidiEditorState {
    /// Replace the selection with a single note, or clear it when `None`.
    /// This is the plain-click / vocal-roll path.
    pub fn select_single(&mut self, note_index: Option<usize>) {
        self.selected_notes.clear();
        if let Some(i) = note_index {
            self.selected_notes.insert(i);
        }
    }

    /// Toggle one note's membership in the selection (shift/ctrl-click).
    pub fn toggle_note(&mut self, note_index: usize) {
        if !self.selected_notes.remove(&note_index) {
            self.selected_notes.insert(note_index);
        }
    }

    /// Apply a marquee result: union with the existing selection when
    /// `additive` (shift held), otherwise replace it.
    pub fn apply_marquee(&mut self, indices: impl IntoIterator<Item = usize>, additive: bool) {
        if !additive {
            self.selected_notes.clear();
        }
        self.selected_notes.extend(indices);
    }

    /// Select every note of a clip holding `len` notes.
    pub fn select_all(&mut self, len: usize) {
        self.selected_notes = (0..len).collect();
    }

    /// Drop the whole selection.
    pub fn clear_selection(&mut self) {
        self.selected_notes.clear();
    }

    /// Whether `note_index` is currently selected.
    pub fn is_selected(&self, note_index: usize) -> bool {
        self.selected_notes.contains(&note_index)
    }

    /// A single representative selected index, for editors that still
    /// operate on one note at a time (the vocal roll).
    pub fn primary_selected(&self) -> Option<usize> {
        self.selected_notes.iter().copied().next()
    }
}

/// Transient clip interaction state: current selection, active drag/trim,
/// and the open MIDI editor if any.
#[derive(Debug, Default)]
pub struct ClipInteractionState {
    pub selected_clip: Option<ClipId>,
    pub selected_midi_clip: Option<ClipId>,
    /// Currently selected (highlighted) track in the arrange view.
    pub selected_track: Option<TrackId>,
    pub clip_drag: Option<ClipDragState>,
    pub clip_trim: Option<ClipTrimState>,
    /// Active fade-handle drag on an audio clip, if any (todo #317).
    pub clip_fade_drag: Option<FadeDragState>,
    /// Active clip-gain bead drag, if any (todo #317).
    pub clip_gain_drag: Option<GainDragState>,
    pub midi_clip_drag: Option<MidiClipDragState>,
    pub midi_clip_trim: Option<MidiClipTrimState>,
    pub editing_midi_clip: Option<MidiEditorState>,
    /// Audio clip whose vocal pitch editor is open, if any (doc #160).
    /// Set when the user opens the pitch editor on a vocal clip (which
    /// also requests analysis); the editor view (a later todo) renders
    /// the clip's [`ClipState::vocal_tuning`](super::ClipState) mirror.
    pub editing_pitch_clip: Option<ClipId>,
    /// Currently selected event on a global track (tempo or signature).
    pub selected_global_event: Option<SelectedGlobalEvent>,
    /// Currently selected arrangement marker, if any. Threaded into the
    /// timeline canvas so the selected flag / region span renders with the
    /// stronger accent (todo #368). Set by the ruler hit-testing (#369).
    pub selected_marker_id: Option<u64>,
    /// Open right-click context menu for a marker, if any (todo #369). The
    /// menu is rendered as a floating overlay anchored at `x` / `y`.
    pub marker_menu: Option<MarkerMenuState>,
    /// In-progress inline rename of a marker, if any (todo #369). Holds the
    /// live edit buffer; committing re-dispatches `MarkerMessage::Rename`.
    pub marker_rename: Option<MarkerRenameState>,
}

/// A marker's open right-click context menu. `x` / `y` are the window-space
/// anchor (cursor position at open time) the overlay positions itself at.
#[derive(Debug, Clone)]
pub struct MarkerMenuState {
    pub marker_id: u64,
    pub x: f32,
    pub y: f32,
}

/// An in-progress inline marker rename. `text` is the live edit buffer,
/// seeded from the marker's current name; `x` / `y` anchor the floating
/// text field in window space.
#[derive(Debug, Clone)]
pub struct MarkerRenameState {
    pub marker_id: u64,
    pub text: String,
    pub x: f32,
    pub y: f32,
}

/// A user-selectable quantize grid division for the MIDI editor's
/// Quantize panel (todo #392). Each variant maps to a resonance-audio
/// [`Division`] via [`GridChoice::division`]. Twelve entries: 1/4 .. 1/32
/// each in straight, triplet (`T`) and dotted (`.`) flavours. Used as the
/// (static, never-changing) option set for the grid pick_list, so the
/// view caches the option slice once rather than allocating per frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridChoice {
    Quarter,
    QuarterTriplet,
    QuarterDotted,
    Eighth,
    EighthTriplet,
    EighthDotted,
    Sixteenth,
    SixteenthTriplet,
    SixteenthDotted,
    ThirtySecond,
    ThirtySecondTriplet,
    ThirtySecondDotted,
}

impl GridChoice {
    /// Every choice, in pick_list display order (coarse → fine, each
    /// value grouped straight / triplet / dotted).
    pub const ALL: [GridChoice; 12] = [
        GridChoice::Quarter,
        GridChoice::QuarterTriplet,
        GridChoice::QuarterDotted,
        GridChoice::Eighth,
        GridChoice::EighthTriplet,
        GridChoice::EighthDotted,
        GridChoice::Sixteenth,
        GridChoice::SixteenthTriplet,
        GridChoice::SixteenthDotted,
        GridChoice::ThirtySecond,
        GridChoice::ThirtySecondTriplet,
        GridChoice::ThirtySecondDotted,
    ];

    /// The base note value and modifier this choice resolves to.
    fn parts(self) -> (GridValue, GridModifier) {
        match self {
            GridChoice::Quarter => (GridValue::Quarter, GridModifier::Straight),
            GridChoice::QuarterTriplet => (GridValue::Quarter, GridModifier::Triplet),
            GridChoice::QuarterDotted => (GridValue::Quarter, GridModifier::Dotted),
            GridChoice::Eighth => (GridValue::Eighth, GridModifier::Straight),
            GridChoice::EighthTriplet => (GridValue::Eighth, GridModifier::Triplet),
            GridChoice::EighthDotted => (GridValue::Eighth, GridModifier::Dotted),
            GridChoice::Sixteenth => (GridValue::Sixteenth, GridModifier::Straight),
            GridChoice::SixteenthTriplet => (GridValue::Sixteenth, GridModifier::Triplet),
            GridChoice::SixteenthDotted => (GridValue::Sixteenth, GridModifier::Dotted),
            GridChoice::ThirtySecond => (GridValue::ThirtySecond, GridModifier::Straight),
            GridChoice::ThirtySecondTriplet => (GridValue::ThirtySecond, GridModifier::Triplet),
            GridChoice::ThirtySecondDotted => (GridValue::ThirtySecond, GridModifier::Dotted),
        }
    }

    /// The resonance-audio [`Division`] this choice resolves to.
    pub fn division(self) -> Division {
        let (value, modifier) = self.parts();
        Division { value, modifier }
    }

    /// Short label shown in the pick_list (e.g. `1/8T`, `1/16.`).
    pub fn label(self) -> &'static str {
        match self {
            GridChoice::Quarter => "1/4",
            GridChoice::QuarterTriplet => "1/4T",
            GridChoice::QuarterDotted => "1/4.",
            GridChoice::Eighth => "1/8",
            GridChoice::EighthTriplet => "1/8T",
            GridChoice::EighthDotted => "1/8.",
            GridChoice::Sixteenth => "1/16",
            GridChoice::SixteenthTriplet => "1/16T",
            GridChoice::SixteenthDotted => "1/16.",
            GridChoice::ThirtySecond => "1/32",
            GridChoice::ThirtySecondTriplet => "1/32T",
            GridChoice::ThirtySecondDotted => "1/32.",
        }
    }
}

impl std::fmt::Display for GridChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Current settings of the MIDI editor's Quantize panel (todo #392). The
/// panel's controls write here; the Apply button reads it to build the
/// bulk [`MidiEditorMessage::Quantize`](crate::message::MidiEditorMessage)
/// that operates on the active note selection (or the whole clip when the
/// selection is empty). Lives at the app level so the chosen settings
/// persist across clip open/close — and so the groove/settings
/// persistence slice (todo #395) can serialise the last-used values.
#[derive(Debug, Clone)]
pub struct MidiQuantizePanelState {
    /// Selected grid division.
    pub grid: GridChoice,
    /// Quantize strength, `0.0..=1.0` (shown as 0–100%).
    pub strength: f32,
    /// Swing amount, `0.0..=1.0` (shown as 0–100%).
    pub swing: f32,
    /// Whether to quantize note starts only or starts and lengths.
    pub mode: QuantizeMode,
    /// Snap note-offs to the grid as well as note-ons.
    pub quantize_ends: bool,
    /// Apply the strength blend iteratively (soft quantize).
    pub iterative: bool,
    /// Humanize timing jitter — maximum absolute offset, in ticks
    /// (`0..=`[`HUMANIZE_TIMING_MAX_TICKS`]). Drives the Humanize panel's
    /// timing slider; the Humanize Apply button reads it.
    pub humanize_timing: u32,
    /// Humanize velocity jitter fraction, `0.0..=1.0` (shown as 0–100%).
    pub humanize_velocity: f32,

    // -- Groove extract / apply (todo #394, doc #163) --
    /// Name typed into the "Extract groove" field. When the user extracts,
    /// this is stashed in [`pending_groove_name`](Self::pending_groove_name)
    /// and the freshly captured template lands in the project groove library
    /// under it (a blank name falls back to an auto-numbered default).
    pub groove_name: String,
    /// Name awaiting the in-flight `GrooveExtracted` engine event. Set when
    /// the extract command is dispatched and consumed by the event mirror
    /// (#390) that creates the named [`UserGroove`](super::quantize::UserGroove).
    pub pending_groove_name: Option<String>,
    /// Groove currently selected in the apply picker (stock or user). Drives
    /// the pick_list value and the Apply button's dispatch.
    pub groove_selection: super::quantize::GrooveSelection,
    /// Strength of the groove feel to apply, `0.0..=1.0` (shown as 0–100%).
    pub groove_strength: f32,
}

/// Upper bound of the Humanize timing slider, in ticks. One eighth note
/// (`TICKS_PER_QUARTER_NOTE / 2 = 240`): enough loosening to feel human
/// without smearing notes across the beat. Kept here so the view and the
/// setter handler agree on the clamp.
pub const HUMANIZE_TIMING_MAX_TICKS: u32 = 240;

impl Default for MidiQuantizePanelState {
    fn default() -> Self {
        Self {
            grid: GridChoice::Sixteenth,
            strength: 1.0,
            swing: 0.0,
            mode: QuantizeMode::StartOnly,
            quantize_ends: false,
            iterative: false,
            humanize_timing: 0,
            humanize_velocity: 0.0,
            groove_name: String::new(),
            pending_groove_name: None,
            groove_selection: super::quantize::GrooveSelection::None,
            groove_strength: 1.0,
        }
    }
}
