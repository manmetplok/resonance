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
//! - this file: [`VocalRollCanvas`] struct, small helpers, the
//!   `build_canvas` entry function, shared constants, and the
//!   [`VocalRollState`] / [`VocalRollFingerprint`] persistent state types.
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

use crate::compose::{ChordState, SectionDefinitionState};
use crate::state::MidiClipState;

mod canvas_program;
mod draw;
mod grid;
mod keyboard;
mod notes;

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

/// Format the chord as a short label (e.g. "Bm", "F#7"). Uses the
/// `Display` impls already published by `resonance-music-theory`.
pub(super) fn chord_label(chord: &resonance_music_theory::Chord) -> String {
    format!("{}{}", chord.root, chord.quality)
}

/// Snapshot of everything the canvas needs to render and respond to
/// input. Built fresh from the app state every frame; the persistent
/// drag/preview state lives in `VocalRollState`.
#[derive(Debug)]
pub struct VocalRollCanvas<'a> {
    pub clip: &'a MidiClipState,
    pub track_id: TrackId,
    pub params: &'a VocalParams,
    /// Section chords + total beats — used to draw the read-only chord
    /// context strip above the note grid.
    pub chords: &'a [ChordState],
    pub section_beats: u32,
    pub scroll_y: f32,
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub snap_ticks: u64,
    pub selected_note: Option<usize>,
    pub time_sig_num: u8,
    /// Section BPM at the playhead. Plumbed in so the pitch-curve
    /// preview's portamento + vibrato match what the SVS pipeline
    /// will produce at the same tempo.
    pub bpm: f32,
    /// Voice name shown in the top-left meta corner.
    pub voice_label: &'a str,
    /// Per-note lyrics from `compose.vocal_audio.clip_lyrics`. Index i aligns
    /// with the i-th `MidiNote`. Slurs are identified by the lyric
    /// being equal to [`resonance_music_theory::g2p::SLUR_MARKER`]
    /// (`"+"`).
    pub lyrics: &'a [String],
}

/// Build a `VocalRollCanvas` from the `Compose` view's already-available
/// state. Returns `None` when the editing clip's track isn't a vocal
/// track or its section/definition can't be located.
pub fn build_canvas<'a>(
    app: &'a crate::Resonance,
    clip: &'a MidiClipState,
) -> Option<VocalRollCanvas<'a>> {
    let editor_state = app.interaction.editing_midi_clip.as_ref()?;
    let definition = find_definition_for_clip(app, clip)?;
    let params = find_vocal_params(definition, clip.track_id)?;
    let voice_label = params.voice.as_str();
    // Side-table lookup — falls back to an empty slice when the clip
    // has never had lyrics installed (e.g. a vocal clip created
    // through the engine without going through the generator). Callers
    // already pad to `clip.notes.len()` when they install, so when
    // present the slice length always matches the note count.
    static EMPTY_LYRICS: Vec<String> = Vec::new();
    let lyrics = app
        .compose
        .vocal_audio
        .clip_lyrics
        .get(&clip.id)
        .map(|v| v.as_slice())
        .unwrap_or(EMPTY_LYRICS.as_slice());
    Some(VocalRollCanvas {
        clip,
        track_id: editor_state.track_id,
        params,
        chords: &definition.chords,
        section_beats: definition.length_bars * app.transport.time_sig_num as u32,
        scroll_y: editor_state.scroll_y,
        zoom_x: editor_state.zoom_x,
        zoom_y: editor_state.zoom_y,
        snap_ticks: editor_state.snap_ticks,
        selected_note: editor_state.selected_note,
        time_sig_num: app.transport.time_sig_num,
        bpm: app.transport.bpm,
        voice_label,
        lyrics,
    })
}

/// Walk the compose state to find which section definition owns the
/// derived MIDI clip the user just opened. Falls back to scanning every
/// definition's lane generators for one keyed to the clip's track id.
fn find_definition_for_clip<'a>(
    app: &'a crate::Resonance,
    clip: &MidiClipState,
) -> Option<&'a SectionDefinitionState> {
    for ((def_id, _placement_id, track_id), cid) in app.compose.derived_clips.iter() {
        if *cid == clip.id && *track_id == clip.track_id {
            return app.compose.definitions.iter().find(|d| d.id == *def_id);
        }
    }
    // Fall back: any definition that has a vocal generator for this
    // track — this covers projects where the derived-clip map hasn't
    // been rebuilt yet (e.g. immediately after load).
    app.compose
        .definitions
        .iter()
        .find(|d| find_vocal_params(d, clip.track_id).is_some())
}

fn find_vocal_params(
    def: &SectionDefinitionState,
    track_id: TrackId,
) -> Option<&VocalParams> {
    use crate::compose::LaneGeneratorKind;
    def.lane_generators.get(&track_id).and_then(|cfg| match &cfg.kind {
        LaneGeneratorKind::Vocal(p) => Some(p),
        _ => None,
    })
}

#[derive(Debug, Clone)]
pub(super) enum DragMode {
    MoveNote {
        note_index: usize,
        start_tick_offset: i64,
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
    /// Fields that affect the pitch-curve overlay. Tracked so dragging
    /// the portamento / vibrato sliders in the right rail invalidates
    /// the canvas cache and repaints the curve immediately.
    pub(super) bpm_bits: u32,
    pub(super) portamento_ms_bits: u32,
    pub(super) vibrato_bits: u32,
    pub(super) vibrato_rate_bits: u32,
}
