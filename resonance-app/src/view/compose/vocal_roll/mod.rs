//! Vocal roll — the piano-roll-style editor that opens when a vocal lane
//! is double-clicked.
//!
//! Mirrors `crate::view::midi_editor::PianoRollCanvas` but tuned for vocals:
//!
//! * **Warm accent** on note bodies (matches the rest of the vocal UI).
//! * **Chord context strip** above the grid (read-only) — the singer
//!   reads the chord above the bar they're singing over.
//! * **Phoneme strip** under the chord strip — per-syllable ARPAbet
//!   phonemes from `g2p::phonemes_for_draft`, lined up with their note.
//! * **Lyric on each note body** — italic-serif syllable painted on the
//!   note rectangle, matching the design's "notes carry lyrics" pattern.
//! * **Slur arcs** between adjacent notes that flow without a rest —
//!   the visual cue for legato phoneme transitions in the SVS render.
//! * **Pitch curve overlay** — the rendered f0 path the SVS engine will
//!   sing: piecewise-constant note pitches, linear portamento between
//!   them, sinusoidal vibrato wobble on sustained notes. Lets the user
//!   audition portamento / vibrato changes visually.
//! * **Voice-type-bounded keyboard** — only renders the rows inside
//!   `params.range` so the editor doesn't waste vertical space on
//!   octaves the part will never touch.
//!
//! Edits go through the same `MidiEditorMessage::{Add,Remove,Move,Resize}`
//! the piano roll uses; the engine command path is shared.
//!
//! The canvas's concerns are split across files:
//!
//! - this file: [`VocalRollCanvas`] struct, small helpers, shared
//!   constants, and the [`VocalRollState`] / [`VocalRollFingerprint`]
//!   persistent state types.
//! - [`build`]: the [`build_canvas`] entry function — which definition
//!   and lane the opened clip belongs to, and what the canvas borrows.
//! - [`canvas_program`]: the [`canvas::Program`] impl that orchestrates
//!   per-event dispatch and per-frame drawing.
//! - [`draw`]: the per-frame orchestrator (`draw_into`), the cache
//!   `fingerprint`, and the small tick/y coordinate helpers shared with
//!   the event handlers.
//! - [`grid`]: structural backdrop — note-row stripes, bar/beat lines,
//!   chord strip, phoneme strip.
//! - [`notes`]: note overlays — note rectangles, slur arcs, pitch
//!   curve, stress contour, and the velocity lane.
//! - [`keyboard`]: piano keyboard column on the left edge.

use iced::widget::canvas;

use resonance_audio::types::TrackId;
use resonance_music_theory::VocalParams;

use crate::compose::ChordState;
use crate::state::MidiClipState;

mod build;
mod canvas_program;
mod draw;
mod grid;
mod keyboard;
mod notes;

pub use build::{build_canvas, lane_for_clip};

/// Width of the piano keyboard column.
pub const VR_KEYBOARD_WIDTH: f32 = 56.0;
/// Height of the chord-context strip above the note grid.
pub const VR_CHORD_STRIP_HEIGHT: f32 = 28.0;
/// Height of the phoneme strip directly above the note grid.
pub const VR_PHONEME_STRIP_HEIGHT: f32 = 22.0;
/// Height of the velocity lane below the note grid.
pub const VR_VELOCITY_LANE_HEIGHT: f32 = 44.0;
/// Stacked header height — chord + phoneme strips.
pub(super) const HEADER_TOTAL_HEIGHT: f32 = VR_CHORD_STRIP_HEIGHT + VR_PHONEME_STRIP_HEIGHT;
/// Minimum resize threshold in pixels for the right edge of a note.
pub(super) const RESIZE_EDGE_PX: f32 = 6.0;
/// Default velocity for newly drawn notes.
pub(super) const DEFAULT_VELOCITY: f32 = 0.8;

/// Returns true if the given MIDI note number corresponds to a black key.
pub(super) fn is_black_key(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

/// "C4", "F#3", ...
pub(super) fn note_name(note: u8) -> String {
    resonance_music_theory::midi_note_name(note)
}

/// Format the chord as a short label (e.g. "Bm", "F#7", "Bb" in a flat
/// key) — root and quality only, root spelled with `spelling`.
pub(super) fn chord_label(
    chord: &resonance_music_theory::Chord,
    spelling: resonance_music_theory::Spelling,
) -> String {
    format!("{}{}", chord.root.name(spelling), chord.quality)
}

/// Snapshot of everything the canvas needs to render and respond to
/// input. Built fresh from the app state every frame; the persistent
/// drag/preview state lives in `VocalRollState`.
#[derive(Debug)]
pub struct VocalRollCanvas<'a> {
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
    pub params: &'a VocalParams,
    /// The lane's bank: its dictionary and inventory shape the phoneme
    /// strip.
    pub bank: crate::compose::vocal_svs::BankProfile,
    /// Section chords + total beats — used to draw the read-only chord
    /// context strip above the note grid.
    pub chords: &'a [ChordState],
    /// Accidental the chord strip spells roots with — the section's key
    /// (BUG-72).
    pub chord_spelling: resonance_music_theory::Spelling,
    pub section_beats: u32,
    pub scroll_y: f32,
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub snap_ticks: u64,
    pub selected_note: Option<usize>,
    pub time_sig_num: u8,
    /// The section's BPM (from the tempo map at the section). Plumbed in
    /// so the pitch-curve preview's portamento + vibrato match what the
    /// SVS pipeline will produce at the same tempo.
    pub bpm: f32,
    /// Voice name shown in the top-left meta corner.
    pub voice_label: &'a str,
    /// Per-note lyrics from `compose.vocal_audio.clip_lyrics`. Index i
    /// aligns with the i-th `MidiNote`. Slurs are identified by the lyric
    /// being equal to [`resonance_music_theory::g2p::SLUR_MARKER`]
    /// (`"+"`).
    pub lyrics: &'a [String],
}

#[derive(Debug, Clone)]
pub(super) enum DragMode {
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
    ResizeNote {
        note_index: usize,
        anchor_tick: u64,
    },
}

/// Local canvas state — only drag + preview live here; everything else
/// is read off `Resonance` every paint.
#[derive(Debug, Default)]
pub struct VocalRollState {
    pub(super) drag: Option<DragMode>,
    pub(super) previewing_note: Option<u8>,
    /// Whether the vocal roll owns the keyboard (last mouse press landed
    /// on it) — gates Delete and the `s`/`+` slur toggle so typing into a
    /// lyric field never edits the selected note.
    pub(super) key_focus: crate::focus::KeyFocus,
    pub(super) cache: canvas::Cache,
    pub(super) cache_fingerprint: std::cell::Cell<VocalRollFingerprint>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VocalRollFingerprint {
    pub(super) clip_id: u64,
    pub(super) notes_len: usize,
    pub(super) notes_hash: u64,
    pub(super) scroll_y_bits: u32,
    pub(super) zoom_x_bits: u32,
    pub(super) zoom_y_bits: u32,
    pub(super) snap_ticks: u64,
    pub(super) selected_note: Option<usize>,
    pub(super) time_sig_num: u8,
    pub(super) drag_active: bool,
    pub(super) preview_note: Option<u8>,
    pub(super) range_lo: u8,
    pub(super) range_hi: u8,
    pub(super) chords_hash: u64,
    pub(super) draft_hash: u64,
    pub(super) lyrics_hash: u64,
    /// Voice label + voicebank (VIEW-31).
    pub(super) meta_hash: u64,
    /// Fields that affect the pitch-curve overlay. Tracked so dragging
    /// the portamento / vibrato sliders in the right rail invalidates
    /// the canvas cache and repaints the curve immediately.
    pub(super) bpm_bits: u32,
    pub(super) portamento_ms_bits: u32,
    pub(super) vibrato_bits: u32,
    pub(super) vibrato_rate_bits: u32,
}
