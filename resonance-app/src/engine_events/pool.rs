//! App-side handlers for the media-pool import lifecycle (doc #175, ba
//! todos #597 / #598) — the per-file progress mirror and the placement
//! half of the import orchestration.
//!
//! The engine copies/transcodes each imported source file off-thread and
//! reports back per file: an ordered `ImportProgress` lifecycle, then a
//! terminal `AssetImported` (success) or `ImportFailed` (error). On
//! success we mirror the asset into the project pool and — if the import
//! was a drop with a queued placement (see `state::pool_import`) — place
//! it as an audio clip on the target track, reusing the engine's
//! clip-from-WAV load path and tying the new clip to its `AssetRef`.
//!
//! The per-file progress state (`Resonance::import_progress`) feeds the
//! transcode-progress modal (todo #606); it is transient UI state — not
//! undoable and not persisted (same rule as audition state, doc #175).
//!
//! The single-action undo entry for the whole import + placement was
//! recorded up front when the import was issued (`undo::classify` marks
//! `PoolMessage` as `Record`), so nothing here touches the undo history:
//! one undo of that pre-import snapshot removes the pool asset, the placed
//! clip, and any track spawned for a new-track drop.

use std::path::Path;

use resonance_audio::types::*;
use resonance_common::AudioFormat;

use crate::state::{AssetRef, ClipState, FileImportProgress, PlacementTarget, PoolAsset};
use crate::Resonance;

/// Mirror a freshly imported asset into the pool, then place it if a drop
/// queued a placement for its source file. Idempotent on the pool add
/// (`MediaPool::add` replaces an existing id in place), so a re-import or
/// a duplicate event refreshes metadata rather than duplicating the asset.
#[allow(clippy::too_many_arguments)]
pub(super) fn asset_imported(
    r: &mut Resonance,
    asset_id: AssetId,
    project_relative_path: String,
    original_path: String,
    format: AudioFormat,
    channels: u16,
    source_sample_rate: u32,
    duration_frames: u64,
    peaks: Vec<(f32, f32)>,
) {
    r.add_pool_asset(PoolAsset {
        id: asset_id,
        project_relative_path: project_relative_path.clone(),
        original_path: original_path.clone(),
        format,
        channels,
        source_sample_rate,
        duration_frames,
        thumbnail_peaks: peaks.clone(),
        missing: false,
    });

    // Place the asset as a clip if this file's import queued one. A
    // pool-only import (dialog / `PoolOnly`) queues no clip; a stray
    // asset with no matching entry is left in the pool unplaced.
    let placed = match r.pool_import.take_matching(&original_path) {
        // The target track was deleted while the file transcoded: drop
        // the placement rather than push a clip onto a dead id (code
        // review UPD-04). A waiting `clip.place` job fails below.
        Some(PlacementTarget::Track { track_id, .. })
            if !r.registry.tracks.iter().any(|t| t.id == track_id) =>
        {
            None
        }
        Some(PlacementTarget::Track {
            track_id,
            start_sample,
        }) => Some(place_clip(
            r,
            asset_id,
            &project_relative_path,
            &original_path,
            start_sample,
            duration_frames,
            peaks,
            track_id,
        )),
        Some(PlacementTarget::PoolOnly) | None => None,
    };

    resolve_control_import(r, &original_path, None, asset_id, placed);
}

/// Tick this source file off any control-endpoint import job waiting on it
/// and, for a batch whose last file just landed, build that job's result
/// from what actually arrived (ba doc #265, `pool.import` / `clip.place`).
///
/// The result shape depends on which method started the job — a
/// `clip.place` reports the placed clip, a `pool.import` the assets — so
/// the job's `kind` selects it. A batch with any failed file resolves as an
/// error naming the first failure; its successful files stay in the pool
/// and are visible to `pool.list`.
fn resolve_control_import(
    r: &mut Resonance,
    source_path: &str,
    error: Option<&str>,
    asset_id: AssetId,
    placed: Option<ClipId>,
) {
    let finished = r.control.jobs.tick_import_path(source_path, error);
    for (job_id, batch_error) in finished {
        if let Some(message) = batch_error {
            r.control.jobs.fail(job_id, message);
            continue;
        }
        let kind = r.control.jobs.kind_of(job_id);
        let result = match kind.as_deref() {
            Some(resonance_control::methods::clip::PLACE) => match placed {
                Some(clip_id) => serde_json::to_value(
                    crate::update::control::place_result(r, clip_id, asset_id),
                )
                .unwrap_or(serde_json::Value::Null),
                // The import succeeded but no clip came out of it — the
                // queued placement was dropped (its track went away).
                // Failing is honest; the asset is still in the pool.
                None => {
                    r.control.jobs.fail(
                        job_id,
                        "the file imported but its placement did not run \
                         (was the target track deleted?)",
                    );
                    continue;
                }
            },
            _ => {
                let paths = r.control.jobs.import_batch_paths(job_id);
                serde_json::to_value(crate::update::control::import_result(r, &paths))
                    .unwrap_or(serde_json::Value::Null)
            }
        };
        r.control.jobs.complete(job_id, result);
    }
}

/// Update the per-file progress tracker from an `ImportProgress` engine
/// event. This drives the transcode modal (todo #606) and is transient UI
/// state — not undoable, not persisted.
pub(super) fn import_progress(
    r: &mut Resonance,
    asset_id: AssetId,
    path: String,
    stage: ImportStage,
) {
    let progress = match stage {
        ImportStage::Queued => FileImportProgress::Queued,
        ImportStage::Working => FileImportProgress::Working,
        ImportStage::Done => FileImportProgress::Done,
    };
    r.import_progress.upsert(asset_id, path, progress);
}

/// A source file failed to import (decode/transcode error, missing file,
/// …). Drop its queued placement so a later stray event can't place a
/// phantom clip, update the progress tracker with the failure reason, and
/// surface the reason. The batch's other files are independent and
/// continue.
pub(super) fn import_failed(r: &mut Resonance, asset_id: AssetId, path: String, reason: String) {
    let _ = r.pool_import.take_matching(&path);
    r.import_progress.upsert(
        asset_id,
        path.clone(),
        FileImportProgress::Failed {
            reason: reason.clone(),
        },
    );
    r.banners.error_message = Some(format!("Import failed: {reason}"));
    resolve_control_import(r, &path, Some(&reason), asset_id, None);
}

/// Place an imported asset as an audio clip on `track_id` at
/// `start_sample`. Allocates an app-side clip id (same high-range
/// allocator the bounce path uses), hands the engine the asset's WAV via
/// `LoadClipFromWav`, and pushes the mirrored [`ClipState`] with its
/// `asset_ref` set so usage counts and persistence reconnect the clip to
/// its pool asset. Mirrors the project-load clip-replay path, which also
/// pushes `ClipState` directly rather than waiting on a `ClipImported`
/// echo.
#[allow(clippy::too_many_arguments)]
fn place_clip(
    r: &mut Resonance,
    asset_id: AssetId,
    project_relative_path: &str,
    original_path: &str,
    start_sample: SamplePos,
    duration_frames: u64,
    peaks: Vec<(f32, f32)>,
    track_id: TrackId,
) -> ClipId {
    let clip_id = r.compose.fresh_derived_clip_id();
    place_clip_with_id(
        r,
        clip_id,
        asset_id,
        project_relative_path,
        original_path,
        start_sample,
        duration_frames,
        peaks,
        track_id,
    );
    clip_id
}

/// [`place_clip`] with the clip id supplied by the caller. The control
/// endpoint's `clip.place` allocates the id up front — the same trick
/// `notes.create_clip` uses — so its reply can name the clip without
/// waiting for any engine echo.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place_clip_with_id(
    r: &mut Resonance,
    clip_id: ClipId,
    asset_id: AssetId,
    project_relative_path: &str,
    original_path: &str,
    start_sample: SamplePos,
    duration_frames: u64,
    peaks: Vec<(f32, f32)>,
    track_id: TrackId,
) {
    let name = clip_name_from(original_path);

    // Resolve the engine-format WAV (which the engine wrote into the
    // project's `audio/` dir on import) to an absolute path for the mmap
    // load. `project_path` is guaranteed set — the import handler refuses
    // to run without it.
    if let Some(project_dir) = r.io.project_path.clone() {
        let abs_path = project_dir.join(project_relative_path);
        let _ = r.engine.send(AudioCommand::LoadClipFromWav {
            clip_id,
            track_id,
            start_sample,
            path: abs_path,
            name: name.clone(),
            trim_start_frames: 0,
            trim_end_frames: 0,
        });
    }

    r.clips.push(ClipState {
        id: clip_id,
        track_id,
        start_sample,
        duration_samples: duration_frames,
        name,
        total_frames: duration_frames,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: peaks,
        vocal_tuning: None,
        // The whole point of the placement: tie the clip to its pool asset
        // so usage counts, persistence, and relink all reconnect on load.
        asset_ref: Some(AssetRef::new(asset_id)),
    });

    // A new clip now references the asset — refresh the pool usage counts.
    r.recompute_pool_usage();
}

/// Derive a clip name from the imported source file: its file stem, or a
/// generic fallback when the path has none.
fn clip_name_from(original_path: &str) -> String {
    Path::new(original_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Audio clip".to_string())
}
