/// Piano roll MIDI editor canvas for the Resonance DAW.
///
/// This module is a thin dispatcher: the interaction state machine lives in
/// [`input`] and the full rendering pipeline in [`draw`]; this file only
/// glues them together through the [`canvas::Program`] implementation and
/// holds the shared data types both submodules depend on.
pub mod draw;
pub mod input;

// Re-export the public helpers used by tests and the editor panel.
pub use input::{ghost_targets, quantize_grid_steps};

use iced::widget::canvas;
use iced::{mouse, Rectangle, Renderer, Theme};

use std::collections::BTreeSet;

use crate::message::*;
use crate::view::piano_roll::{self, PianoRollLayout, PianoRollViewport};
use crate::state::MidiClipState;

use resonance_audio::quantize::{Division, GridModifier, QuantizeMode};
use resonance_audio::types::{MidiNote, TempoMap, TrackId};

// ── Public constants ─────────────────────────────────────────────────────────

/// Width of the piano keyboard area on the left side of the editor.
pub const KEYBOARD_WIDTH: f32 = 50.0;
/// Height of the velocity lane at the bottom of the editor.
pub(crate) const VELOCITY_LANE_HEIGHT: f32 = 40.0;
/// Default velocity for newly created notes.
pub(crate) const DEFAULT_VELOCITY: f32 = 0.8;

// ── Shared data types ────────────────────────────────────────────────────────

/// The active Quantize-panel settings, projected into the piano roll so it
/// can draw the live quantize grid and the non-destructive "ghost" target
/// preview for the current selection (todo #396). A plain `Copy` snapshot of
/// [`MidiQuantizePanelState`](crate::state::MidiQuantizePanelState) — the
/// canvas never mutates it.
#[derive(Debug, Clone, Copy)]
pub struct QuantizePreview {
    /// Grid the notes snap to (carries triplet / dotted feel).
    pub division: Division,
    /// Blend toward the grid, `0.0..=1.0`.
    pub strength: f32,
    /// Swing applied to odd grid steps, `0.0..=1.0`.
    pub swing: f32,
    /// Whether starts only, or starts + length, are quantized.
    pub mode: QuantizeMode,
    /// Snap note-offs to the grid as well as note-ons.
    pub quantize_ends: bool,
    /// Apply the strength blend iteratively (soft quantize).
    pub iterative: bool,
}

/// Data passed to the piano roll canvas for rendering.
#[derive(Debug)]
pub struct PianoRollCanvas<'a> {
    /// The active keymap: canvas-local keys resolve through it
    /// (command-palette.md §4.3).
    pub keymap: &'a crate::commands::BindingMap,
    /// Set while any modal overlay (the palette included) is open or the
    /// Keyboard panel is capturing a chord: the canvas ignores key presses,
    /// so Backspace typed into a dialog can't delete the selection this
    /// canvas owned the keys for (command-palette.md §7.3). See
    /// `Resonance::canvas_keys_blocked`.
    pub keys_blocked: bool,
    pub clip: &'a MidiClipState,
    pub track_id: TrackId,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub snap_ticks: u64,
    pub selected_notes: &'a BTreeSet<usize>,
    pub time_sig_num: u8,
    /// Live Quantize-panel settings driving the grid + ghost overlay.
    pub quantize: QuantizePreview,
    /// Project tempo / signature map, anchoring the quantize grid lines and
    /// the ghost-target snap so odd meters land correctly.
    pub tempo_map: &'a TempoMap,
}

// ── Internal drag-state types ────────────────────────────────────────────────

/// Interaction mode being tracked during a drag operation.
#[derive(Debug, Clone)]
pub(crate) enum DragMode {
    /// Moving a note: (note_index, tick_offset_from_cursor).
    MoveNote {
        note_index: usize,
        start_tick_offset: i64,
        /// The clip's notes as the engine holds them, replayed through
        /// every move this drag sent (`move_note_resorted`). A move
        /// re-sorts the clip, so after crossing a neighbour `note_index`
        /// is re-read from here — the pressed index would name the
        /// neighbour (code review VIEW-02).
        notes: Vec<resonance_audio::types::MidiNote>,
    },
    /// Resizing a note from its right edge.
    ResizeNote { note_index: usize, anchor_tick: u64 },
}

/// Minimum cursor travel (in pixels) before a left-press on empty grid is
/// treated as a rubber-band marquee rather than a click that creates a note.
pub(crate) const MARQUEE_THRESHOLD_PX: f32 = 4.0;

/// A left-press that started on empty grid space. It resolves on release:
/// a negligible drag is a click (creates a note at `create_*`), a longer
/// drag is a marquee selection over the swept rectangle.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EmptyDrag {
    /// Canvas-local press point.
    pub(crate) origin: iced::Point,
    /// Canvas-local current cursor point.
    pub(crate) current: iced::Point,
    /// The note + snapped start tick to create if this turns out to be a
    /// plain click rather than a marquee drag.
    pub(crate) create_note: u8,
    pub(crate) create_tick: u64,
}

impl EmptyDrag {
    /// Whether the cursor has travelled far enough to count as a marquee.
    pub(crate) fn is_marquee(&self) -> bool {
        (self.current.x - self.origin.x).abs() >= MARQUEE_THRESHOLD_PX
            || (self.current.y - self.origin.y).abs() >= MARQUEE_THRESHOLD_PX
    }

    /// The swept (normalised) rectangle, in canvas-local coordinates.
    pub(crate) fn rect(&self) -> Rectangle {
        piano_roll::rect_from_points(self.origin, self.current)
    }
}

// ── Canvas state ─────────────────────────────────────────────────────────────

/// Local state for the piano roll canvas, tracking drags and previews.
#[derive(Debug, Default)]
pub struct PianoRollState {
    pub(crate) drag: Option<DragMode>,
    pub(crate) previewing_note: Option<u8>,
    /// Active empty-grid press: either a pending note-create or an
    /// in-progress rubber-band marquee (see [`EmptyDrag`]).
    pub(crate) empty_drag: Option<EmptyDrag>,
    /// Latest keyboard modifier state, tracked from `ModifiersChanged`
    /// events so mouse presses can tell shift/ctrl-click from a plain click.
    pub(crate) modifiers: iced::keyboard::Modifiers,
    /// Whether the piano roll owns the keyboard (last mouse press landed
    /// on it) — gates Delete / Ctrl+A so they never fire from another
    /// surface or while a text field is being typed into.
    pub(crate) key_focus: crate::focus::KeyFocus,
    /// Cached drawn geometry — invalidated only when the fingerprint of the
    /// inputs (notes / scroll / zoom / selection / clip identity) changes.
    pub(crate) cache: canvas::Cache,
    pub(crate) cache_fingerprint: std::cell::Cell<PianoRollFingerprint>,
}

/// Minimal projection of the piano roll's inputs into a comparable value.
/// The draw routine asks for the current fingerprint, compares it with what
/// was used for the cached geometry, and only re-runs the drawing closure
/// when something visible has actually changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PianoRollFingerprint {
    pub clip_id: u64,
    pub notes_len: usize,
    /// Hash of the (note, start_tick, duration_ticks, velocity) tuples so
    /// an edit inside the clip invalidates the cache even when `notes_len`
    /// doesn't change.
    pub notes_hash: u64,
    pub scroll_x_bits: u32,
    pub scroll_y_bits: u32,
    pub zoom_x_bits: u32,
    pub zoom_y_bits: u32,
    pub snap_ticks: u64,
    pub selected_notes_hash: u64,
    pub time_sig_num: u8,
    pub drag_active: bool,
    pub preview_note: Option<u8>,
    /// Hash of the active quantize settings (grid / strength / swing /
    /// mode / ends / iterative) so the cached grid + ghost overlay
    /// invalidate the moment the user adjusts the Quantize panel.
    pub quantize_hash: u64,
}

// ── Helper functions used by both submodules ─────────────────────────────────

/// Hash the selection set into a single comparable value for the draw cache
/// fingerprint. `BTreeSet` iterates in sorted order, so equal selections
/// always hash equally.
fn hash_selection(set: &BTreeSet<usize>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    set.len().hash(&mut h);
    for i in set {
        i.hash(&mut h);
    }
    h.finish()
}

/// Small stable discriminant for a grid modifier, used in the cache
/// fingerprint (the enum doesn't derive `Hash`).
fn modifier_code(m: GridModifier) -> u8 {
    match m {
        GridModifier::Straight => 0,
        GridModifier::Triplet => 1,
        GridModifier::Dotted => 2,
    }
}

// ── PianoRollCanvas helper methods ───────────────────────────────────────────

impl PianoRollCanvas<'_> {
    /// Layout for the bottom-panel piano roll: keyboard on the left,
    /// no toolbar, velocity lane below the grid.
    pub(crate) fn layout(&self, bounds: Rectangle) -> PianoRollLayout {
        PianoRollLayout {
            keyboard_w: KEYBOARD_WIDTH,
            grid_top: 0.0,
            grid_h: bounds.height - VELOCITY_LANE_HEIGHT,
        }
    }

    pub(crate) fn viewport(&self) -> PianoRollViewport {
        PianoRollViewport {
            zoom_x: self.zoom_x,
            zoom_y: self.zoom_y,
            scroll_x: self.scroll_x,
            scroll_y: self.scroll_y,
        }
    }

    /// Pixel rectangle for `note`, in canvas-local coordinates.
    pub(crate) fn note_rect(
        &self,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
        note: &MidiNote,
    ) -> Rectangle {
        Rectangle {
            x: layout.grid_x() + viewport.tick_to_x_local(note.start_tick),
            y: layout.grid_top + viewport.note_to_y_local(note.note),
            width: viewport.duration_to_w(note.duration_ticks),
            height: viewport.zoom_y,
        }
    }

    /// Snap a tick value to the nearest grid position.
    pub(crate) fn snap(&self, tick: u64) -> u64 {
        if self.snap_ticks == 0 {
            return tick;
        }
        let half = self.snap_ticks / 2;
        ((tick + half) / self.snap_ticks) * self.snap_ticks
    }

    /// Hash of the inputs that affect the drawn geometry. Excludes
    /// `bounds.size()` because the cache invalidates on size change
    /// automatically (via `canvas::Cache::draw`), so adding it here would
    /// double the work during a resize.
    pub(crate) fn fingerprint(&self, state: &PianoRollState) -> PianoRollFingerprint {
        use std::hash::{Hash, Hasher};
        let mut nh = std::collections::hash_map::DefaultHasher::new();
        for n in self.clip.notes.iter() {
            n.note.hash(&mut nh);
            n.start_tick.hash(&mut nh);
            n.duration_ticks.hash(&mut nh);
            n.velocity.to_bits().hash(&mut nh);
        }
        PianoRollFingerprint {
            clip_id: self.clip.id,
            notes_len: self.clip.notes.len(),
            notes_hash: nh.finish(),
            scroll_x_bits: self.scroll_x.to_bits(),
            scroll_y_bits: self.scroll_y.to_bits(),
            zoom_x_bits: self.zoom_x.to_bits(),
            zoom_y_bits: self.zoom_y.to_bits(),
            snap_ticks: self.snap_ticks,
            selected_notes_hash: hash_selection(self.selected_notes),
            time_sig_num: self.time_sig_num,
            drag_active: state.drag.is_some(),
            preview_note: state.previewing_note,
            quantize_hash: self.quantize_hash(),
        }
    }

    /// Fold the active quantize settings into one comparable value for the
    /// draw-cache fingerprint. The quantize enums don't derive `Hash`, so
    /// they're folded in via their tick size / a small discriminant rather
    /// than `Hash`. Selection already lives in `selected_notes_hash`.
    pub(crate) fn quantize_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let q = &self.quantize;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        q.division.ticks().hash(&mut h);
        modifier_code(q.division.modifier).hash(&mut h);
        q.strength.to_bits().hash(&mut h);
        q.swing.to_bits().hash(&mut h);
        matches!(q.mode, QuantizeMode::StartAndLength).hash(&mut h);
        q.quantize_ends.hash(&mut h);
        q.iterative.hash(&mut h);
        h.finish()
    }
}

// ── canvas::Program — thin dispatcher ────────────────────────────────────────

impl canvas::Program<Message> for PianoRollCanvas<'_> {
    type State = PianoRollState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        input::handle_event(self, state, event, bounds, cursor)
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        draw::draw_canvas(self, state, renderer, bounds, theme)
    }
}
