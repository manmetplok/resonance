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

/// What is being dragged during a drag-and-drop group-membership edit
/// (epic #36, doc #200, todo #685). A track row joins / leaves a group; a
/// group header nests under / un-nests from another group (members travel
/// with it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembershipDragSubject {
    /// A track row being dragged to change which group it belongs to.
    Track(TrackId),
    /// A group header being dragged to nest under / detach from a parent
    /// group. The id is the group's own id.
    Group(TrackId),
}

/// Where a membership drag currently hovers, already resolved to a drop
/// intent by the view's hit-test (a hover over a group's header *or* any
/// of its members both resolve to [`IntoGroup`](Self::IntoGroup)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembershipDropTarget {
    /// Over a group — the dragged subject joins this group (a track) or
    /// nests under it (a group). The id is the destination group's id.
    IntoGroup(TrackId),
    /// Over open, ungrouped space — the dragged subject leaves its group
    /// (a track) or un-nests back to the top level (a group).
    Ungrouped,
}

/// Live state of an in-progress drag-and-drop group-membership edit
/// (todo #685). Opened when a track row or group header starts dragging,
/// updated as the pointer moves over candidate drop targets, and consumed
/// on drop. Purely transient: it is never persisted and never enters the
/// undo snapshot — only the committed membership change is recorded.
#[derive(Debug, Clone)]
pub struct MembershipDragState {
    /// The track or group being dragged.
    pub subject: MembershipDragSubject,
    /// The group the subject currently sits in — a track's parent group or
    /// a group's nesting parent — so a drop onto open space knows what to
    /// detach from. `None` when the subject is already at the top level.
    pub origin_group: Option<TrackId>,
    /// The drop target under the pointer, if any. Drives the drop-target
    /// highlight, insertion line and destination chip while dragging, and
    /// selects the membership change applied on drop. `None` means "no
    /// valid target here" — dropping is then a no-op.
    pub hover: Option<MembershipDropTarget>,
    /// Latest pointer Y within the track-header column, for the drag ghost.
    pub cursor_y: f32,
}

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
    /// Primary (last-clicked) selected track. Drives the single-selection
    /// highlight the timeline canvas, mixer strip and inspector read.
    pub selected_track: Option<TrackId>,
    /// Multi-track selection set for the Arrange track-header column, in
    /// click order. A plain click resets this to the one clicked track; an
    /// additive (Cmd/Shift) click toggles membership. The "Group selected"
    /// floating bar appears while this holds two or more tracks (todo #684).
    pub selected_tracks: Vec<TrackId>,
    /// Whether the next track-header click should extend the multi-selection
    /// rather than replace it. Mirrors the live Cmd/Shift modifier state,
    /// kept in sync from `keyboard::Event::ModifiersChanged`.
    pub select_additive: bool,
    pub clip_drag: Option<ClipDragState>,
    pub clip_trim: Option<ClipTrimState>,
    /// Active fade-handle drag on an audio clip, if any (todo #317).
    pub clip_fade_drag: Option<FadeDragState>,
    /// Active clip-gain bead drag, if any (todo #317).
    pub clip_gain_drag: Option<GainDragState>,
    /// Active warp-marker drag on an audio clip, if any (clip warp).
    pub warp_marker_drag: Option<super::clip_warp::WarpMarkerDragState>,
    /// Per-clip tempo-detection status shown in the clip inspector.
    /// Transient: not project data, not undoable.
    pub tempo_detect: std::collections::HashMap<ClipId, super::clip_warp::TempoDetectStatus>,
    /// The inspector's source-tempo field mid-edit, if any.
    pub warp_bpm_draft: Option<super::clip_warp::WarpBpmDraft>,
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
    /// Bumped whenever a GUI message changes the timeline's Delete targets
    /// (`selected_clip` / `selected_midi_clip` / `selected_global_event`)
    /// to a new selection. The timeline canvas takes the keyboard when it
    /// sees a new value (`focus::KeyFocus::sync_grant`), so a selection
    /// made without a press on the canvas is deletable at once (code
    /// review FU-C2). Control-API calls never bump it.
    pub timeline_key_grant: u64,
    /// Active drag-and-drop group-membership edit, if any (todo #685).
    pub membership_drag: Option<MembershipDragState>,
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
    /// Open right-click context menu for an arrange track, if any (design
    /// doc #181, todo #581). Rendered as a floating overlay over the
    /// arrange main area, anchored at `x` / `y`.
    pub track_menu: Option<TrackMenuState>,
    /// In-progress "Save track as preset" name prompt, if any (ba todo
    /// #1303). Holds the live edit buffer and whether the typed name
    /// already exists, which is what turns the button into "Overwrite".
    pub preset_save: Option<PresetSaveState>,
    /// Tracks whose automation lanes are expanded into dedicated slim
    /// arrange sub-rows (doc #256, todo #1096). Transient view state —
    /// not project data, not undoable, not persisted. Toggled by
    /// `AutomationMessage::ToggleTrackExpanded`; consumed by the shared
    /// `ArrangeRowLayout` build so the canvas, track-header column and
    /// hit-testing all agree on the extra rows.
    pub automation_expanded_tracks: std::collections::HashSet<TrackId>,
    /// Tracks whose take lanes are expanded into stacked take sub-rows
    /// (epic #15, doc #165). Transient view state — not project data, not
    /// undoable, not persisted (the takes themselves are; whether their
    /// folder is open is not). Toggled by `UiMessage::ToggleTakeLane`;
    /// consumed by the shared `ArrangeRowLayout` build so the canvas and
    /// the track-header column agree on the extra rows.
    ///
    /// The comp ribbon on the track's own lane is drawn regardless of this
    /// set — a folded take lane must still show which take is audible
    /// where.
    pub take_lane_expanded_tracks: std::collections::HashSet<TrackId>,
}

impl ClipInteractionState {
    /// Select a single track, replacing any existing multi-selection. Passing
    /// `None` clears the selection entirely. Keeps `selected_track` (the
    /// primary highlight) and `selected_tracks` (the Arrange multi-selection)
    /// in agreement so a normal click never leaves a stale group highlighted.
    ///
    /// Callers go through [`UiTransientState::select_track`](crate::state::UiTransientState::select_track),
    /// which also takes the mixer's bus / master selection off.
    pub(crate) fn select_single_track(&mut self, id: Option<TrackId>) {
        self.selected_track = id;
        self.selected_tracks = id.into_iter().collect();
    }

    /// Toggle a track in the multi-selection (an additive Cmd/Shift click).
    /// The primary `selected_track` follows the most recent member, or clears
    /// when the set empties. Callers go through
    /// [`UiTransientState::toggle_track_selection`](crate::state::UiTransientState::toggle_track_selection).
    pub(crate) fn toggle_track_selection(&mut self, id: TrackId) {
        if let Some(pos) = self.selected_tracks.iter().position(|&t| t == id) {
            self.selected_tracks.remove(pos);
        } else {
            self.selected_tracks.push(id);
        }
        self.selected_track = self.selected_tracks.last().copied();
    }

    /// Make `id` the primary selected track without dropping the rest of
    /// the multi-selection: a member moves to the end (the most recent),
    /// a non-member joins. Focusing a plugin slot does this — it is not a
    /// selection gesture, so it never thins the selection out.
    pub(crate) fn make_primary_track(&mut self, id: TrackId) {
        self.selected_tracks.retain(|&t| t != id);
        self.selected_tracks.push(id);
        self.selected_track = Some(id);
    }

    /// Drop a track from the selection (e.g. when it is removed). Clears the
    /// primary highlight when it pointed at the gone track.
    pub fn deselect_track(&mut self, id: TrackId) {
        self.selected_tracks.retain(|&t| t != id);
        if self.selected_track == Some(id) {
            self.selected_track = None;
        }
    }
}

/// A marker's open right-click context menu. `x` / `y` are the window-space
/// anchor (cursor position at open time) the overlay positions itself at.
#[derive(Debug, Clone)]
pub struct MarkerMenuState {
    pub marker_id: u64,
    pub x: f32,
    pub y: f32,
}

/// A track's open right-click context menu (design doc #181, todo #581).
/// `x` / `y` anchor the floating overlay in arrange-area space — computed
/// from the row layout at open time, since a widget `mouse_area` press
/// carries no cursor position.
#[derive(Debug, Clone)]
pub struct TrackMenuState {
    pub track_id: TrackId,
    pub x: f32,
    pub y: f32,
}

/// An in-progress "Save track as preset" prompt (ba todo #1303).
///
/// A preset is a file, and a name that already exists would replace one
/// — so the prompt asks for the name and says, before the click, which
/// of the two things the button is about to do.
#[derive(Debug, Clone)]
pub struct PresetSaveState {
    /// The track being captured.
    pub track_id: TrackId,
    /// Live edit buffer, seeded with the track's own name.
    pub name: String,
    /// Whether a user preset of this name is already on disk.
    /// Recomputed on every keystroke, so the button label follows what
    /// is actually typed.
    pub exists: bool,
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
