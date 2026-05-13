//! App-side handlers for MIDI clip + note events from the engine.

use resonance_audio::types::*;

use crate::state::MidiClipState;
use crate::Resonance;

#[allow(clippy::too_many_arguments)]
pub(super) fn clip_created(
    r: &mut Resonance,
    clip_id: ClipId,
    track_id: TrackId,
    start_sample: SamplePos,
    duration_ticks: u64,
    name: String,
    notes: Vec<MidiNote>,
    trim_start_ticks: u64,
    trim_end_ticks: u64,
) {
    // Idempotent: skip if the MIDI clip already exists (created by project load).
    if r.midi_clips.iter().any(|c| c.id == clip_id) {
        return;
    }
    r.midi_clips.push(MidiClipState {
        id: clip_id,
        track_id,
        start_sample,
        duration_ticks,
        name,
        notes,
        trim_start_ticks,
        trim_end_ticks,
    });
}

pub(super) fn clip_moved(
    r: &mut Resonance,
    clip_id: ClipId,
    new_start_sample: SamplePos,
    new_track_id: TrackId,
) {
    if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.track_id = new_track_id;
    }
}

pub(super) fn clip_trimmed(
    r: &mut Resonance,
    clip_id: ClipId,
    new_start_sample: SamplePos,
    trim_start_ticks: u64,
    trim_end_ticks: u64,
) {
    if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.trim_start_ticks = trim_start_ticks;
        clip.trim_end_ticks = trim_end_ticks;
    }
}

pub(super) fn clip_deleted(r: &mut Resonance, clip_id: ClipId) {
    r.midi_clips.retain(|c| c.id != clip_id);
}

pub(super) fn note_added(r: &mut Resonance, clip_id: ClipId, note: MidiNote) {
    if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
        let pos = clip
            .notes
            .partition_point(|n| n.start_tick <= note.start_tick);
        clip.notes.insert(pos, note);
    }
}

pub(super) fn note_removed(r: &mut Resonance, clip_id: ClipId, note_index: usize) {
    if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
        if note_index < clip.notes.len() {
            clip.notes.remove(note_index);
        }
    }
}

pub(super) fn note_moved(
    r: &mut Resonance,
    clip_id: ClipId,
    note_index: usize,
    new_start_tick: u64,
    new_note: u8,
) {
    if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
        if note_index < clip.notes.len() {
            clip.notes[note_index].start_tick = new_start_tick;
            clip.notes[note_index].note = new_note;
            clip.notes.sort_by_key(|n| n.start_tick);
        }
    }
}

pub(super) fn note_resized(
    r: &mut Resonance,
    clip_id: ClipId,
    note_index: usize,
    new_duration_ticks: u64,
) {
    if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
        if note_index < clip.notes.len() {
            clip.notes[note_index].duration_ticks = new_duration_ticks;
        }
    }
}

pub(super) fn note_velocity_set(
    r: &mut Resonance,
    clip_id: ClipId,
    note_index: usize,
    velocity: f32,
) {
    if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
        if note_index < clip.notes.len() {
            clip.notes[note_index].velocity = velocity;
        }
    }
}
