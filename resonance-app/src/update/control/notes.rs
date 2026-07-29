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
    self, CreateClipParams, CreateClipResult, DeleteParams, EditParams, InsertParams,
    InsertResult,
};
use resonance_control::{MutationAck, Request, Response, RpcError};

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
        _ => return None,
    };
    Some(out)
}

fn reject(request: &Request, error: RpcError) -> (Response, Task<Message>) {
    (super::failure(request, error), Task::none())
}

fn ack(app: &Resonance, request: &Request) -> Response {
    super::success(request, &MutationAck { revision: app.revision() })
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
    let detail = if app.clips.iter().any(|c| c.id == clip_id) {
        format!("clip {clip_id} is an audio clip; notes.* edits MIDI clips")
    } else {
        format!("no MIDI clip with id {clip_id}")
    };
    reject(request, RpcError::not_found(detail))
}

/// Beats -> ticks, clamped non-negative.
fn beats_to_ticks(beats: f64) -> Result<u64, RpcError> {
    if !beats.is_finite() || beats < 0.0 {
        return Err(RpcError::invalid_params(format!(
            "beat value must be finite and non-negative (got {beats})"
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

    // The engine inserts keeping the notes sorted by start_tick; the
    // `MidiNoteAdded` handler mirrors that with `partition_point`. Report
    // the same index so the client can address the new note immediately,
    // before the async echo lands.
    let index = clip
        .notes
        .partition_point(|n| n.start_tick <= start_tick);

    let task = super::run_via_update(
        app,
        Message::MidiEditor(MidiEditorMessage::AddNote {
            clip_id: params.clip_id.0,
            note: params.pitch,
            start_tick,
            duration_ticks,
            velocity: params.velocity as f32 / 127.0,
        }),
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
    // messages, each an undoable step of its own. Pitch + start share
    // MoveNote; duration is ResizeNote; velocity is SetNoteVelocity.
    // Skip a sub-edit when its field is unchanged so the undo history and
    // engine traffic stay minimal.
    let clip_id = params.clip_id.0;
    let index = params.index;
    let mut tasks = Vec::new();

    let want_pitch = params.pitch.unwrap_or(existing.note);
    let want_start = start_tick.unwrap_or(existing.start_tick);
    if params.pitch.is_some() || start_tick.is_some() {
        if want_pitch != existing.note || want_start != existing.start_tick {
            tasks.push(super::run_via_update(
                app,
                Message::MidiEditor(MidiEditorMessage::MoveNote {
                    clip_id,
                    note_index: index,
                    new_start_tick: want_start,
                    new_note: want_pitch,
                }),
            ));
        }
    }
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
        }
    }

    (ack(app, request), Task::batch(tasks))
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
        return reject(
            request,
            RpcError::not_found(format!("no track with id {}", params.track_id)),
        );
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
                return reject(
                    request,
                    RpcError::not_found(format!("no section placement with id {pid}")),
                );
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
                    def.length_bars as u64
                        * app.transport.time_sig_num as u64
                        * resonance_audio::types::TICKS_PER_QUARTER_NOTE
                }
            };
            (start, ticks)
        }
        None => {
            let bar = params.start_bar.unwrap_or(1);
            if bar < 1 {
                return reject(request, RpcError::invalid_params("start_bar is 1-based"));
            }
            let start = app.tempo_map.bar_to_sample(bar - 1);
            let ticks = match params.length_beats {
                Some(b) => match beats_to_ticks(b) {
                    Ok(t) => t,
                    Err(e) => return reject(request, e),
                },
                None => {
                    app.transport.time_sig_num as u64
                        * resonance_audio::types::TICKS_PER_QUARTER_NOTE
                }
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
