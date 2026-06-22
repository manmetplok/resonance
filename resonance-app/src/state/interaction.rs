//! Transient interaction state (selection, drag/trim handles, the open
//! MIDI editor) and the MIDI editor's own viewport.

use resonance_audio::types::*;

use super::clips::{ClipDragState, ClipTrimState, MidiClipDragState, MidiClipTrimState};
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
    pub selected_note: Option<usize>,
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
}
