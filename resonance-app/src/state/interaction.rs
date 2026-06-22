//! Transient interaction state (selection, drag/trim handles, the open
//! MIDI editor) and the MIDI editor's own viewport.

use resonance_audio::types::*;

use super::clips::{ClipDragState, ClipTrimState, MidiClipDragState, MidiClipTrimState};
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
    pub selected_note: Option<usize>,
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
    pub midi_clip_drag: Option<MidiClipDragState>,
    pub midi_clip_trim: Option<MidiClipTrimState>,
    pub editing_midi_clip: Option<MidiEditorState>,
    /// Currently selected event on a global track (tempo or signature).
    pub selected_global_event: Option<SelectedGlobalEvent>,
    /// Active drag-and-drop group-membership edit, if any (todo #685).
    pub membership_drag: Option<MembershipDragState>,
}

impl ClipInteractionState {
    /// Select a single track, replacing any existing multi-selection. Passing
    /// `None` clears the selection entirely. Keeps `selected_track` (the
    /// primary highlight) and `selected_tracks` (the Arrange multi-selection)
    /// in agreement so a normal click never leaves a stale group highlighted.
    pub fn select_single_track(&mut self, id: Option<TrackId>) {
        self.selected_track = id;
        self.selected_tracks = id.into_iter().collect();
    }

    /// Toggle a track in the multi-selection (an additive Cmd/Shift click).
    /// The primary `selected_track` follows the most recent member, or clears
    /// when the set empties.
    pub fn toggle_track_selection(&mut self, id: TrackId) {
        if let Some(pos) = self.selected_tracks.iter().position(|&t| t == id) {
            self.selected_tracks.remove(pos);
        } else {
            self.selected_tracks.push(id);
        }
        self.selected_track = self.selected_tracks.last().copied();
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
