//! `notes.import_midi` — Standard MIDI File import over the control API
//! (ba doc #270 §10, doc #273, todo #1195).
//!
//! Every hand-authored part otherwise has to be serialised as a JSON
//! note array through a tool call — ~2900 notes for one reported song —
//! which dominates the cost of authored material over the control API.
//!
//! THIS IS WIRING, NOT AN IMPORTER. The parser is
//! [`resonance_audio::midi_io::parse_smf_bytes`] (midly 0.5), the same
//! one the GUI import dialog uses; nothing here re-reads the format.
//! The notes land through the existing `SetClipNotes` bulk path, so an
//! import is ONE undoable edit like any other.

use crate::message::{Message, MidiEditorMessage};
use crate::Resonance;
use base64::Engine as _;
use iced::Task;
use resonance_audio::midi_io::{parse_smf_bytes, ImportedSmf, ImportedTrack};
use resonance_audio::types::{MidiNote, TrackType, TICKS_PER_QUARTER_NOTE};
use resonance_control::methods::notes::{
    ImportMidiParams, ImportMidiResult, MAX_MIDI_BYTES,
};
use resonance_control::{Request, Response, RpcError};

use super::reply::{no_midi_clip, no_track, reject};

/// Upper bound on notes accepted from one file. A part this size is
/// already far past anything musical; refusing beats spending minutes
/// rebuilding the engine's note tables for a pathological file.
const MAX_NOTES: usize = 100_000;

pub(super) fn handle(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: ImportMidiParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };

    let bytes = match read_source(&params) {
        Ok(bytes) => bytes,
        Err(e) => return reject(request, e),
    };
    let smf = match parse_smf_bytes(&bytes) {
        Ok(smf) => smf,
        // The parser's own reason, verbatim: "Format 2 is not
        // supported", "parse smf: ..." — a client can act on that.
        Err(reason) => {
            return reject(
                request,
                RpcError::invalid_params(format!("not a usable MIDI file: {reason}")),
            )
        }
    };

    let (source_track, track) = match pick_track(&smf, params.source_track) {
        Ok(picked) => picked,
        Err(e) => return reject(request, e),
    };
    if track.notes.len() > MAX_NOTES {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "SMF track {source_track} carries {} notes, over the {MAX_NOTES} limit",
                track.notes.len()
            )),
        );
    }
    let notes: Vec<MidiNote> = track.notes.clone();
    let length_ticks = notes
        .iter()
        .map(|n| n.start_tick + n.duration_ticks)
        .max()
        .unwrap_or(0);

    // Resolve the target: an existing clip whose notes are replaced, or
    // a new clip on a track.
    let (clip_id, track_id, task) = match resolve_target(app, &params, length_ticks) {
        Ok(target) => target,
        Err(e) => return reject(request, e),
    };

    // The notes are already in engine ticks and clip-relative, so the
    // project's tempo map governs where they sound: the CLIP's start is
    // resolved through `tempo_map.bar_to_sample` and the notes ride the
    // map from there, exactly like authored ones. A fixed BPM is never
    // assumed.
    let write = super::run_via_update(
        app,
        Message::MidiEditor(MidiEditorMessage::SetClipNotes {
            clip_id,
            notes: notes.clone(),
        }),
    );
    crate::engine_events::midi::optimistic_set_notes(app, clip_id, notes);

    let result = ImportMidiResult {
        clip_id: resonance_control::ids::ClipId(clip_id),
        track_id: resonance_control::ids::TrackId(track_id),
        note_count: track.notes.len(),
        source_track,
        source_track_name: track.name.clone(),
        length_beats: length_ticks as f64 / TICKS_PER_QUARTER_NOTE as f64,
        revision: app.revision(),
    };
    (super::success(request, &result), Task::batch([task, write]))
}

/// The file's bytes, from exactly one of `path` / `data_base64`.
fn read_source(params: &ImportMidiParams) -> Result<Vec<u8>, RpcError> {
    match (&params.path, &params.data_base64) {
        (Some(_), Some(_)) => Err(RpcError::invalid_params(
            "give exactly one of path or data_base64, not both",
        )),
        (None, None) => Err(RpcError::invalid_params(
            "give the MIDI file as an absolute path or as data_base64",
        )),
        (Some(path), None) => {
            let path = std::path::Path::new(path);
            if !path.is_absolute() {
                return Err(RpcError::invalid_params(format!(
                    "path must be absolute (got {})",
                    path.display()
                )));
            }
            let size = std::fs::metadata(path)
                .map_err(|e| RpcError::not_found(format!("cannot read {}: {e}", path.display())))?
                .len();
            if size as usize > MAX_MIDI_BYTES {
                return Err(too_large(size as usize));
            }
            std::fs::read(path)
                .map_err(|e| RpcError::not_found(format!("cannot read {}: {e}", path.display())))
        }
        (None, Some(encoded)) => {
            // Bound BEFORE decoding: base64 is 4/3 the size of its
            // payload, so this refuses an oversized blob without
            // allocating it.
            if encoded.len() / 4 * 3 > MAX_MIDI_BYTES {
                return Err(too_large(encoded.len() / 4 * 3));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .map_err(|e| {
                    RpcError::invalid_params(format!("data_base64 is not valid base64: {e}"))
                })?;
            if bytes.len() > MAX_MIDI_BYTES {
                return Err(too_large(bytes.len()));
            }
            Ok(bytes)
        }
    }
}

fn too_large(size: usize) -> RpcError {
    RpcError::invalid_params(format!(
        "MIDI file is {size} bytes, over the {MAX_MIDI_BYTES}-byte limit; import it in \
         smaller parts rather than expecting a truncated one"
    ))
}

/// Which SMF track to import. A file with one note-carrying track needs
/// no selector; a multi-track file without one is REFUSED, listing what
/// it contains — silently flattening several parts into a single clip is
/// the failure mode this guards against.
fn pick_track(
    smf: &ImportedSmf,
    requested: Option<usize>,
) -> Result<(usize, &ImportedTrack), RpcError> {
    if let Some(index) = requested {
        let track = smf.tracks.get(index).ok_or_else(|| {
            RpcError::invalid_params(format!(
                "source_track {index} is out of range; the file has {} track(s): {}",
                smf.tracks.len(),
                describe_tracks(smf)
            ))
        })?;
        if track.notes.is_empty() {
            return Err(RpcError::invalid_params(format!(
                "SMF track {index} carries no notes; the file's tracks are: {}",
                describe_tracks(smf)
            )));
        }
        return Ok((index, track));
    }

    let with_notes: Vec<usize> = smf
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.notes.is_empty())
        .map(|(i, _)| i)
        .collect();
    match with_notes.as_slice() {
        [] => Err(RpcError::invalid_params(
            "the MIDI file contains no notes".to_owned(),
        )),
        [only] => Ok((*only, &smf.tracks[*only])),
        _ => Err(RpcError::invalid_params(format!(
            "the file has {} note-carrying tracks and would be flattened into one clip; \
             name one with source_track. Tracks: {}",
            with_notes.len(),
            describe_tracks(smf)
        ))),
    }
}

/// `0: "Bass" (312 notes), 1: unnamed (0 notes)` — enough for the caller
/// to pick without opening the file itself.
fn describe_tracks(smf: &ImportedSmf) -> String {
    smf.tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let name = t
                .name
                .as_deref()
                .map(|n| format!("{n:?}"))
                .unwrap_or_else(|| "unnamed".to_owned());
            format!("{i}: {name} ({} notes)", t.notes.len())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve `clip_id` / `track_id` into the clip the notes go into,
/// creating one when the caller named a track.
fn resolve_target(
    app: &mut Resonance,
    params: &ImportMidiParams,
    length_ticks: u64,
) -> Result<(u64, u64, Task<Message>), RpcError> {
    match (params.clip_id, params.track_id) {
        (Some(_), Some(_)) => Err(RpcError::invalid_params(
            "target either an existing clip_id or a track_id, not both",
        )),
        (None, None) => Err(RpcError::invalid_params(
            "name the import target: clip_id (replaces that clip's notes) or track_id \
             (creates a clip)",
        )),
        (Some(clip_id), None) => {
            let clip = app
                .midi_clips
                .iter()
                .find(|c| c.id == clip_id.0)
                .ok_or_else(|| {
                    no_midi_clip(clip_id.into())
                })?;
            let track_id = clip.track_id;
            if let Some(error) = frozen_reject(app, track_id) {
                return Err(error);
            }
            Ok((clip_id.0, track_id, Task::none()))
        }
        (None, Some(track_id)) => {
            let track = app
                .registry
                .tracks
                .iter()
                .find(|t| t.id == track_id.0)
                .ok_or_else(|| no_track(track_id.into()))?;
            if !matches!(track.track_type, TrackType::Instrument | TrackType::Vocal) {
                return Err(RpcError::invalid_params(format!(
                    "track {track_id} is an audio track; MIDI needs an instrument/vocal track"
                )));
            }
            if let Some(error) = frozen_reject(app, track_id.0) {
                return Err(error);
            }
            let bar = params.start_bar.unwrap_or(1);
            if bar < 1 {
                return Err(RpcError::invalid_params("start_bar is 1-based"));
            }
            let start_sample = app.tempo_map.bar_to_sample(bar - 1);
            // Long enough to hold the imported material, rounded up to a
            // whole bar so the clip lines up with the grid.
            let bar_ticks =
                app.transport.time_sig_num as u64 * TICKS_PER_QUARTER_NOTE;
            let duration_ticks = length_ticks.div_ceil(bar_ticks).max(1) * bar_ticks;

            let clip_id = app.compose.fresh_derived_clip_id();
            let name = params
                .name
                .clone()
                .unwrap_or_else(|| "Imported MIDI".to_owned());
            let task = super::run_via_update(
                app,
                Message::MidiClip(crate::message::MidiClipMessage::CreateEmptyClip {
                    clip_id,
                    track_id: track_id.0,
                    start_sample,
                    duration_ticks,
                    name,
                }),
            );
            Ok((clip_id, track_id.0, task))
        }
    }
}

/// The #576 frozen-input rule: a frozen track's note edits are swallowed
/// by the `update()` gate, so say so here rather than letting the import
/// vanish while the reply claims success.
fn frozen_reject(app: &Resonance, track_id: u64) -> Option<RpcError> {
    app.freeze.status(track_id).is_frozen().then(|| {
        RpcError::busy(format!(
            "track {track_id} is frozen; unfreeze it before importing notes"
        ))
    })
}
