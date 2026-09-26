//! `notes.*` control methods — piano-roll-level note editing (ba doc
//! #265, todo #1155).
//!
//! Notes are addressed by their `index` in a clip's note list (as
//! reported by `song.notes`). Every mutation synthesizes the existing
//! `MidiEditorMessage` (or, for `create_clip`,
//! `ComposeMessage::CreateMidiClipInSection` / a direct clip load)
//! through the FULL `update()` path via [`super::run_via_update`], so an
//! AI note edit is undoable exactly like the GUI. `notes.edit` fans a
//! multi-field change out into the matching move/resize/velocity edits.
//!
//! Async note: the app-side note vector is updated from engine events,
//! so an edit's effect isn't visible in the same dispatch. `notes.insert`
//! reports the index the note *will* occupy (computed the same way the
//! `MidiNoteAdded` handler inserts — `partition_point` on `start_tick`),
//! and `notes.create_clip` allocates the clip id app-side so the reply
//! returns it immediately.

use crate::message::{Message, MidiEditorMessage};
use crate::Resonance;
use iced::Task;
use crate::state::MidiClipState;
use resonance_audio::types::TrackType;
use resonance_control::methods::notes::{
    self, CreateClipParams, CreateClipResult, DeleteParams, EditParams, InsertManyParams,
    InsertManyResult, InsertParams, InsertResult, MoveClipParams, NoteSpec, ReplaceAllParams,
};
use resonance_control::{Request, Response, RpcError};

use super::reply::{ack, no_midi_clip, no_track, no_section_placement, reject};

/// Ticks per quarter note — the app's MIDI resolution.
const TPQ: f64 = resonance_audio::types::TICKS_PER_QUARTER_NOTE as f64;

/// Handle a `notes.*` request, or `None` when `method` belongs to
/// another namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let out = match request.method.as_str() {
        notes::INSERT => insert(app, request),
        notes::EDIT => edit(app, request),
        notes::DELETE => delete(app, request),
        notes::CREATE_CLIP => create_clip(app, request),
        notes::MOVE_CLIP => move_clip(app, request),
        notes::INSERT_MANY => insert_many(app, request),
        notes::REPLACE_ALL => replace_all(app, request),
        notes::IMPORT_MIDI => super::import_midi::handle(app, request),
        _ => return None,
    };
    Some(out)
}

/// The MIDI clip with `clip_id`, or `None`.
fn find_clip(app: &Resonance, clip_id: u64) -> Option<&MidiClipState> {
    app.midi_clips.iter().find(|c| c.id == clip_id)
}

/// A `busy` error when `track_id` is frozen (Frozen or Stale), else
/// `None`. Note edits on a frozen track are swallowed by the
/// `frozen_input_edit_target` gate in `update()` (which flips the freeze
/// to Stale and returns `Task::none()`), so acking success there would
/// falsely report an edit that never happened. The gate still backstops
/// the non-frozen path; this pre-check just makes the rejection visible
/// to the remote client (doc #265 DoD).
fn frozen_reject(app: &Resonance, track_id: resonance_audio::types::TrackId) -> Option<RpcError> {
    app.freeze.status(track_id).is_frozen().then(|| {
        RpcError::busy(format!(
            "track {track_id} is frozen; unfreeze it before editing its notes"
        ))
    })
}

/// Reject with `not_found`, distinguishing an audio clip (wrong kind)
/// from a genuinely missing id.
fn clip_not_found(app: &Resonance, request: &Request, clip_id: u64) -> (Response, Task<Message>) {
    if app.clips.iter().any(|c| c.id == clip_id) {
        return reject(
            request,
            RpcError::not_found(format!(
                "clip {clip_id} is an audio clip; notes.* edits MIDI clips"
            )),
        );
    }
    reject(request, no_midi_clip(clip_id))
}

/// Beats -> ticks, clamped non-negative and bounded by
/// [`notes::MAX_BEATS`]. The bound is what keeps the tick arithmetic
/// honest: an unbounded `start_beat` like `1e15` converts to ~1e18
/// ticks, and the engine's later `start_tick + duration_ticks` overflows
/// u64 — a panic in debug, a wrapped (corrupted) note written to the
/// project's `.mid` on save in release.
fn beats_to_ticks(beats: f64) -> Result<u64, RpcError> {
    if !beats.is_finite() || beats < 0.0 {
        return Err(RpcError::invalid_params(format!(
            "beat value must be finite and non-negative (got {beats})"
        )));
    }
    if beats > notes::MAX_BEATS {
        return Err(RpcError::invalid_params(format!(
            "beat value {beats} is over the {} limit",
            notes::MAX_BEATS as u64
        )));
    }
    Ok((beats * TPQ).round() as u64)
}

/// Validate a MIDI velocity `0..=127`.
fn check_velocity(velocity: u8) -> Result<(), RpcError> {
    // u8 already bounds 0..=255; MIDI is 0..=127.
    if velocity > 127 {
        return Err(RpcError::invalid_params(format!(
            "velocity {velocity} out of MIDI range 0..=127"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// notes.insert
// ---------------------------------------------------------------------------

fn insert(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: InsertParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(clip) = find_clip(app, params.clip_id.0) else {
        return clip_not_found(app, request, params.clip_id.0);
    };
    if let Some(e) = frozen_reject(app, clip.track_id) {
        return reject(request, e);
    }
    if params.pitch > 127 {
        return reject(
            request,
            RpcError::invalid_params(format!("pitch {} out of MIDI range 0..=127", params.pitch)),
        );
    }
    if let Err(e) = check_velocity(params.velocity) {
        return reject(request, e);
    }
    let start_tick = match beats_to_ticks(params.start_beat) {
        Ok(t) => t,
        Err(e) => return reject(request, e),
    };
    let duration_ticks = match beats_to_ticks(params.duration_beats) {
        Ok(t) => t,
        Err(e) => return reject(request, e),
    };
    if duration_ticks == 0 {
        return reject(request, RpcError::invalid_params("duration must be positive"));
    }

    let velocity = params.velocity as f32 / 127.0;
    let clip_id = params.clip_id.0;

    // Route the edit through the full update path (gates / undo / engine
    // command). The undo snapshot is taken pre-dispatch, so the note is
    // not yet in it.
    let task = super::run_via_update(
        app,
        Message::MidiEditor(MidiEditorMessage::AddNote {
            clip_id,
            note: params.pitch,
            start_tick,
            duration_ticks,
            velocity,
        }),
    );

    // Read-your-own-writes (Bug 2b): mirror the note into `app.midi_clips`
    // synchronously so the very next `song.notes` on this connection sees
    // it, instead of racing the async `MidiNoteAdded` echo (which is now
    // suppressed for this optimistic insert). The reported index is the
    // sorted position the mirror actually used.
    let index = crate::engine_events::midi::optimistic_add_note(
        app,
        clip_id,
        resonance_audio::types::MidiNote {
            note: params.pitch,
            velocity,
            start_tick,
            duration_ticks,
        },
    );
    let result = InsertResult {
        index,
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

// ---------------------------------------------------------------------------
// notes.edit
// ---------------------------------------------------------------------------

fn edit(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: EditParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(clip) = find_clip(app, params.clip_id.0) else {
        return clip_not_found(app, request, params.clip_id.0);
    };
    if let Some(e) = frozen_reject(app, clip.track_id) {
        return reject(request, e);
    }
    let Some(existing) = clip.notes.get(params.index).cloned() else {
        return reject(
            request,
            RpcError::not_found(format!(
                "clip {} has no note at index {} (it has {} note(s))",
                params.clip_id,
                params.index,
                clip.notes.len()
            )),
        );
    };
    if params.pitch.is_none()
        && params.start_beat.is_none()
        && params.duration_beats.is_none()
        && params.velocity.is_none()
    {
        return reject(request, RpcError::invalid_params("notes.edit changed nothing"));
    }
    if let Some(pitch) = params.pitch {
        if pitch > 127 {
            return reject(
                request,
                RpcError::invalid_params(format!("pitch {pitch} out of MIDI range 0..=127")),
            );
        }
    }
    if let Some(v) = params.velocity {
        if let Err(e) = check_velocity(v) {
            return reject(request, e);
        }
    }
    let start_tick = match params.start_beat.map(beats_to_ticks).transpose() {
        Ok(t) => t,
        Err(e) => return reject(request, e),
    };
    let duration_ticks = match params.duration_beats.map(beats_to_ticks).transpose() {
        Ok(t) => t,
        Err(e) => return reject(request, e),
    };
    if let Some(0) = duration_ticks {
        return reject(request, RpcError::invalid_params("duration must be positive"));
    }

    // Fan the multi-field edit out into the matching single-purpose
    // messages, grouped into ONE undoable transaction (the wire promises
    // one revision bump per call, and one edit_undo must take back the
    // whole call). Pitch + start share MoveNote; duration is ResizeNote;
    // velocity is SetNoteVelocity. Skip a sub-edit when its field is
    // unchanged so the engine traffic stays minimal.
    let clip_id = params.clip_id.0;
    let index = params.index;

    // Each dispatched sub-edit is also mirrored into `app.midi_clips`
    // synchronously (Bug 2b) so the change is visible to the next
    // `song.notes`; the matching echo is suppressed. The mirror uses the
    // same `note_index` as the dispatched message, faithful to the engine
    // command.
    //
    // The move goes LAST: it re-sorts the clip by start tick (engine and
    // mirror alike, `move_note_resorted`), after which `index` may name a
    // different note. Resize and velocity therefore address the note
    // while `index` is still its own (code review CTL-02).
    use crate::engine_events::midi;

    app.with_compound_undo(|app| {
        let mut tasks = Vec::new();
        if let Some(dur) = duration_ticks {
            if dur != existing.duration_ticks {
                tasks.push(super::run_via_update(
                    app,
                    Message::MidiEditor(MidiEditorMessage::ResizeNote {
                        clip_id,
                        note_index: index,
                        new_duration_ticks: dur,
                    }),
                ));
                midi::optimistic_resize_note(app, clip_id, index, dur);
            }
        }
        if let Some(v) = params.velocity {
            let want = v as f32 / 127.0;
            if (want - existing.velocity).abs() > f32::EPSILON {
                tasks.push(super::run_via_update(
                    app,
                    Message::MidiEditor(MidiEditorMessage::SetNoteVelocity {
                        clip_id,
                        note_index: index,
                        velocity: want,
                    }),
                ));
                midi::optimistic_set_velocity(app, clip_id, index, want);
            }
        }
        let want_pitch = params.pitch.unwrap_or(existing.note);
        let want_start = start_tick.unwrap_or(existing.start_tick);
        if (params.pitch.is_some() || start_tick.is_some())
            && (want_pitch != existing.note || want_start != existing.start_tick)
        {
            tasks.push(super::run_via_update(
                app,
                Message::MidiEditor(MidiEditorMessage::MoveNote {
                    clip_id,
                    note_index: index,
                    new_start_tick: want_start,
                    new_note: want_pitch,
                }),
            ));
            midi::optimistic_move_note(app, clip_id, index, want_start, want_pitch);
        }

        (ack(app, request), Task::batch(tasks))
    })
}

// ---------------------------------------------------------------------------
// notes.delete
// ---------------------------------------------------------------------------

fn delete(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: DeleteParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(clip) = find_clip(app, params.clip_id.0) else {
        return clip_not_found(app, request, params.clip_id.0);
    };
    if let Some(e) = frozen_reject(app, clip.track_id) {
        return reject(request, e);
    }
    if params.index >= clip.notes.len() {
        return reject(
            request,
            RpcError::not_found(format!(
                "clip {} has no note at index {} (it has {} note(s))",
                params.clip_id,
                params.index,
                clip.notes.len()
            )),
        );
    }
    let task = super::run_via_update(
        app,
        Message::MidiEditor(MidiEditorMessage::RemoveNote {
            clip_id: params.clip_id.0,
            note_index: params.index,
        }),
    );
    // Read-your-own-writes (Bug 2b): remove the note from `app.midi_clips`
    // now so the next `song.notes` reflects the deletion; the
    // `MidiNoteRemoved` echo is suppressed so it can't remove a second,
    // index-shifted note.
    crate::engine_events::midi::optimistic_remove_note(app, params.clip_id.0, params.index);
    (ack(app, request), task)
}

// ---------------------------------------------------------------------------
// notes.create_clip
// ---------------------------------------------------------------------------

fn create_clip(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: CreateClipParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(track) = app.registry.tracks.iter().find(|t| t.id == params.track_id.0) else {
        return reject(request, no_track(params.track_id.into()));
    };
    // MIDI clips only live on MIDI-capable tracks (instrument / vocal).
    if !matches!(track.track_type, TrackType::Instrument | TrackType::Vocal) {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "track {} is an audio track; MIDI clips need an instrument/vocal track",
                params.track_id
            )),
        );
    }

    // Resolve the clip's start + length. A placement anchors it to the
    // section's bars; otherwise `start_bar` (1-based) positions it, and
    // `length_beats` (or one bar) sets its length.
    let (start_sample, duration_ticks) = match params.placement_id {
        Some(pid) => {
            let Some(placement) =
                app.compose.placements.iter().find(|p| p.id == u64::from(pid))
            else {
                return reject(request, no_section_placement(pid.into()));
            };
            let Some(def) = app
                .compose
                .definitions
                .iter()
                .find(|d| d.id == placement.definition_id)
            else {
                return reject(
                    request,
                    RpcError::internal(format!(
                        "placement {pid} references a missing definition"
                    )),
                );
            };
            let start = app.tempo_map.bar_to_sample(placement.start_bar);
            let ticks = match params.length_beats {
                Some(b) => match beats_to_ticks(b) {
                    Ok(t) => t,
                    Err(e) => return reject(request, e),
                },
                None => {
                    // The signature IN EFFECT at the section's start bar,
                    // not the transport's current one: a default-length
                    // clip dropped into a 6/8 section of a 4/4 song must
                    // get a 6/8 bar's worth of ticks, not four quarter
                    // notes. `bar_len_ticks_at` already honours both
                    // halves of the signature (a 6/8 bar is six eighth
                    // notes, not six quarters).
                    def.length_bars as u64 * app.tempo_map.bar_len_ticks_at(placement.start_bar)
                }
            };
            (start, ticks)
        }
        None => {
            let bar = params.start_bar.unwrap_or(1);
            if bar < 1 {
                return reject(request, RpcError::invalid_params("start_bar is 1-based"));
            }
            if let Err(e) = resonance_control::check_max_bars("start_bar", bar) {
                return reject(request, e);
            }
            let start = app.tempo_map.bar_to_sample(bar - 1);
            let ticks = match params.length_beats {
                Some(b) => match beats_to_ticks(b) {
                    Ok(t) => t,
                    Err(e) => return reject(request, e),
                },
                // Same fix as above: the signature at the clip's own
                // target bar, not the transport's current one.
                None => app.tempo_map.bar_len_ticks_at(bar - 1),
            };
            (start, ticks)
        }
    };
    if duration_ticks == 0 {
        return reject(request, RpcError::invalid_params("clip length must be positive"));
    }

    // Allocate the clip id app-side (the high derived-id range) so the
    // reply returns it immediately; `LoadMidiClipDirect` carries the id
    // to the engine, which echoes `MidiClipCreated { clip_id }` to
    // mirror the empty clip into the registry.
    let clip_id = app.compose.fresh_derived_clip_id();
    let name = params.name.clone().unwrap_or_else(|| "MIDI Clip".to_owned());
    let task = super::run_via_update(
        app,
        Message::MidiClip(crate::message::MidiClipMessage::CreateEmptyClip {
            clip_id,
            track_id: params.track_id.0,
            start_sample,
            duration_ticks,
            name,
        }),
    );
    let result = CreateClipResult {
        clip_id: resonance_control::ids::ClipId(clip_id),
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

// ---------------------------------------------------------------------------
// notes.move_clip
// ---------------------------------------------------------------------------

/// `notes.move_clip` (ba doc #269 FR-4): reposition an existing MIDI
/// clip. Nothing else on the wire could change a clip's start, so a
/// client could not correct its own `notes.create_clip` position mistake
/// except by deleting and rebuilding the clip.
///
/// The target resolves through `tempo_map.bar_to_sample` exactly as
/// `notes.create_clip` does, so the move also re-grids a clip whose
/// start drifted off the bar line.
fn move_clip(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: MoveClipParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(clip) = find_clip(app, params.clip_id.0) else {
        return clip_not_found(app, request, params.clip_id.0);
    };
    if let Some(e) = frozen_reject(app, clip.track_id) {
        return reject(request, e);
    }

    // Exactly one target: a bare bar, or a placement to anchor to.
    let bar = match (params.start_bar, params.placement_id) {
        (Some(bar), None) => {
            if bar < 1 {
                return reject(request, RpcError::invalid_params("start_bar is 1-based"));
            }
            if let Err(e) = resonance_control::check_max_bars("start_bar", bar) {
                return reject(request, e);
            }
            bar - 1
        }
        (None, Some(pid)) => {
            let Some(placement) = app.compose.placements.iter().find(|p| p.id == u64::from(pid))
            else {
                return reject(request, no_section_placement(pid.into()));
            };
            placement.start_bar
        }
        (Some(_), Some(_)) => {
            return reject(
                request,
                RpcError::invalid_params(
                    "give exactly one of start_bar or placement_id, not both",
                ),
            )
        }
        (None, None) => {
            return reject(
                request,
                RpcError::invalid_params("notes.move_clip needs start_bar or placement_id"),
            )
        }
    };

    let new_start_sample = app.tempo_map.bar_to_sample(bar);
    let task = super::run_via_update(
        app,
        Message::MidiClip(crate::message::MidiClipMessage::MoveClipTo {
            clip_id: params.clip_id.0,
            new_start_sample,
        }),
    );
    (ack(app, request), task)
}

// ---------------------------------------------------------------------------
// notes.insert_many / notes.replace_all
// ---------------------------------------------------------------------------

/// `notes.insert_many` (ba doc #269 FR-5): add every submitted note in
/// ONE undoable transaction.
///
/// Every control mutation is its own undoable transaction
/// (`run_via_update` -> `record_undo`), so a 468-note melody written
/// note-by-note left 468 undo entries — undo was effectively unusable —
/// and multiplied the read-after-write window by the note count.
fn insert_many(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: InsertManyParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    bulk_write(app, request, params.clip_id.0, &params.notes, false)
}

/// `notes.replace_all` (ba doc #269 FR-5): make the submitted notes the
/// clip's entire note list, in ONE undoable transaction. Clearing and
/// rewriting together also sidesteps the highest-index-first ordering
/// trap of an N-delete loop.
///
/// Destructive on a non-empty clip — every existing note is dropped —
/// so it follows the same confirm convention as `track.delete` /
/// `section.delete`: without `"confirm": true` it refuses with a summary
/// of what would be lost. An empty clip loses nothing and needs no
/// confirmation (the `arrangement.remove_bars` rule: gate on casualties,
/// not on the method name).
fn replace_all(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: ReplaceAllParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(clip) = find_clip(app, params.clip_id.0) else {
        return clip_not_found(app, request, params.clip_id.0);
    };
    // Frozen wins over unconfirmed: a `busy` here spares the client a
    // confirm round-trip on an edit that could not land anyway.
    if let Some(e) = frozen_reject(app, clip.track_id) {
        return reject(request, e);
    }
    if !params.confirm && !clip.notes.is_empty() {
        return reject(
            request,
            RpcError::needs_confirmation(format!(
                "replacing all notes of clip {} ({:?} on track {}) drops its {} existing \
                 note(s); re-send with \"confirm\": true",
                params.clip_id,
                clip.name,
                clip.track_id,
                clip.notes.len(),
            )),
        );
    }
    bulk_write(app, request, params.clip_id.0, &params.notes, true)
}

/// The shared body of the two bulk writes: validate every note up front
/// (so a bad batch mutates nothing), build the clip's final sorted note
/// array, dispatch it as one `SetClipNotes`, and mirror it optimistically
/// so the next `song.notes` sees all of it (the #1166 read-your-own-
/// writes machinery, reused rather than reinvented).
fn bulk_write(
    app: &mut Resonance,
    request: &Request,
    clip_id: u64,
    specs: &[NoteSpec],
    replace: bool,
) -> (Response, Task<Message>) {
    let Some(clip) = find_clip(app, clip_id) else {
        return clip_not_found(app, request, clip_id);
    };
    if let Some(e) = frozen_reject(app, clip.track_id) {
        return reject(request, e);
    }
    // The same bound `notes.import_midi` puts on an SMF track: a batch
    // this size is far past anything musical, and refusing beats spending
    // minutes rebuilding the engine's note tables for a pathological one.
    if specs.len() > notes::MAX_BATCH_NOTES {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "the batch carries {} notes, over the {} limit",
                specs.len(),
                notes::MAX_BATCH_NOTES
            )),
        );
    }

    let mut incoming = Vec::with_capacity(specs.len());
    for (i, spec) in specs.iter().enumerate() {
        match build_note(spec) {
            Ok(note) => incoming.push(note),
            // Point at the offending entry: in a 400-note batch
            // "duration must be positive" alone is not actionable.
            Err(e) => {
                return reject(
                    request,
                    RpcError::invalid_params(format!("notes[{i}]: {}", e.message)),
                )
            }
        }
    }

    // Existing notes survive an insert_many and are dropped by a
    // replace_all. The final array is sorted by start_tick, the order
    // the single-note insert path maintains, so the reported indices
    // address the same notes `song.notes` reports.
    //
    // Each entry is tagged with its submission index (`None` for a kept
    // note) so the merge itself yields the mapping — sorting first and
    // reconstructing afterwards cannot distinguish a kept note from a
    // submitted one at the same tick.
    let mut tagged: Vec<(resonance_audio::types::MidiNote, Option<usize>)> = if replace {
        Vec::with_capacity(incoming.len())
    } else {
        clip.notes.iter().cloned().map(|n| (n, None)).collect()
    };
    tagged.extend(incoming.into_iter().enumerate().map(|(i, n)| (n, Some(i))));
    // Stable, so notes sharing a start_tick keep this order: kept notes
    // first, then the submitted ones in submission order.
    tagged.sort_by_key(|(n, _)| n.start_tick);

    let mut indices = vec![0usize; specs.len()];
    for (position, (_, origin)) in tagged.iter().enumerate() {
        if let Some(i) = origin {
            indices[*i] = position;
        }
    }
    let final_notes: Vec<resonance_audio::types::MidiNote> =
        tagged.into_iter().map(|(n, _)| n).collect();

    let task = super::run_via_update(
        app,
        Message::MidiEditor(MidiEditorMessage::SetClipNotes {
            clip_id,
            notes: final_notes.clone(),
        }),
    );
    crate::engine_events::midi::optimistic_set_notes(app, clip_id, final_notes);

    let result = InsertManyResult {
        indices,
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

/// Validate one wire note and convert it to the engine's shape.
fn build_note(spec: &NoteSpec) -> Result<resonance_audio::types::MidiNote, RpcError> {
    if spec.pitch > 127 {
        return Err(RpcError::invalid_params(format!(
            "pitch {} out of MIDI range 0..=127",
            spec.pitch
        )));
    }
    check_velocity(spec.velocity)?;
    let start_tick = beats_to_ticks(spec.start_beat)?;
    let duration_ticks = beats_to_ticks(spec.duration_beats)?;
    if duration_ticks == 0 {
        return Err(RpcError::invalid_params("duration must be positive"));
    }
    Ok(resonance_audio::types::MidiNote {
        note: spec.pitch,
        velocity: spec.velocity as f32 / 127.0,
        start_tick,
        duration_ticks,
    })
}
