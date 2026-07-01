//! App-side handlers for the media-pool import lifecycle (doc #175, ba
//! todo #598) — the placement half of the import orchestration.
//!
//! The engine copies/transcodes each imported source file off-thread and
//! reports back per file: an ordered `ImportProgress` lifecycle, then a
//! terminal `AssetImported` (success) or `ImportFailed` (error). On
//! success we mirror the asset into the project pool and — if the import
//! was a drop with a queued placement (see `state::pool_import`) — place
//! it as an audio clip on the target track, reusing the engine's
//! clip-from-WAV load path and tying the new clip to its `AssetRef`.
//!
//! The single-action undo entry for the whole import + placement was
//! recorded up front when the import was issued (`undo::classify` marks
//! `PoolMessage` as `Record`), so nothing here touches the undo history:
//! one undo of that pre-import snapshot removes the pool asset, the placed
//! clip, and any track spawned for a new-track drop.

use std::path::Path;

use resonance_audio::types::*;
use resonance_common::AudioFormat;

use crate::state::{AssetRef, ClipState, PlacementTarget, PoolAsset};
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
    match r.pool_import.take_matching(&original_path) {
        Some(PlacementTarget::Track {
            track_id,
            start_sample,
        }) => place_clip(
            r,
            asset_id,
            &project_relative_path,
            &original_path,
            start_sample,
            duration_frames,
            peaks,
            track_id,
        ),
        Some(PlacementTarget::PoolOnly) | None => {}
    }
}

/// A source file failed to import (decode/transcode error, missing file,
/// …). Drop its queued placement so a later stray event can't place a
/// phantom clip, and surface the reason. The batch's other files are
/// independent and continue.
pub(super) fn import_failed(r: &mut Resonance, _asset_id: AssetId, path: String, reason: String) {
    let _ = r.pool_import.take_matching(&path);
    r.error_message = Some(format!("Import failed: {reason}"));
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
) {
    let clip_id = r.compose.fresh_derived_clip_id();
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
