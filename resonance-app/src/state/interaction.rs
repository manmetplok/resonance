//! Transient interaction state (selection, drag/trim handles, the open
//! MIDI editor) and the MIDI editor's own viewport.

use std::collections::BTreeSet;

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
    pub midi_clip_drag: Option<MidiClipDragState>,
    pub midi_clip_trim: Option<MidiClipTrimState>,
    pub editing_midi_clip: Option<MidiEditorState>,
    /// Currently selected event on a global track (tempo or signature).
    pub selected_global_event: Option<SelectedGlobalEvent>,
}
