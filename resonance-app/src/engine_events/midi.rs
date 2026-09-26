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
    // The load of a clip whose deletion echo is still owed (ARCH-01
    // A-13i): FIFO puts it before that deletion, which the mirror already
    // reflects — mirroring it would push a phantom.
    if r.io.restore_echoes.midi_clip_deletion_owed(clip_id) {
        return;
    }
    // Idempotent: skip if the MIDI clip already exists (created by project load).
    if r.midi_clips.iter().any(|c| c.id == clip_id) {
        return;
    }
    // While recording, a new clip is the one a live MIDI recording opens
    // on its first note: an undoable edit, snapshotted before it lands
    // (STATE-02). Outside recording it echoes an app-issued edit whose
    // message already recorded its own entry.
    if r.transport.recording {
        r.record_recording_edit();
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
    // An edit of the instance whose deletion is still owed (A-13i).
    if r.io.restore_echoes.midi_clip_deletion_owed(clip_id) {
        return;
    }
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
    if r.io.restore_echoes.midi_clip_deletion_owed(clip_id) {
        return;
    }
    if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.trim_start_ticks = trim_start_ticks;
        clip.trim_end_ticks = trim_end_ticks;
    }
}

/// The `MidiClipDeleted` echo: swallowed when a diff restore or a live
/// delete already mirrored it (ARCH-01 A-13i), mirrored otherwise.
pub(super) fn clip_deleted_echo(r: &mut Resonance, clip_id: ClipId) {
    if r.io.restore_echoes.settle_midi_clip_deleted(clip_id) {
        return;
    }
    clip_deleted(r, clip_id);
}

pub(super) fn clip_deleted(r: &mut Resonance, clip_id: ClipId) {
    r.midi_clips.retain(|c| c.id != clip_id);
    // Drop the lyric side-table entry — keeping it would only leak
    // memory and risk collisions if a future clip is allocated the
    // same id.
    r.compose.vocal_audio.clip_lyrics.remove(&clip_id);
}

pub(super) fn note_added(r: &mut Resonance, clip_id: ClipId, note: MidiNote) {
    // Read-your-own-writes (Bug 2b): a control `notes.insert` already
    // mirrored this note into `midi_clips` at handler time and left a
    // matching `Added` token. Drain it and skip so the note isn't added
    // twice. GUI edits leave no token, so their echo inserts as before.
    if take_pending_added(r, clip_id, &note) {
        return;
    }
    insert_note_sorted(r, clip_id, note);
}

/// Insert `note` into `clip_id` keeping the note vector sorted by
/// `start_tick`, and keep the parallel lyric side-table index-aligned.
/// Shared by the `MidiNoteAdded` echo and the optimistic control mirror.
pub(crate) fn insert_note_sorted(r: &mut Resonance, clip_id: ClipId, note: MidiNote) -> usize {
    let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) else {
        return 0;
    };
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
        debug_assert_eq!(lyrics.len(), r.midi_clips.iter().find(|c| c.id == clip_id).map_or(pos, |c| c.notes.len()));
    }
    pos
}

pub(super) fn note_removed(r: &mut Resonance, clip_id: ClipId, note_index: usize) {
    // Read-your-own-writes (Bug 2b): a control `notes.delete` already
    // removed this note at handler time; drain its `Removed` token and
    // skip so the echo doesn't remove a second (now index-shifted) note.
    if take_pending_removed(r, clip_id) {
        return;
    }
    remove_note_at(r, clip_id, note_index);
}

/// Remove the note at `note_index` from `clip_id`, keeping the lyric
/// side-table aligned. Shared by the echo and the optimistic mirror.
pub(crate) fn remove_note_at(r: &mut Resonance, clip_id: ClipId, note_index: usize) {
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

// ---------------------------------------------------------------------------
// Control read-your-own-writes token queue (ba doc #265, Bug 2b)
// ---------------------------------------------------------------------------

use crate::state::{note_matches, PendingNoteEcho};

/// Enqueue a pending-echo token for a control-originated optimistic edit.
pub(crate) fn push_pending(r: &mut Resonance, clip_id: ClipId, token: PendingNoteEcho) {
    r.control_pending_note_echoes
        .entry(clip_id)
        .or_default()
        .push_back(token);
}

/// Drain the front token for `clip_id` when the predicate accepts it.
/// Returns true when a token was consumed.
fn take_pending_if(
    r: &mut Resonance,
    clip_id: ClipId,
    accept: impl Fn(&PendingNoteEcho) -> bool,
) -> bool {
    if let Some(queue) = r.control_pending_note_echoes.get_mut(&clip_id) {
        if queue.front().is_some_and(|t| accept(t)) {
            queue.pop_front();
            if queue.is_empty() {
                r.control_pending_note_echoes.remove(&clip_id);
            }
            return true;
        }
    }
    false
}

/// Drain the front token for `clip_id` when it is an `Added` whose note
/// equals `note` (value match — the engine echoes the exact note the
/// control handler optimistically inserted). Returns true when consumed.
fn take_pending_added(r: &mut Resonance, clip_id: ClipId, note: &MidiNote) -> bool {
    if let Some(queue) = r.control_pending_note_echoes.get_mut(&clip_id) {
        if let Some(PendingNoteEcho::Added(pending)) = queue.front() {
            if note_matches(pending, note) {
                queue.pop_front();
                if queue.is_empty() {
                    r.control_pending_note_echoes.remove(&clip_id);
                }
                return true;
            }
        }
    }
    false
}

/// Drain the front `Updated` token for `clip_id` (move/resize/velocity).
/// Returns true when consumed.
fn take_pending_updated(r: &mut Resonance, clip_id: ClipId) -> bool {
    take_pending_if(r, clip_id, |t| matches!(t, PendingNoteEcho::Updated))
}

/// Drain the front `Removed` token for `clip_id`. Returns true when
/// consumed.
fn take_pending_removed(r: &mut Resonance, clip_id: ClipId) -> bool {
    take_pending_if(r, clip_id, |t| matches!(t, PendingNoteEcho::Removed))
}

/// Drain the front `Replaced` token for `clip_id` (a bulk control
/// write). Returns true when consumed.
fn take_pending_replaced(r: &mut Resonance, clip_id: ClipId) -> bool {
    take_pending_if(r, clip_id, |t| matches!(t, PendingNoteEcho::Replaced))
}

// ---------------------------------------------------------------------------
// Optimistic control mirror (ba doc #265, Bug 2b): apply an edit to
// `midi_clips` synchronously AND record the token its engine echo will
// drain, so a control `notes.*` mutation is visible to the next request
// without racing the round trip. Each returns the same value the reply
// would report (an insert index where relevant).
// ---------------------------------------------------------------------------

/// Mirror a control `notes.insert`: insert the note now and enqueue the
/// `Added` token. Returns the sorted index it landed at.
pub(crate) fn optimistic_add_note(r: &mut Resonance, clip_id: ClipId, note: MidiNote) -> usize {
    let index = insert_note_sorted(r, clip_id, note.clone());
    push_pending(r, clip_id, PendingNoteEcho::Added(note));
    index
}

/// Mirror a control `notes.delete`: remove the note now and enqueue the
/// `Removed` token.
pub(crate) fn optimistic_remove_note(r: &mut Resonance, clip_id: ClipId, note_index: usize) {
    remove_note_at(r, clip_id, note_index);
    push_pending(r, clip_id, PendingNoteEcho::Removed);
}

/// Mirror a control `notes.edit` move and enqueue the `Updated` token.
pub(crate) fn optimistic_move_note(
    r: &mut Resonance,
    clip_id: ClipId,
    note_index: usize,
    new_start_tick: u64,
    new_note: u8,
) {
    apply_note_move(r, clip_id, note_index, new_start_tick, new_note);
    push_pending(r, clip_id, PendingNoteEcho::Updated);
}

/// Mirror a control `notes.edit` resize and enqueue the `Updated` token.
pub(crate) fn optimistic_resize_note(
    r: &mut Resonance,
    clip_id: ClipId,
    note_index: usize,
    new_duration_ticks: u64,
) {
    apply_note_resize(r, clip_id, note_index, new_duration_ticks);
    push_pending(r, clip_id, PendingNoteEcho::Updated);
}

/// Mirror a control `notes.insert_many` / `notes.replace_all`: make
/// `notes` the clip's note array now and enqueue the `Replaced` token.
/// `notes` must already be sorted by `start_tick` — the same order the
/// single-note inserts maintain, so reported indices stay meaningful.
/// The lyric side-table is padded/truncated to match, as the bulk echo
/// handler does.
pub(crate) fn optimistic_set_notes(r: &mut Resonance, clip_id: ClipId, notes: Vec<MidiNote>) {
    if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
        let new_len = notes.len();
        clip.notes = notes;
        if let Some(lyrics) = r.compose.vocal_audio.clip_lyrics.get_mut(&clip_id) {
            lyrics.resize(new_len, String::new());
        }
    }
    push_pending(r, clip_id, PendingNoteEcho::Replaced);
}

/// Mirror a control `notes.edit` velocity change and enqueue the token.
pub(crate) fn optimistic_set_velocity(
    r: &mut Resonance,
    clip_id: ClipId,
    note_index: usize,
    velocity: f32,
) {
    apply_note_velocity(r, clip_id, note_index, velocity);
    push_pending(r, clip_id, PendingNoteEcho::Updated);
}

pub(super) fn note_moved(
    r: &mut Resonance,
    clip_id: ClipId,
    note_index: usize,
    new_start_tick: u64,
    new_note: u8,
) {
    // A control `notes.edit` move already applied this optimistically;
    // re-applying the same value would be harmless, but drain the token
    // and skip to keep the queue in step (Bug 2b).
    if take_pending_updated(r, clip_id) {
        return;
    }
    apply_note_move(r, clip_id, note_index, new_start_tick, new_note);
}

/// Move the `note_index`-th note of `clip_id` to `(new_start_tick,
/// new_note)`, re-sorting by start_tick and permuting the lyric
/// side-table to match. Shared by the echo and the optimistic mirror.
pub(crate) fn apply_note_move(
    r: &mut Resonance,
    clip_id: ClipId,
    note_index: usize,
    new_start_tick: u64,
    new_note: u8,
) {
    if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
        if note_index < clip.notes.len() {
            // The notes vec needs to stay sorted by start_tick, exactly as
            // the engine re-sorts it (`move_note_resorted`). The lyric
            // side-table and the editor's note selection are indexed
            // parallel to `notes`, so we permute them the same way: the
            // same stable sort over the post-move start ticks.
            let mut ticks: Vec<u64> = clip.notes.iter().map(|n| n.start_tick).collect();
            ticks[note_index] = new_start_tick;
            move_note_resorted(&mut clip.notes, note_index, new_start_tick, new_note);
            let mut perm: Vec<usize> = (0..ticks.len()).collect();
            perm.sort_by_key(|&i| ticks[i]);
            // perm[new_i] == old_i.
            if let Some(lyrics) = r.compose.vocal_audio.clip_lyrics.get_mut(&clip_id) {
                if lyrics.len() == perm.len() {
                    // Build new lyrics vec via gather.
                    let new_lyrics: Vec<String> =
                        perm.iter().map(|&i| lyrics[i].clone()).collect();
                    *lyrics = new_lyrics;
                }
            }
            // Keep the selection on the same notes, so the dragged note
            // stays highlighted and Delete removes it, not its neighbour.
            if let Some(editor) = r
                .ui
                .interaction
                .editing_midi_clip
                .as_mut()
                .filter(|e| e.clip_id == clip_id)
            {
                let mut old_to_new = vec![0; perm.len()];
                for (new_i, &old_i) in perm.iter().enumerate() {
                    old_to_new[old_i] = new_i;
                }
                editor.selected_notes = editor
                    .selected_notes
                    .iter()
                    .filter_map(|&old| old_to_new.get(old).copied())
                    .collect();
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
    if take_pending_updated(r, clip_id) {
        return;
    }
    apply_note_resize(r, clip_id, note_index, new_duration_ticks);
}

/// Set the duration of the `note_index`-th note of `clip_id`. Shared by
/// the echo and the optimistic mirror.
pub(crate) fn apply_note_resize(
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
    if take_pending_updated(r, clip_id) {
        return;
    }
    apply_note_velocity(r, clip_id, note_index, velocity);
}

/// Set the velocity of the `note_index`-th note of `clip_id`. Shared by
/// the echo and the optimistic mirror.
pub(crate) fn apply_note_velocity(
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
    // A control `notes.insert_many` / `notes.replace_all` already applied
    // this array optimistically (doc #269 FR-5). Drain its token and
    // skip: the echo carries the engine's state from before any *later*
    // mirrored single-note edit, so re-applying it would roll that back.
    if take_pending_replaced(r, clip_id) {
        return;
    }
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
///
/// The capture is filed into the **project** groove library
/// ([`QuantizeState::groove_library`](crate::state::QuantizeState)) as a
/// named [`UserGroove`](crate::state::UserGroove) so it persists, rides
/// the undo snapshot, and shows up in the apply picker (#394/#395). The
/// name is the one the user typed into the Extract field
/// ([`MidiQuantizePanelState::pending_groove_name`](crate::state::MidiQuantizePanelState)),
/// or an auto-numbered default when that was blank.
pub(super) fn groove_extracted(r: &mut Resonance, template: GrooveTemplate) {
    let id = r.quantize.next_groove_id();
    let name = r
        .midi_quantize
        .pending_groove_name
        .take()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("Groove {}", id + 1));
    r.quantize.groove_library.push(crate::state::UserGroove {
        id,
        name,
        template,
    });
}

/// Send `DeleteMidiClip` for a clip the caller drops from the mirror
/// itself, right now, and owe its `MidiClipDeleted` echo (ARCH-01 A-13i):
/// an undo may re-add the clip under this id before the echo lands. The
/// engine echoes every MIDI delete, known id or not.
pub(crate) fn send_mirrored_delete(r: &mut Resonance, clip_id: ClipId) {
    let _ = r.engine.send(AudioCommand::DeleteMidiClip { clip_id });
    r.io.restore_echoes.expect_midi_clip_deleted(clip_id);
}
