//! Instrument-track creation, MIDI clip CRUD, per-note editing, and
//! live-MIDI note-on/note-off passthrough to the track's instrument
//! plugin.

use crate::types::*;

use super::thread::{HandlerCtx, HandlerState};

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
    let mut guard = ctx.midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.track_id = new_track_id;
    }
    let _ = ctx.event_tx.send(AudioEvent::MidiClipMoved {
        clip_id,
        new_start_sample,
        new_track_id,
    });
}

pub(crate) fn handle_trim_midi_clip(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    new_start_sample: u64,
    trim_start_ticks: u64,
    trim_end_ticks: u64,
) {
    let mut guard = ctx.midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.trim_start_ticks = trim_start_ticks;
        clip.trim_end_ticks = trim_end_ticks;
    }
    let _ = ctx.event_tx.send(AudioEvent::MidiClipTrimmed {
        clip_id,
        new_start_sample,
        trim_start_ticks,
        trim_end_ticks,
    });
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

pub(crate) fn handle_remove_midi_note(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    note_index: usize,
) {
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

pub(crate) fn handle_send_note_on(
    ctx: &HandlerCtx,
    track_id: TrackId,
    note: u8,
    velocity: f32,
) {
    let tracks_guard = ctx.tracks.read();
    if let Some(track) = tracks_guard.get(&track_id) {
        if track.track_type == TrackType::Instrument {
            if let Some(&inst_id) = track.plugin_ids.first() {
                let plugins_guard = ctx.plugins.read();
                if let Some(mutex) = plugins_guard.get(&inst_id) {
                    let mut inst = mutex.lock();
                    inst.0.queue_note_on(note, velocity, 0);
                }
            }
        }
    }
}

pub(crate) fn handle_send_note_off(ctx: &HandlerCtx, track_id: TrackId, note: u8) {
    let tracks_guard = ctx.tracks.read();
    if let Some(track) = tracks_guard.get(&track_id) {
        if track.track_type == TrackType::Instrument {
            if let Some(&inst_id) = track.plugin_ids.first() {
                let plugins_guard = ctx.plugins.read();
                if let Some(mutex) = plugins_guard.get(&inst_id) {
                    let mut inst = mutex.lock();
                    inst.0.queue_note_off(note, 0);
                }
            }
        }
    }
}
