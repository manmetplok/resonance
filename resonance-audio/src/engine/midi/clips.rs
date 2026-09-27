//! Instrument-track creation and MIDI clip / note CRUD. All handlers
//! mutate the engine-side clip table and emit a matching `AudioEvent`
//! so the app can mirror the change.
//!
//! Move/trim handlers fold mutation and event emission into a single
//! `if let Some(clip) = ...` branch so a missing clip lookup never
//! emits a ghost event. The pure inner helpers (`move_midi_clip_in_place`,
//! `trim_midi_clip_in_place`) are re-exported under `test_support` so
//! the regression test in `tests/engine/midi_clip_handlers.rs` can drive them
//! without bringing up the engine thread.

use std::sync::Arc;

use crossbeam_channel::Sender;

use crate::quantize::{
    apply_groove, extract_groove, humanize_notes, quantize_notes, Division, GrooveTemplate,
    QuantizeMode,
};
use crate::types::*;

use super::super::thread::{HandlerCtx, HandlerState};
use super::super::SharedState;
use super::super::tracks::reject_if_track_id_in_use;

pub(crate) fn handle_add_instrument_track(ctx: &HandlerCtx, id: TrackId, name: Option<String>) {
    if reject_if_track_id_in_use(ctx, id) {
        return;
    }
    let name = name.unwrap_or_else(|| format!("Instrument {}", id));
    let track = Arc::new(Track::with_type(id, name, TrackType::Instrument));
    ctx.shared.edit_tracks(|tracks| tracks.insert(id, track));
    let _ = ctx
        .event_tx
        .send(AudioEvent::InstrumentTrackAdded { track_id: id });
}

/// Same as `handle_add_instrument_track` but creates the track with
/// `TrackType::Vocal` so the view layer can route it to the vocal lane.
/// Live MIDI input still works (vocal accepts MIDI for staff capture);
/// playback runs through the audio-clip path.
pub(crate) fn handle_add_vocal_track(ctx: &HandlerCtx, id: TrackId, name: Option<String>) {
    if reject_if_track_id_in_use(ctx, id) {
        return;
    }
    let name = name.unwrap_or_else(|| format!("Vocal {}", id));
    let track = Arc::new(Track::with_type(id, name, TrackType::Vocal));
    ctx.shared.edit_tracks(|tracks| tracks.insert(id, track));
    let _ = ctx
        .event_tx
        .send(AudioEvent::VocalTrackAdded { track_id: id });
}

/// `clip_id` is mandatory (D-7c): the app allocates it and the engine no
/// longer touches `next_clip_id` for this command (that counter still
/// serves recording until D-7d). A collision is refused rather than
/// silently replacing the live clip — see `reject_if_clip_id_in_use`.
pub(crate) fn handle_create_midi_clip(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    track_id: TrackId,
    start_sample: u64,
    duration_ticks: u64,
    name: String,
) {
    if super::super::clips::reject_if_clip_id_in_use(ctx, clip_id) {
        return;
    }
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
    ctx.shared.edit_midi_clips(|clips| clips.push(Arc::new(clip)));
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
    ctx.shared.edit_midi_clips(|clips| clips.push(Arc::new(clip)));
    crate::engine::clips::reserve_clip_id(&mut state.next_clip_id, clip_id);
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
        ctx.shared,
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
        ctx.shared,
        ctx.event_tx,
        clip_id,
        new_start_sample,
        trim_start_ticks,
        trim_end_ticks,
    );
}

/// Apply a move to the MIDI clip with `clip_id`, publish the new render
/// graph, and emit `MidiClipMoved`.
///
/// The event is sent only when the edit found the clip (the edit returns
/// `Some`), so when the clip lookup misses nothing is published and no
/// event is emitted — mirroring the audio-clip move handler in
/// [`super::super::clips::handle_move_clip`].
pub fn move_midi_clip_in_place(
    shared: &SharedState,
    event_tx: &Sender<AudioEvent>,
    clip_id: ClipId,
    new_start_sample: u64,
    new_track_id: TrackId,
) {
    let moved = shared.edit_midi_clip(clip_id, |clip| {
        clip.start_sample = new_start_sample;
        clip.track_id = new_track_id;
    });
    if moved.is_some() {
        let _ = event_tx.send(AudioEvent::MidiClipMoved {
            clip_id,
            new_start_sample,
            new_track_id,
        });
    }
}

/// Apply a trim to the MIDI clip with `clip_id` and emit `MidiClipTrimmed`.
///
/// Same invariant as [`move_midi_clip_in_place`]: the event follows a
/// successful edit only, so a missing-clip lookup never emits a ghost
/// event. Mirrors
/// [`super::super::clips::handle_trim_clip`].
pub fn trim_midi_clip_in_place(
    shared: &SharedState,
    event_tx: &Sender<AudioEvent>,
    clip_id: ClipId,
    new_start_sample: u64,
    trim_start_ticks: u64,
    trim_end_ticks: u64,
) {
    let trimmed = shared.edit_midi_clip(clip_id, |clip| {
        clip.start_sample = new_start_sample;
        clip.trim_start_ticks = trim_start_ticks;
        clip.trim_end_ticks = trim_end_ticks;
    });
    if trimmed.is_some() {
        let _ = event_tx.send(AudioEvent::MidiClipTrimmed {
            clip_id,
            new_start_sample,
            trim_start_ticks,
            trim_end_ticks,
        });
    }
}

pub(crate) fn handle_delete_midi_clip(ctx: &HandlerCtx, clip_id: ClipId) {
    ctx.shared.edit_midi_clips(|clips| clips.retain(|c| c.id != clip_id));
    let _ = ctx.event_tx.send(AudioEvent::MidiClipDeleted { clip_id });
}

pub(crate) fn handle_add_midi_note(ctx: &HandlerCtx, clip_id: ClipId, note: MidiNote) {
    let echo = note.clone();
    let added = ctx.shared.edit_midi_clip(clip_id, |clip| {
        // Insert sorted by start_tick
        let pos = clip
            .notes
            .partition_point(|n| n.start_tick <= note.start_tick);
        clip.notes.insert(pos, note);
    });
    if added.is_some() {
        let _ = ctx
            .event_tx
            .send(AudioEvent::MidiNoteAdded { clip_id, note: echo });
    }
}

pub(crate) fn handle_remove_midi_note(ctx: &HandlerCtx, clip_id: ClipId, note_index: usize) {
    if note_in_range(ctx.shared, clip_id, note_index) {
        ctx.shared.edit_midi_clip(clip_id, |clip| {
            clip.notes.remove(note_index);
        });
        let _ = ctx.event_tx.send(AudioEvent::MidiNoteRemoved {
            clip_id,
            note_index,
        });
    }
}

pub(crate) fn handle_move_midi_note(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    note_index: usize,
    new_start_tick: u64,
    new_note: u8,
) {
    if note_in_range(ctx.shared, clip_id, note_index) {
        // The echo carries the ORIGINAL index: the app mirror replays
        // the same `move_note_resorted`, landing on the same order.
        ctx.shared.edit_midi_clip(clip_id, |clip| {
            move_note_resorted(&mut clip.notes, note_index, new_start_tick, new_note);
        });
        let _ = ctx.event_tx.send(AudioEvent::MidiNoteMoved {
            clip_id,
            note_index,
            new_start_tick,
            new_note,
        });
    }
}

pub(crate) fn handle_resize_midi_note(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    note_index: usize,
    new_duration_ticks: u64,
) {
    if note_in_range(ctx.shared, clip_id, note_index) {
        ctx.shared.edit_midi_clip(clip_id, |clip| {
            clip.notes[note_index].duration_ticks = new_duration_ticks;
        });
        let _ = ctx.event_tx.send(AudioEvent::MidiNoteResized {
            clip_id,
            note_index,
            new_duration_ticks,
        });
    }
}

pub(crate) fn handle_set_midi_note_velocity(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    note_index: usize,
    velocity: f32,
) {
    if note_in_range(ctx.shared, clip_id, note_index) {
        ctx.shared.edit_midi_clip(clip_id, |clip| {
            clip.notes[note_index].velocity = velocity;
        });
        let _ = ctx.event_tx.send(AudioEvent::MidiNoteVelocitySet {
            clip_id,
            note_index,
            velocity,
        });
    }
}

/// Whether `clip_id` exists and has a note at `note_index` — checked on
/// the published graph first, so an out-of-range edit publishes nothing.
/// Every writer runs on the engine thread, so the answer still holds
/// when the edit that follows runs.
fn note_in_range(shared: &SharedState, clip_id: ClipId, note_index: usize) -> bool {
    shared
        .graph
        .load()
        .midi_clip(clip_id)
        .is_some_and(|clip| note_index < clip.notes.len())
}

// -- Bulk MIDI note edits: quantize / humanize / groove --
//
// Each bulk op is atomic: it reads the clip from the published graph,
// applies the pure
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
// re-exported under `test_support` (via `lib.rs`) so the regression
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
        ctx.shared,
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
        ctx.shared,
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
        ctx.shared,
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
    extract_groove_from_clip_in_place(ctx.shared, ctx.event_tx, &tempo, clip_id, grid);
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
    shared: &SharedState,
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
    // Compute from the published graph, then publish the result as one
    // edit (code review ARCH-02 A2-3/A2-4). Every writer of the MIDI
    // clips runs on the engine thread, so nothing interleaves between
    // the read and the edit.
    let Some((notes, clip_start_tick)) = read_notes(shared, clip_id, |clip| {
        tempo
            .sample_to_abs_tick(clip.start_sample, sample_rate)
            .saturating_sub(clip.trim_start_ticks)
    }) else {
        return;
    };
    let new_notes = quantize_notes(
        &notes,
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
    replace_notes_and_echo(shared, event_tx, clip_id, new_notes);
}

/// Copy `clip_id`'s notes (plus whatever `extra` reads off the clip) from
/// the published graph. `None` if the clip is missing.
fn read_notes<T>(
    shared: &SharedState,
    clip_id: ClipId,
    extra: impl FnOnce(&MidiClip) -> T,
) -> Option<(Vec<MidiNote>, T)> {
    let graph = shared.graph.load();
    let clip = graph.midi_clip(clip_id)?;
    Some((clip.notes.clone(), extra(clip)))
}

/// Install `notes` on `clip_id` and publish, then echo them as
/// `MidiNotesEdited`. No event if the clip is missing.
fn replace_notes_and_echo(
    shared: &SharedState,
    event_tx: &Sender<AudioEvent>,
    clip_id: ClipId,
    notes: Vec<MidiNote>,
) {
    let echo = notes.clone();
    if shared
        .edit_midi_clip(clip_id, |clip| clip.notes = notes)
        .is_none()
    {
        return;
    }
    let _ = event_tx.send(AudioEvent::MidiNotesEdited {
        clip_id,
        notes: echo,
    });
}

/// Engine-thread handler for [`AudioCommand::SetMidiClipNotes`]:
/// replace a clip's whole note array atomically and echo the result as
/// one `MidiNotesEdited` (ba doc #269 FR-5). The caller supplies the
/// final array — merging and sorting are the app's job, so the engine
/// stays a dumb, single-writer store. No-op (and no event) if the clip
/// is missing.
pub(crate) fn handle_set_midi_clip_notes(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    notes: Vec<MidiNote>,
) {
    replace_notes_and_echo(ctx.shared, ctx.event_tx, clip_id, notes);
}

/// Humanize the selected notes in `clip_id` and emit one bulk
/// `MidiNotesEdited`. No-op (and no event) if the clip is missing.
pub fn humanize_midi_notes_in_place(
    shared: &SharedState,
    event_tx: &Sender<AudioEvent>,
    clip_id: ClipId,
    indices: &[usize],
    timing_ticks: u32,
    vel_amt: f32,
    seed: u64,
) {
    // Off-lock compute; see `quantize_midi_notes_in_place`.
    let Some((notes, ())) = read_notes(shared, clip_id, |_| ()) else {
        return;
    };
    let new_notes = humanize_notes(&notes, indices, timing_ticks, vel_amt, seed);
    replace_notes_and_echo(shared, event_tx, clip_id, new_notes);
}

/// Apply a groove template to the selected notes in `clip_id` and emit
/// one bulk `MidiNotesEdited`. No-op (and no event) if the clip is
/// missing.
pub fn apply_groove_to_clip_in_place(
    shared: &SharedState,
    event_tx: &Sender<AudioEvent>,
    tempo: &TempoMap,
    clip_id: ClipId,
    indices: &[usize],
    template: &GrooveTemplate,
    strength: f32,
) {
    // Off-lock compute; see `quantize_midi_notes_in_place`.
    let Some((notes, ())) = read_notes(shared, clip_id, |_| ()) else {
        return;
    };
    let new_notes = apply_groove(&notes, indices, template, strength, tempo);
    replace_notes_and_echo(shared, event_tx, clip_id, new_notes);
}

/// Extract a groove template from `clip_id` at `grid` resolution and emit
/// `GrooveExtracted`. Read-only: the clip is not modified. No-op (and no
/// event) if the clip is missing.
pub fn extract_groove_from_clip_in_place(
    shared: &SharedState,
    event_tx: &Sender<AudioEvent>,
    tempo: &TempoMap,
    clip_id: ClipId,
    grid: Division,
) {
    let graph = shared.graph.load();
    if let Some(clip) = graph.midi_clip(clip_id) {
        let template = extract_groove(&clip.notes, grid, tempo);
        let _ = event_tx.send(AudioEvent::GrooveExtracted { template });
    }
}
