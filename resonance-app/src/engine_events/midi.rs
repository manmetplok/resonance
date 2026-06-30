//! App-side handlers for MIDI clip + note events from the engine.

use resonance_audio::quantize::GrooveTemplate;
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
    // Drop the lyric side-table entry — keeping it would only leak
    // memory and risk collisions if a future clip is allocated the
    // same id.
    r.compose.vocal_audio.clip_lyrics.remove(&clip_id);
}

pub(super) fn note_added(r: &mut Resonance, clip_id: ClipId, note: MidiNote) {
    if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
        let pos = clip
            .notes
            .partition_point(|n| n.start_tick <= note.start_tick);
        clip.notes.insert(pos, note);
        // Keep the lyric side-table aligned — insert a blank lyric
        // at the same index so subsequent indices still reference the
        // right note. If the side-table is shorter than the notes vec
        // (e.g. a clip created by raw `AddMidiNote` before any vocal
        // edit), pad with empty strings up to `pos` first so the
        // newly inserted entry lands at the correct position and the
        // post-insert length matches `clip.notes.len()`.
        if let Some(lyrics) = r.compose.vocal_audio.clip_lyrics.get_mut(&clip_id) {
            if lyrics.len() < pos {
                lyrics.resize(pos, String::new());
            }
            lyrics.insert(pos, String::new());
            // Post-condition: lyrics.len() == clip.notes.len().
            debug_assert_eq!(lyrics.len(), clip.notes.len());
        }
    }
}

pub(super) fn note_removed(r: &mut Resonance, clip_id: ClipId, note_index: usize) {
    if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
        if note_index < clip.notes.len() {
            clip.notes.remove(note_index);
            if let Some(lyrics) = r.compose.vocal_audio.clip_lyrics.get_mut(&clip_id) {
                if note_index < lyrics.len() {
                    lyrics.remove(note_index);
                }
            }
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
            // The notes vec needs to stay sorted by start_tick. The
            // lyric side-table is indexed parallel to `notes`, so we
            // permute it the same way. Build an index permutation
            // from the pre-sort order, sort, then apply it.
            let pre: Vec<(u64, u8)> =
                clip.notes.iter().map(|n| (n.start_tick, n.note)).collect();
            clip.notes.sort_by_key(|n| n.start_tick);
            if let Some(lyrics) = r.compose.vocal_audio.clip_lyrics.get_mut(&clip_id) {
                if lyrics.len() == pre.len() {
                    let mut perm: Vec<usize> = (0..pre.len()).collect();
                    perm.sort_by_key(|&i| pre[i].0);
                    // perm[new_i] == old_i. Build new lyrics vec via
                    // gather.
                    let new_lyrics: Vec<String> =
                        perm.iter().map(|&i| lyrics[i].clone()).collect();
                    *lyrics = new_lyrics;
                }
            }
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

/// Mirror a bulk MIDI edit (quantize / humanize / groove) into app
/// state. The engine sends one `MidiNotesEdited` carrying the **full
/// resulting note array** for the clip, so we replace the clip's note
/// vector wholesale — no per-note event churn.
///
/// These operations work by index and never reorder, merge, or drop
/// notes, so the parallel lyric side-table stays index-aligned. We
/// nevertheless reconcile its length defensively: if the new note count
/// differs (a future op that adds/removes notes, or a clip whose lyric
/// table was never populated), pad with blanks / truncate so
/// `lyrics.len() == notes.len()` holds afterwards.
pub(super) fn notes_edited(r: &mut Resonance, clip_id: ClipId, notes: Vec<MidiNote>) {
    if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
        let new_len = notes.len();
        clip.notes = notes;
        if let Some(lyrics) = r.compose.vocal_audio.clip_lyrics.get_mut(&clip_id) {
            if lyrics.len() != new_len {
                lyrics.resize(new_len, String::new());
            }
            debug_assert_eq!(lyrics.len(), clip.notes.len());
        }
    }
}

/// Add a groove template extracted from a clip to the app-side groove
/// library. Extraction is read-only on the engine side — no clip is
/// modified — so this handler only grows the library.
pub(super) fn groove_extracted(r: &mut Resonance, template: GrooveTemplate) {
    r.groove_library.push(template);
}
