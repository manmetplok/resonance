//! `pool.*` and `clip.*` control handlers (ba doc #265): getting a sample
//! into the project and placing it on the timeline.
//!
//! The two namespaces are one flow and live in one file. A sample must be
//! a *pool asset* before it can sound — the engine decodes it, mixes it to
//! stereo, resamples it to the project rate and copies it into the
//! project's `audio/` folder — and a *clip* is one placement of such an
//! asset on a track. `pool.list` and `pool.import` cover the first half,
//! `clip.*` the second.
//!
//! # Why placing is a job
//!
//! The import runs on an engine worker thread, so neither the asset id nor
//! the audio's length exists when the request returns. Rather than reply
//! with a half-filled result, `pool.import` and `clip.place` are jobs
//! ([`JobToken::PoolImport`]): the reply carries a `job_id`, and the
//! completion hook in `engine_events::pool` resolves it with the real
//! numbers once the file has landed. A `clip.place` naming an asset that
//! is ALREADY pooled has nothing to wait for, so its job is completed
//! inside this handler, before the reply is even sent — the client sees a
//! `done` job on the first look either way.
//!
//! Placement itself reuses the app's own drag-and-drop orchestration
//! (`PoolMessage::ImportAndPlace` -> `state::pool_import` -> the
//! `AssetImported` hook), so a placed clip is indistinguishable from one
//! the user dragged in: same asset ref, same usage counts, same single
//! undo entry covering import and placement together.
//!
//! # What is not here
//!
//! `SetClipWarp` exists as an engine command but has no app-side mirror,
//! message or undo entry — the GUI cannot set it either. Exposing
//! "follow tempo" over the wire means building that half first, so it is
//! deliberately absent rather than half-wired.

use crate::control_jobs::JobToken;
use crate::control_socket::ConnId;
use crate::message::{ClipMessage, Message, PoolMessage};
use crate::state::ClipState;
use crate::Resonance;
use iced::Task;
use resonance_audio::types::{ClipId, FadeCurve, TrackType};
use resonance_control::methods::clip::{
    self as proto, AmountSpec, DeleteParams, FadeResult, FadeShape, MoveParams, PlaceParams,
    SetFadeParams, SetGainParams, TrimParams, TrimResult,
};
use resonance_control::methods::pool::{self as pool_proto, ImportParams, PoolAssetView, PoolView};
use resonance_control::{PositionSpec, Request, Response, RpcError};

use super::reply::{ack, no_track, reject};
use std::path::{Path, PathBuf};

/// Handle a `pool.*` / `clip.*` request, or `None` when `method` belongs
/// to another namespace. Mutating dispatch: the returned [`Task`] must
/// reach the runtime.
pub(super) fn try_handle(
    app: &mut Resonance,
    conn: ConnId,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let handled = match request.method.as_str() {
        pool_proto::LIST => (list(app, request), Task::none()),
        pool_proto::IMPORT => import(app, conn, request),
        proto::PLACE => place(app, conn, request),
        proto::MOVE => move_clip(app, request),
        proto::TRIM => trim(app, request),
        proto::SPLIT => split(app, request),
        proto::DELETE => delete(app, request),
        proto::SET_GAIN => set_gain(app, request),
        proto::SET_FADE => set_fade(app, request),
        _ => return None,
    };
    Some(handled)
}

// ---------------------------------------------------------------------------
// pool.list
// ---------------------------------------------------------------------------

fn list(app: &Resonance, request: &Request) -> Response {
    let assets = app
        .media
        .pool
        .assets
        .iter()
        .map(|a| asset_view(app, a))
        .collect::<Vec<_>>();
    super::success(
        request,
        &PoolView {
            assets,
            revision: app.revision(),
        },
    )
}

/// Project one pool asset onto the wire, resolving its usage count and
/// turning its format enum into the lowercase string the protocol uses.
pub(crate) fn asset_view(app: &Resonance, asset: &crate::state::PoolAsset) -> PoolAssetView {
    PoolAssetView {
        id: resonance_control::ids::AssetId(asset.id),
        original_path: asset.original_path.clone(),
        name: name_from_path(&asset.original_path),
        duration_frames: asset.duration_frames,
        duration_seconds: asset.duration_frames as f64 / app.sample_rate as f64,
        channels: asset.channels,
        source_sample_rate: asset.source_sample_rate,
        format: format!("{:?}", asset.format).to_lowercase(),
        usage_count: app.media.pool.usage_count(asset.id),
        missing: asset.missing,
    }
}

/// The file stem of a source path — the name a clip placed from it gets.
/// Matches `engine_events::pool::clip_name_from` so the reported name is
/// the one the clip actually ends up with.
fn name_from_path(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Audio clip".to_string())
}

// ---------------------------------------------------------------------------
// pool.import
// ---------------------------------------------------------------------------

fn import(app: &mut Resonance, conn: ConnId, request: &Request) -> (Response, Task<Message>) {
    let params: ImportParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if let Some(e) = project_dir_guard(app) {
        return reject(request, e);
    }
    let paths = match check_paths(&params.paths) {
        Ok(paths) => paths,
        Err(e) => return reject(request, e),
    };
    let file_count = paths.len();

    // Dispatch first, THEN build the job token from the ids `import()` just
    // allocated (D-7a: the app is the pool's only asset-id allocator, so
    // they don't exist until it runs). `run_via_update` runs the reducer
    // synchronously — the only async part is the returned `Task`, which
    // resolves later via the ordinary engine-event path — so the newly
    // queued placements are still in `pool_import` right here, and nothing
    // else can interleave a second import between the two calls.
    let task = super::run_via_update(app, Message::Pool(PoolMessage::ImportFilesToPool(paths)));
    let asset_ids = app.media.pool_import.last_n_asset_ids(file_count);
    let started = app.start_control_job(
        pool_proto::IMPORT,
        &format!("Import {file_count} file(s) into the pool"),
        JobToken::PoolImport { asset_ids },
        Some(conn),
    );
    (super::success(request, &started), task)
}

/// Validate a batch of source paths: non-empty, within the batch cap,
/// absolute, and present on disk. Checked up front so a bad batch imports
/// nothing rather than half of itself.
fn check_paths(paths: &[String]) -> Result<Vec<PathBuf>, RpcError> {
    if paths.is_empty() {
        return Err(RpcError::invalid_params("no paths given"));
    }
    if paths.len() > pool_proto::MAX_IMPORT_FILES {
        return Err(RpcError::invalid_params(format!(
            "{} files exceeds the {} per import",
            paths.len(),
            pool_proto::MAX_IMPORT_FILES
        )));
    }
    paths.iter().map(|p| check_path(p)).collect()
}

fn check_path(path: &str) -> Result<PathBuf, RpcError> {
    let buf = PathBuf::from(path);
    if !buf.is_absolute() {
        return Err(RpcError::invalid_params(format!(
            "path must be absolute, got {path:?}"
        )));
    }
    if !buf.is_file() {
        return Err(RpcError::not_found(format!("no file at {path:?}")));
    }
    Ok(buf)
}

/// Importing copies the decoded audio into the project's own `audio/`
/// folder, so an unsaved project has nowhere to put it. The app's import
/// handler refuses in that case; say so plainly instead of starting a job
/// that can never complete.
fn project_dir_guard(app: &Resonance) -> Option<RpcError> {
    app.io.project_path.is_none().then(|| {
        RpcError::busy(
            "the project has never been saved, so imported audio has nowhere to \
             live; save it first (project.save_as)",
        )
    })
}

// ---------------------------------------------------------------------------
// clip.place
// ---------------------------------------------------------------------------

fn place(app: &mut Resonance, conn: ConnId, request: &Request) -> (Response, Task<Message>) {
    let params: PlaceParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };

    // The target track must exist and take audio. An instrument/drums/
    // vocal track would accept the clip into a lane that never renders it.
    let track_id = params.track_id.0;
    let Some(track) = app.registry.tracks.iter().find(|t| t.id == track_id) else {
        return reject(request, no_track(track_id));
    };
    if !takes_audio_clips(app, track) {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "track {track_id} is a {:?} track; audio clips need an audio track \
                 (track.add with kind \"audio\") or an external-instrument track",
                track.track_type
            )),
        );
    }

    // Exactly one way of naming the sample.
    let source = match (params.asset_id, params.path.as_deref()) {
        (Some(asset_id), None) => {
            let Some(asset) = app.media.pool.assets.iter().find(|a| a.id == asset_id.0) else {
                return reject(
                    request,
                    RpcError::not_found(format!(
                        "no pool asset with id {asset_id}; list them with pool.list"
                    )),
                );
            };
            Source::Pooled(asset.id, asset.original_path.clone())
        }
        (None, Some(path)) => {
            // A path already in the pool places from the existing asset
            // instead of importing the same file twice.
            match app.media.pool.assets.iter().find(|a| a.original_path == path) {
                Some(asset) => Source::Pooled(asset.id, asset.original_path.clone()),
                None => match check_path(path) {
                    Ok(buf) => Source::File(buf),
                    Err(e) => return reject(request, e),
                },
            }
        }
        (Some(_), Some(_)) => {
            return reject(
                request,
                RpcError::invalid_params("give exactly one of asset_id or path, not both"),
            )
        }
        (None, None) => {
            return reject(
                request,
                RpcError::invalid_params("clip.place needs an asset_id or a path"),
            )
        }
    };

    let start_spec = if params.start.is_empty() {
        PositionSpec::musical(1, 1.0)
    } else {
        params.start
    };
    let start_sample = match super::transport::resolve_position(app, &start_spec) {
        Ok(sample) => sample,
        Err(e) => return reject(request, e),
    };

    match source {
        // Already imported: place it right now, so the job is `done`
        // before the reply leaves.
        Source::Pooled(asset_id, original_path) => {
            let started = app.start_control_job(
                proto::PLACE,
                &format!("Place {original_path} on track {track_id}"),
                JobToken::PoolImport {
                    asset_ids: vec![asset_id],
                },
                Some(conn),
            );
            // Allocate the clip id here, the way `notes.create_clip` does,
            // so the result is complete before the engine has echoed
            // anything. Routed through `update()` so the placement records
            // its own undo entry, exactly like the import path does.
            let clip_id = app.media.ids.clips.allocate();
            let task = super::run_via_update(
                app,
                Message::Pool(PoolMessage::PlacePooledAsset {
                    clip_id,
                    asset_id,
                    track_id,
                    start_sample,
                }),
            );
            // The dispatch runs synchronously, but not unconditionally:
            // the pool handler refuses when the asset has vanished, and
            // the pre-dispatch gates swallow `Pool` messages outright
            // while a bounce or freeze render is in flight. Completing
            // the job from `place_result`'s missing-clip fallback would
            // then report `done` with fabricated geometry (track 0,
            // sample 0, empty name) — fail it instead, so a no-op can
            // never be mistaken for success.
            if find_clip(app, clip_id).is_some() {
                let result = place_result(app, clip_id, asset_id);
                app.control.jobs.complete(
                    u64::from(started.job_id),
                    serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
                );
            } else {
                let error = if app.media.pool.assets.iter().any(|a| a.id == asset_id) {
                    format!(
                        "placed clip {clip_id} not found: the placement was dropped \
                         before it reached the project; nothing was placed"
                    )
                } else {
                    format!("asset {asset_id} is no longer in the pool; nothing was placed")
                };
                app.control.jobs.fail(u64::from(started.job_id), error);
            }
            (super::success(request, &started), task)
        }
        // Not imported yet: hand it to the same import+place orchestration
        // a drag-and-drop uses and let the completion hook resolve the job.
        Source::File(path) => {
            if let Some(e) = project_dir_guard(app) {
                return reject(request, e);
            }
            let display_path = path.display().to_string();
            // Dispatch first, then build the job token from the id
            // `import()` just allocated (D-7a) — see the same reordering
            // note on `pool.import` above.
            let task = super::run_via_update(
                app,
                Message::Pool(PoolMessage::ImportAndPlaceExact {
                    paths: vec![path],
                    track_id,
                    start_sample,
                }),
            );
            let asset_ids = app.media.pool_import.last_n_asset_ids(1);
            let started = app.start_control_job(
                proto::PLACE,
                &format!("Import and place {display_path} on track {track_id}"),
                JobToken::PoolImport { asset_ids },
                Some(conn),
            );
            (super::success(request, &started), task)
        }
    }
}

/// Where `clip.place` is getting its audio from.
enum Source {
    /// Already in the pool — place immediately. Carries the asset's source
    /// path for the job description.
    Pooled(u64, String),
    /// A file on disk that has to be imported first.
    File(PathBuf),
}

/// Build the `clip.place` job result for a clip that has landed. Public to
/// the control module so the async completion hook
/// (`engine_events::pool`) and the synchronous already-pooled path above
/// share one definition of the result shape.
///
/// The missing-clip fallback (zeroed geometry) exists only so the async
/// hook cannot panic on a clip deleted in the same tick; a caller
/// completing a job MUST check the clip is in the mirror first and fail
/// the job otherwise — `done` with fabricated geometry is exactly the
/// bug the synchronous path above guards against.
pub(crate) fn place_result(
    app: &Resonance,
    clip_id: ClipId,
    asset_id: u64,
) -> proto::PlaceResult {
    let (track_id, start_sample, length_samples, name) = match find_clip(app, clip_id) {
        Some(c) => (
            c.track_id,
            c.start_sample,
            c.duration_samples,
            c.name.clone(),
        ),
        None => (0, 0, 0, String::new()),
    };
    proto::PlaceResult {
        clip_id: resonance_control::ids::ClipId(clip_id),
        track_id: resonance_control::ids::TrackId(track_id),
        asset_id: resonance_control::ids::AssetId(asset_id),
        start: super::view_model::song_position(app, start_sample),
        length_beats: samples_to_beats(app, start_sample, length_samples),
        length_samples,
        name,
        revision: app.revision(),
    }
}

/// Build the `pool.import` job result for a finished batch: the pool
/// assets whose id is one of `asset_ids` (D-7a — the app allocated them
/// itself, so they identify the batch's files unambiguously, unlike their
/// source paths when two files in flight share one). Public to the control
/// module so the completion hook in `engine_events::pool` shares this
/// definition.
pub(crate) fn import_result(app: &Resonance, asset_ids: &[u64]) -> pool_proto::ImportResult {
    let assets = app
        .media
        .pool
        .assets
        .iter()
        .filter(|a| asset_ids.contains(&a.id))
        .map(|a| asset_view(app, a))
        .collect();
    pool_proto::ImportResult {
        assets,
        revision: app.revision(),
    }
}

// ---------------------------------------------------------------------------
// clip.move / clip.trim / clip.delete
// ---------------------------------------------------------------------------

/// Whether `track` can hold audio clips.
///
/// An external-instrument track does, even though its type is
/// `Instrument`: its sound is outboard, so what lands on it is recorded
/// AUDIO — the mixer renders it through the audio-track branch, and
/// `clip.trim` has always worked on those takes. Only `clip.place` and
/// `clip.move` refused, which meant a hardware take could be cut but
/// never copied or re-placed: duplicating an 8-bar section was
/// impossible over the API and had to be spliced outside the project
/// (ba doc #275 P2).
fn takes_audio_clips(app: &Resonance, track: &crate::state::TrackState) -> bool {
    track.track_type == TrackType::Audio || app.devices.external_instruments.contains_key(&track.id)
}

fn move_clip(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: MoveParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(clip) = find_clip(app, params.clip_id.0) else {
        return clip_not_found(app, request, params.clip_id.0);
    };
    let current_track = clip.track_id;

    let new_track_id = match params.track_id {
        Some(id) => match app.registry.tracks.iter().find(|t| t.id == id.0) {
            Some(t) if takes_audio_clips(app, t) => id.0,
            Some(t) => {
                return reject(
                    request,
                    RpcError::invalid_params(format!(
                        "track {id} is a {:?} track; audio clips need an audio track \
                         or an external-instrument track",
                        t.track_type
                    )),
                )
            }
            None => return reject(request, no_track(id.into())),
        },
        None => current_track,
    };
    let new_start_sample = match super::transport::resolve_position(app, &params.start) {
        Ok(sample) => sample,
        Err(e) => return reject(request, e),
    };

    let task = super::run_via_update(
        app,
        Message::Clip(ClipMessage::MoveClipTo {
            clip_id: params.clip_id.0,
            new_start_sample,
            new_track_id,
        }),
    );
    (ack(app, request), task)
}

/// `clip.split` — cut one clip in two at a timeline position.
///
/// Both halves are trims of the same source, so the edit is free for a
/// mapped take and exactly reversible with `edit.undo`. The tail's id is
/// allocated here (like `clip.place`) so the reply names both halves and
/// `song.tracks` resolves the new one on the very next request.
fn split(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::SplitParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(clip) = find_clip(app, params.clip_id.0) else {
        return clip_not_found(app, request, params.clip_id.0);
    };
    let (clip_start, duration) = (clip.start_sample, clip.duration_samples);

    let at_sample = match super::transport::resolve_position(app, &params.at) {
        Ok(sample) => sample,
        Err(e) => return reject(request, e),
    };
    // A cut at either edge leaves one half empty. Refusing beats
    // returning a zero-length clip the caller would then have to notice.
    if at_sample <= clip_start || at_sample >= clip_start + duration {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "clip {} plays samples {}..{}; a split at {at_sample} is outside it, so one \
                 half would be empty",
                params.clip_id,
                clip_start,
                clip_start + duration
            )),
        );
    }

    let new_clip_id = app.media.ids.clips.allocate();
    let task = super::run_via_update(
        app,
        Message::Clip(ClipMessage::SplitClipAt {
            clip_id: params.clip_id.0,
            new_clip_id,
            at_sample,
        }),
    );
    let result = proto::SplitResult {
        head_clip_id: resonance_control::ids::ClipId(params.clip_id.0),
        tail_clip_id: resonance_control::ids::ClipId(new_clip_id),
        at: super::view_model::song_position(app, at_sample),
        head_length_samples: at_sample - clip_start,
        tail_length_samples: clip_start + duration - at_sample,
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

fn trim(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: TrimParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(clip) = find_clip(app, params.clip_id.0) else {
        return clip_not_found(app, request, params.clip_id.0);
    };
    if params.start_offset.is_none() && params.end_offset.is_none() && params.start.is_none() {
        return reject(request, RpcError::invalid_params("clip.trim changed nothing"));
    }
    let (clip_start, total_frames, trim_start, trim_end) = (
        clip.start_sample,
        clip.total_frames,
        clip.trim_start_frames,
        clip.trim_end_frames,
    );

    // Amounts convert against the tempo at the clip's own position, so a
    // beat-denominated trim means the same thing as the grid drawn there.
    let mut new_trim_start = match params.start_offset {
        Some(spec) => match resolve_amount(app, &spec, clip_start) {
            Ok(frames) => frames,
            Err(e) => return reject(request, e),
        },
        None => trim_start,
    };
    let mut new_trim_end = match params.end_offset {
        Some(spec) => match resolve_amount(app, &spec, clip_start) {
            Ok(frames) => frames,
            Err(e) => return reject(request, e),
        },
        None => trim_end,
    };
    // Clamp to the source: the two offsets together must leave audio. The
    // head wins ties, matching the drag path's "trim against the opposite
    // edge's current value" rule.
    if new_trim_start >= total_frames {
        new_trim_start = total_frames.saturating_sub(1);
    }
    let max_end = total_frames.saturating_sub(new_trim_start).saturating_sub(1);
    if new_trim_end > max_end {
        new_trim_end = max_end;
    }

    let new_start_sample = match params.start {
        Some(spec) => match super::transport::resolve_position(app, &spec) {
            Ok(sample) => sample,
            Err(e) => return reject(request, e),
        },
        None => clip_start,
    };

    let task = super::run_via_update(
        app,
        Message::Clip(ClipMessage::TrimClipTo {
            clip_id: params.clip_id.0,
            new_start_sample,
            trim_start_frames: new_trim_start,
            trim_end_frames: new_trim_end,
        }),
    );

    let length_samples = total_frames
        .saturating_sub(new_trim_start)
        .saturating_sub(new_trim_end);
    let result = TrimResult {
        clip_id: params.clip_id,
        start: super::view_model::song_position(app, new_start_sample),
        start_offset_samples: new_trim_start,
        end_offset_samples: new_trim_end,
        length_samples,
        length_beats: samples_to_beats(app, new_start_sample, length_samples),
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

fn delete(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: DeleteParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(clip) = find_clip(app, params.clip_id.0) else {
        return clip_not_found(app, request, params.clip_id.0);
    };
    // Destructive, so the same confirm convention as `track.delete` /
    // `section.delete`: refuse without `"confirm": true`, summarizing
    // what would be lost.
    if !params.confirm {
        let (name, track_id, start_sample, length) = (
            clip.name.clone(),
            clip.track_id,
            clip.start_sample,
            clip.duration_samples,
        );
        let start = super::view_model::song_position(app, start_sample);
        return reject(
            request,
            RpcError::needs_confirmation(format!(
                "deleting clip {} ({name:?}) removes {length} sample(s) of audio from \
                 track {track_id} at bar {}; re-send with \"confirm\": true",
                params.clip_id, start.bar,
            )),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Clip(ClipMessage::DeleteClip(params.clip_id.0)),
    );
    // Read-your-own-writes: the app mirror normally drops the clip on the
    // engine's `ClipDeleted` echo, which lands after this reply. Drop it
    // now so the next `song.tracks` on this connection cannot still report
    // a clip the client was just told was deleted. `clips::deleted`
    // retains by id, so the echo is a no-op rather than a double delete.
    crate::engine_events::clips::deleted(app, params.clip_id.0);
    (ack(app, request), task)
}

// ---------------------------------------------------------------------------
// clip.set_gain / clip.set_fade
// ---------------------------------------------------------------------------

fn set_gain(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetGainParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_clip(app, params.clip_id.0).is_none() {
        return clip_not_found(app, request, params.clip_id.0);
    }
    if !params.gain_db.is_finite() {
        return reject(
            request,
            RpcError::invalid_params("gain_db must be a finite number"),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Clip(ClipMessage::SetClipGainDb {
            clip_id: params.clip_id.0,
            gain_db: params.gain_db.min(proto::MAX_GAIN_DB),
        }),
    );
    (ack(app, request), task)
}

fn set_fade(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetFadeParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(clip) = find_clip(app, params.clip_id.0) else {
        return clip_not_found(app, request, params.clip_id.0);
    };
    if params.fade_in.is_none()
        && params.fade_out.is_none()
        && params.fade_in_shape.is_none()
        && params.fade_out_shape.is_none()
    {
        return reject(
            request,
            RpcError::invalid_params("clip.set_fade changed nothing"),
        );
    }
    let clip_start = clip.start_sample;
    let audible = clip.duration_samples;
    let clip_id = params.clip_id.0;

    // Resolve both lengths before dispatching anything, so an invalid
    // second amount doesn't leave the first one applied.
    let fade_in = match params.fade_in {
        Some(spec) => match resolve_amount(app, &spec, clip_start) {
            Ok(frames) => Some(frames.min(audible)),
            Err(e) => return reject(request, e),
        },
        None => None,
    };
    let fade_out = match params.fade_out {
        Some(spec) => match resolve_amount(app, &spec, clip_start) {
            Ok(frames) => Some(frames.min(audible)),
            Err(e) => return reject(request, e),
        },
        None => None,
    };
    for shape in [params.fade_in_shape, params.fade_out_shape].into_iter().flatten() {
        if shape == FadeShape::Unknown {
            return reject(
                request,
                RpcError::invalid_params(
                    "unknown fade shape; use linear, equal_power or exp",
                ),
            );
        }
    }

    // The app's fade setters take milliseconds (the inspector's unit);
    // convert once, against the project rate. Up to four messages land
    // here, so they are grouped into ONE undoable transaction: one
    // revision bump per call on the wire, one edit_undo to take the
    // whole fade change back.
    let sample_rate = app.sample_rate as f32;
    let ms = |frames: u64| frames as f32 * 1000.0 / sample_rate;
    app.with_compound_undo(|app| {
        let mut tasks = Vec::new();
        if let Some(frames) = fade_in {
            tasks.push(super::run_via_update(
                app,
                Message::Clip(ClipMessage::SetClipFadeInMs {
                    clip_id,
                    ms: ms(frames),
                }),
            ));
        }
        if let Some(frames) = fade_out {
            tasks.push(super::run_via_update(
                app,
                Message::Clip(ClipMessage::SetClipFadeOutMs {
                    clip_id,
                    ms: ms(frames),
                }),
            ));
        }
        if let Some(shape) = params.fade_in_shape {
            tasks.push(super::run_via_update(
                app,
                Message::Clip(ClipMessage::SetClipFadeInCurve {
                    clip_id,
                    curve: fade_curve(shape),
                }),
            ));
        }
        if let Some(shape) = params.fade_out_shape {
            tasks.push(super::run_via_update(
                app,
                Message::Clip(ClipMessage::SetClipFadeOutCurve {
                    clip_id,
                    curve: fade_curve(shape),
                }),
            ));
        }

        // Report what the mirror ended up with — the app clamps each fade
        // to the clip's audible length, so the echo is the authority, not
        // the request.
        let (fade_in_samples, fade_out_samples, in_curve, out_curve) =
            match find_clip(app, clip_id) {
                Some(c) => (
                    c.fade_in_frames,
                    c.fade_out_frames,
                    c.fade_in_curve,
                    c.fade_out_curve,
                ),
                None => (0, 0, FadeCurve::default(), FadeCurve::default()),
            };
        let result = FadeResult {
            clip_id: params.clip_id,
            fade_in_samples,
            fade_out_samples,
            fade_in_shape: fade_shape(in_curve),
            fade_out_shape: fade_shape(out_curve),
            revision: app.revision(),
        };
        (super::success(request, &result), Task::batch(tasks))
    })
}

fn fade_curve(shape: FadeShape) -> FadeCurve {
    match shape {
        FadeShape::Linear => FadeCurve::Linear,
        FadeShape::Exp => FadeCurve::Exp,
        // `Unknown` is rejected before this point; default with the rest.
        FadeShape::EqualPower | FadeShape::Unknown => FadeCurve::EqualPower,
    }
}

fn fade_shape(curve: FadeCurve) -> FadeShape {
    match curve {
        FadeCurve::Linear => FadeShape::Linear,
        FadeCurve::EqualPower => FadeShape::EqualPower,
        FadeCurve::Exp => FadeShape::Exp,
    }
}

// ---------------------------------------------------------------------------
// Shared lookups + conversions
// ---------------------------------------------------------------------------

fn find_clip(app: &Resonance, clip_id: ClipId) -> Option<&ClipState> {
    app.clips.iter().find(|c| c.id == clip_id)
}

/// Reject with `not_found`, distinguishing a MIDI clip (wrong kind) from a
/// genuinely missing id — the mirror image of `notes::clip_not_found`.
fn clip_not_found(
    app: &Resonance,
    request: &Request,
    clip_id: ClipId,
) -> (Response, Task<Message>) {
    let detail = if app.midi_clips.iter().any(|c| c.id == clip_id) {
        format!("clip {clip_id} is a MIDI clip; clip.* edits audio clips (use notes.*)")
    } else {
        format!("no audio clip with id {clip_id}")
    };
    reject(request, RpcError::not_found(detail))
}

/// Resolve an [`AmountSpec`] to a frame count. `beats` converts against
/// the tempo map starting at `at_sample`, so it honours a tempo change the
/// way the grid does; `seconds` and `samples` are absolute.
fn resolve_amount(app: &Resonance, spec: &AmountSpec, at_sample: u64) -> Result<u64, RpcError> {
    match spec.given() {
        0 => {
            return Err(RpcError::invalid_params(
                "amount needs one of beats, seconds or samples",
            ))
        }
        1 => {}
        _ => {
            return Err(RpcError::invalid_params(
                "give exactly one of beats, seconds or samples",
            ))
        }
    }
    if let Some(samples) = spec.samples {
        return Ok(samples);
    }
    if let Some(seconds) = spec.seconds {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(RpcError::invalid_params(format!(
                "seconds must be finite and non-negative (got {seconds})"
            )));
        }
        // Bounded so the frame conversion (and everything downstream
        // adding it to a position) stays inside u64 arithmetic — an
        // unbounded value would dispatch a nonsense amount that later
        // overflows, wrapping into a corrupted project on save.
        if seconds > proto::MAX_SECONDS {
            return Err(RpcError::invalid_params(format!(
                "seconds value {seconds} is over the {} limit",
                proto::MAX_SECONDS as u64
            )));
        }
        return Ok((seconds * app.sample_rate as f64).round() as u64);
    }
    let beats = spec.beats.unwrap_or_default();
    if !beats.is_finite() || beats < 0.0 {
        return Err(RpcError::invalid_params(format!(
            "beats must be finite and non-negative (got {beats})"
        )));
    }
    // Same bound the `notes.*` methods put on a beat value, for the same
    // reason: keep the tick arithmetic inside u64.
    if beats > resonance_control::methods::notes::MAX_BEATS {
        return Err(RpcError::invalid_params(format!(
            "beats value {beats} is over the {} limit",
            resonance_control::methods::notes::MAX_BEATS as u64
        )));
    }
    let ticks = (beats * resonance_audio::types::TICKS_PER_QUARTER_NOTE as f64).round() as u64;
    let end = app
        .tempo_map
        .tick_to_abs_sample(at_sample, ticks, app.sample_rate);
    Ok(end.saturating_sub(at_sample))
}

/// A frame count at `start_sample` expressed in beats, the same way
/// `song.tracks` reports a clip's `length_beats`.
fn samples_to_beats(app: &Resonance, start_sample: u64, length_samples: u64) -> f64 {
    let start_tick = app.tempo_map.sample_to_abs_tick(start_sample, app.sample_rate);
    let end_tick = app
        .tempo_map
        .sample_to_abs_tick(start_sample + length_samples, app.sample_rate);
    end_tick.saturating_sub(start_tick) as f64
        / resonance_audio::types::TICKS_PER_QUARTER_NOTE as f64
}
