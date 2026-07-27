//! Instrument-track creation and MIDI clip / note CRUD. All handlers
//! mutate the engine-side clip table and emit a matching `AudioEvent`
//! so the app can mirror the change.
//!
//! Move/trim handlers fold mutation and event emission into a single
//! `if let Some(clip) = ...` branch so a missing clip lookup never
//! emits a ghost event. The pure inner helpers (`move_midi_clip_in_place`,
//! `trim_midi_clip_in_place`) are re-exported under `__test_support` so
//! the regression test in `tests/midi_clip_handlers.rs` can drive them
//! without bringing up the engine thread.

use crossbeam_channel::Sender;
use parking_lot::RwLock;

use crate::quantize::{
    apply_groove, extract_groove, humanize_notes, quantize_notes, Division, GrooveTemplate,
    QuantizeMode,
};
use crate::types::*;

use super::super::thread::{HandlerCtx, HandlerState};

pub(crate) fn handle_add_instrument_track(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    id_hint: Option<TrackId>,
    name: Option<String>,
) {
    let id = id_hint.unwrap_or_else(|| {
        let i = state.next_track_id;
        state.next_track_id += 1;
        i
    });
    if id_hint.is_some() {
        state.next_track_id = state.next_track_id.max(id + 1);
    }
    let name = name.unwrap_or_else(|| format!("Instrument {}", id));
    let track = Track::with_type(id, name, TrackType::Instrument);
    ctx.tracks.write().insert(id, track);
    let _ = ctx
        .event_tx
        .send(AudioEvent::InstrumentTrackAdded { track_id: id });
}

/// Same as `handle_add_instrument_track` but creates the track with
/// `TrackType::Vocal` so the view layer can route it to the vocal lane.
/// Live MIDI input still works (vocal accepts MIDI for staff capture);
/// playback runs through the audio-clip path.
pub(crate) fn handle_add_vocal_track(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    id_hint: Option<TrackId>,
    name: Option<String>,
) {
    let id = id_hint.unwrap_or_else(|| {
        let i = state.next_track_id;
        state.next_track_id += 1;
        i
    });
    if id_hint.is_some() {
        state.next_track_id = state.next_track_id.max(id + 1);
    }
    let name = name.unwrap_or_else(|| format!("Vocal {}", id));
    let track = Track::with_type(id, name, TrackType::Vocal);
    ctx.tracks.write().insert(id, track);
    let _ = ctx
        .event_tx
        .send(AudioEvent::VocalTrackAdded { track_id: id });
}

pub(crate) fn handle_create_midi_clip(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    start_sample: u64,
    duration_ticks: u64,
    name: String,
) {
    let clip_id = state.next_clip_id;
    state.next_clip_id += 1;
    let clip = MidiClip {
        id: clip_id,
        track_id,
        start_sample,
        duration_ticks,
        notes: Vec::new(),
        name: name.clone(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    };
    ctx.midi_clips.write().push(clip);
    let _ = ctx.event_tx.send(AudioEvent::MidiClipCreated {
        clip_id,
        track_id,
        start_sample,
        duration_ticks,
        name,
        notes: Vec::new(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_load_midi_clip_direct(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    clip_id: ClipId,
    track_id: TrackId,
    start_sample: u64,
    duration_ticks: u64,
    notes: Vec<MidiNote>,
    name: String,
    trim_start_ticks: u64,
    trim_end_ticks: u64,
) {
    let clip = MidiClip {
        id: clip_id,
        track_id,
        start_sample,
        duration_ticks,
        notes: notes.clone(),
        name: name.clone(),
        trim_start_ticks,
        trim_end_ticks,
    };
    ctx.midi_clips.write().push(clip);
    state.next_clip_id = state.next_clip_id.max(clip_id + 1);
    let _ = ctx.event_tx.send(AudioEvent::MidiClipCreated {
        clip_id,
        track_id,
        start_sample,
        duration_ticks,
        name,
        notes,
        trim_start_ticks,
        trim_end_ticks,
    });
}

pub(crate) fn handle_move_midi_clip(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    new_start_sample: u64,
    new_track_id: TrackId,
) {
    move_midi_clip_in_place(
        ctx.midi_clips,
        ctx.event_tx,
        clip_id,
        new_start_sample,
        new_track_id,
    );
}

pub(crate) fn handle_trim_midi_clip(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    new_start_sample: u64,
    trim_start_ticks: u64,
    trim_end_ticks: u64,
) {
    trim_midi_clip_in_place(
        ctx.midi_clips,
        ctx.event_tx,
        clip_id,
        new_start_sample,
        trim_start_ticks,
        trim_end_ticks,
    );
}

/// Apply a move to the MIDI clip with `clip_id` and emit `MidiClipMoved`.
///
/// Both the mutation and the event live inside the `if let Some(clip)`
/// branch, so when the clip lookup misses no event is emitted — mirroring
/// the audio-clip move handler in [`super::super::clips::handle_move_clip`].
/// Previously the handler used a `let-else` with early `return`, which was
/// also correct but obscured the invariant; the canonical pattern keeps
/// "mutated state and event are inseparable" syntactically obvious.
pub fn move_midi_clip_in_place(
    midi_clips: &RwLock<Vec<MidiClip>>,
    event_tx: &Sender<AudioEvent>,
    clip_id: ClipId,
    new_start_sample: u64,
    new_track_id: TrackId,
) {
    let mut guard = midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.track_id = new_track_id;
        let _ = event_tx.send(AudioEvent::MidiClipMoved {
            clip_id,
            new_start_sample,
            new_track_id,
        });
    }
}

/// Apply a trim to the MIDI clip with `clip_id` and emit `MidiClipTrimmed`.
///
/// Same invariant as [`move_midi_clip_in_place`]: mutation and event both
/// live inside the `if let Some` branch so a missing-clip lookup never
/// emits a ghost event. Mirrors
/// [`super::super::clips::handle_trim_clip`].
pub fn trim_midi_clip_in_place(
    midi_clips: &RwLock<Vec<MidiClip>>,
    event_tx: &Sender<AudioEvent>,
    clip_id: ClipId,
    new_start_sample: u64,
    trim_start_ticks: u64,
    trim_end_ticks: u64,
) {
    let mut guard = midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.trim_start_ticks = trim_start_ticks;
        clip.trim_end_ticks = trim_end_ticks;
        let _ = event_tx.send(AudioEvent::MidiClipTrimmed {
            clip_id,
            new_start_sample,
            trim_start_ticks,
            trim_end_ticks,
        });
    }
}

pub(crate) fn handle_delete_midi_clip(ctx: &HandlerCtx, clip_id: ClipId) {
    ctx.midi_clips.write().retain(|c| c.id != clip_id);
    let _ = ctx.event_tx.send(AudioEvent::MidiClipDeleted { clip_id });
}

pub(crate) fn handle_add_midi_note(ctx: &HandlerCtx, clip_id: ClipId, note: MidiNote) {
    let mut guard = ctx.midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        let n = note.clone();
        // Insert sorted by start_tick
        let pos = clip
            .notes
            .partition_point(|n| n.start_tick <= note.start_tick);
        clip.notes.insert(pos, note);
        let _ = ctx
            .event_tx
            .send(AudioEvent::MidiNoteAdded { clip_id, note: n });
    }
}

pub(crate) fn handle_remove_midi_note(ctx: &HandlerCtx, clip_id: ClipId, note_index: usize) {
    let mut guard = ctx.midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        if note_index < clip.notes.len() {
            clip.notes.remove(note_index);
            let _ = ctx.event_tx.send(AudioEvent::MidiNoteRemoved {
                clip_id,
                note_index,
            });
        }
    }
}

pub(crate) fn handle_move_midi_note(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    note_index: usize,
    new_start_tick: u64,
    new_note: u8,
) {
    let mut guard = ctx.midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        if note_index < clip.notes.len() {
            clip.notes[note_index].start_tick = new_start_tick;
            clip.notes[note_index].note = new_note;
            // Re-sort
            clip.notes.sort_by_key(|n| n.start_tick);
            let _ = ctx.event_tx.send(AudioEvent::MidiNoteMoved {
                clip_id,
                note_index,
                new_start_tick,
                new_note,
            });
        }
    }
}

pub(crate) fn handle_resize_midi_note(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    note_index: usize,
    new_duration_ticks: u64,
) {
    let mut guard = ctx.midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        if note_index < clip.notes.len() {
            clip.notes[note_index].duration_ticks = new_duration_ticks;
            let _ = ctx.event_tx.send(AudioEvent::MidiNoteResized {
                clip_id,
                note_index,
                new_duration_ticks,
            });
        }
    }
}

pub(crate) fn handle_set_midi_note_velocity(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    note_index: usize,
    velocity: f32,
) {
    let mut guard = ctx.midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        if note_index < clip.notes.len() {
            clip.notes[note_index].velocity = velocity;
            let _ = ctx.event_tx.send(AudioEvent::MidiNoteVelocitySet {
                clip_id,
                note_index,
                velocity,
            });
        }
    }
}

// -- Bulk MIDI note edits: quantize / humanize / groove --
//
// Each bulk op is atomic: it locks the clip table once, applies the pure
// `quantize` algorithm against the clip's notes (and, where the geometry
// needs it, the engine's authoritative `TempoMap`), replaces the clip's
// note array, and emits exactly ONE `AudioEvent::MidiNotesEdited` carrying
// the full resulting note array. The app mirrors that array and records
// the prior notes for a single-step undo. Note order is preserved — the
// pure algorithms operate strictly by index and never reorder, merge, or
// drop notes, so engine and app mirrors stay index-aligned.
//
// As with the move/trim handlers, mutation and event emission are folded
// into a single `if let Some(clip)` branch so a missing-clip lookup is a
// no-op that emits no ghost event. The inner `*_in_place` helpers are
// re-exported under `__test_support` (via `lib.rs`) so the regression
// tests in `tests/` can drive them headlessly — no engine thread.

/// Engine-thread handler for [`AudioCommand::QuantizeMidiNotes`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_quantize_midi_notes(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    indices: Vec<usize>,
    grid: Division,
    strength: f32,
    swing: f32,
    mode: QuantizeMode,
    quantize_ends: bool,
    iterative: bool,
) {
    let tempo = ctx.tempo_map.load();
    quantize_midi_notes_in_place(
        ctx.midi_clips,
        ctx.event_tx,
        &tempo,
        ctx.sample_rate,
        clip_id,
        &indices,
        grid,
        strength,
        swing,
        mode,
        quantize_ends,
        iterative,
    );
}

/// Engine-thread handler for [`AudioCommand::HumanizeMidiNotes`].
pub(crate) fn handle_humanize_midi_notes(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    indices: Vec<usize>,
    timing_ticks: u32,
    vel_amt: f32,
    seed: u64,
) {
    humanize_midi_notes_in_place(
        ctx.midi_clips,
        ctx.event_tx,
        clip_id,
        &indices,
        timing_ticks,
        vel_amt,
        seed,
    );
}

/// Engine-thread handler for [`AudioCommand::ApplyGrooveToClip`].
pub(crate) fn handle_apply_groove_to_clip(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    indices: Vec<usize>,
    template: GrooveTemplate,
    strength: f32,
) {
    let tempo = ctx.tempo_map.load();
    apply_groove_to_clip_in_place(
        ctx.midi_clips,
        ctx.event_tx,
        &tempo,
        clip_id,
        &indices,
        &template,
        strength,
    );
}

/// Engine-thread handler for [`AudioCommand::ExtractGrooveFromClip`].
pub(crate) fn handle_extract_groove_from_clip(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    grid: Division,
) {
    let tempo = ctx.tempo_map.load();
    extract_groove_from_clip_in_place(ctx.midi_clips, ctx.event_tx, &tempo, clip_id, grid);
}

/// Quantize the selected notes in `clip_id` and emit one bulk
/// `MidiNotesEdited`. No-op (and no event) if the clip is missing.
///
/// The grid is aligned to the project bar lines via the clip's absolute
/// start tick (`sample_to_abs_tick(start_sample) - trim_start_ticks`),
/// matching the playback projection in `outbound.rs`, so trimmed clips
/// and clips that do not begin on a bar boundary quantize correctly.
#[allow(clippy::too_many_arguments)]
pub fn quantize_midi_notes_in_place(
    midi_clips: &RwLock<Vec<MidiClip>>,
    event_tx: &Sender<AudioEvent>,
    tempo: &TempoMap,
    sample_rate: u32,
    clip_id: ClipId,
    indices: &[usize],
    grid: Division,
    strength: f32,
    swing: f32,
    mode: QuantizeMode,
    quantize_ends: bool,
    iterative: bool,
) {
    let mut guard = midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        let clip_start_tick = tempo
            .sample_to_abs_tick(clip.start_sample, sample_rate)
            .saturating_sub(clip.trim_start_ticks);
        let new_notes = quantize_notes(
            &clip.notes,
            indices,
            grid,
            strength,
            swing,
            mode,
            quantize_ends,
            iterative,
            tempo,
            clip_start_tick,
        );
        clip.notes = new_notes.clone();
        let _ = event_tx.send(AudioEvent::MidiNotesEdited {
            clip_id,
            notes: new_notes,
        });
    }
}

/// Humanize the selected notes in `clip_id` and emit one bulk
/// `MidiNotesEdited`. No-op (and no event) if the clip is missing.
pub fn humanize_midi_notes_in_place(
    midi_clips: &RwLock<Vec<MidiClip>>,
    event_tx: &Sender<AudioEvent>,
    clip_id: ClipId,
    indices: &[usize],
    timing_ticks: u32,
    vel_amt: f32,
    seed: u64,
) {
    let mut guard = midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        let new_notes = humanize_notes(&clip.notes, indices, timing_ticks, vel_amt, seed);
        clip.notes = new_notes.clone();
        let _ = event_tx.send(AudioEvent::MidiNotesEdited {
            clip_id,
            notes: new_notes,
        });
    }
}

/// Apply a groove template to the selected notes in `clip_id` and emit
/// one bulk `MidiNotesEdited`. No-op (and no event) if the clip is
/// missing.
pub fn apply_groove_to_clip_in_place(
    midi_clips: &RwLock<Vec<MidiClip>>,
    event_tx: &Sender<AudioEvent>,
    tempo: &TempoMap,
    clip_id: ClipId,
    indices: &[usize],
    template: &GrooveTemplate,
    strength: f32,
) {
    let mut guard = midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        let new_notes = apply_groove(&clip.notes, indices, template, strength, tempo);
        clip.notes = new_notes.clone();
        let _ = event_tx.send(AudioEvent::MidiNotesEdited {
            clip_id,
            notes: new_notes,
        });
    }
}

/// Extract a groove template from `clip_id` at `grid` resolution and emit
/// `GrooveExtracted`. Read-only: the clip is not modified. No-op (and no
/// event) if the clip is missing.
pub fn extract_groove_from_clip_in_place(
    midi_clips: &RwLock<Vec<MidiClip>>,
    event_tx: &Sender<AudioEvent>,
    tempo: &TempoMap,
    clip_id: ClipId,
    grid: Division,
) {
    let guard = midi_clips.read();
    if let Some(clip) = guard.iter().find(|c| c.id == clip_id) {
        let template = extract_groove(&clip.notes, grid, tempo);
        let _ = event_tx.send(AudioEvent::GrooveExtracted { template });
    }
}
