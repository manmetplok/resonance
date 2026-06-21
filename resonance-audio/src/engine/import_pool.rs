//! Audio import-to-pool: bring one or more source files into the
//! project's media pool without placing a clip on any track.
//!
//! Each file is decoded (wav/flac/mp3/ogg via the shared symphonia
//! reader), channel up/down-mixed and resampled to the project rate (via
//! [`crate::decode::decode_file`]), copied into `{project_dir}/audio/`
//! under a stable `asset_{id}.wav` filename, and decimated to waveform
//! peaks. The work runs on a short-lived worker thread; lifecycle events
//! ([`AudioEvent::ImportProgress`] / [`AudioEvent::AssetImported`] /
//! [`AudioEvent::ImportFailed`]) flow back to the GUI through the regular
//! event channel.
//!
//! This is deliberately decoupled from [`super::clips`]: the engine does
//! not retain pool assets (the app owns the pool); it only writes the
//! engine-format WAV to disk and reports the metadata. Clip placement is
//! a separate, later step.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use resonance_common::probe_audio_file;

use crate::decode;
use crate::types::*;

use super::clips::transcode_to_wav;
use super::thread::{HandlerCtx, HandlerState};

/// Outcome of importing one source file into the project pool. Mirrors
/// the payload of [`AudioEvent::AssetImported`]; kept as a value type so
/// the pure per-file step is testable without an event channel.
#[derive(Debug, Clone, PartialEq)]
pub struct PoolImportOutcome {
    pub asset_id: AssetId,
    /// Project-relative path of the written WAV, e.g. `"audio/asset_7.wav"`.
    pub project_relative_path: String,
    pub original_path: String,
    pub format: resonance_common::AudioFormat,
    /// Channel count of the *source* file (pre-mix).
    pub channels: u16,
    /// Sample rate of the *source* file (pre-resample).
    pub source_sample_rate: u32,
    /// Per-channel frame count of the imported (project-rate) WAV.
    pub duration_frames: u64,
    pub peaks: Vec<(f32, f32)>,
}

/// The `audio/` subdirectory name and `asset_{id}.wav` stem are the
/// stable on-disk contract for pool assets. Kept here so the handler and
/// any future relink/persistence code agree on the layout.
fn asset_relative_path(asset_id: AssetId) -> String {
    format!("audio/asset_{asset_id}.wav")
}

/// Import a single source file into the pool: probe its source metadata,
/// decode + channel-mix + resample to `engine_rate`, write the
/// engine-format stereo WAV under `{project_dir}/audio/`, and compute
/// waveform peaks. Pure (no event emission, no engine state) so it can
/// be unit-tested directly. Returns a user-facing error string on any
/// failure (missing/corrupt file, decode error, write error).
pub fn import_one_to_pool(
    asset_id: AssetId,
    src_path: &str,
    project_dir: &Path,
    engine_rate: u32,
) -> Result<PoolImportOutcome, String> {
    // Source metadata for display (format / channels / original rate).
    // Cheap for WAV/FLAC (declared frame counts); the media browser's
    // probe helper handles the compressed-format fallback.
    let info = probe_audio_file(Path::new(src_path))?;

    // Decode + up/down-mix to stereo + resample to the project rate in
    // one pass. `decode_file` returns stereo-interleaved f32 already at
    // `engine_rate`, so mismatched-rate sources land at correct
    // pitch/speed and the project stays self-contained.
    let (data, _name) = decode::decode_file(src_path, engine_rate)?;

    let project_relative_path = asset_relative_path(asset_id);
    let target = project_dir.join(&project_relative_path);
    transcode_to_wav(&target, &data, engine_rate)?;

    let peaks = compute_waveform_peaks(&data);
    let duration_frames = (data.len() / 2) as u64;

    Ok(PoolImportOutcome {
        asset_id,
        project_relative_path,
        original_path: src_path.to_string(),
        format: info.format,
        channels: info.channels,
        source_sample_rate: info.sample_rate,
        duration_frames,
        peaks,
    })
}

/// Run an import batch, emitting the full per-file event lifecycle
/// through `emit`. Generic over the sink so the engine handler can pass
/// a channel-backed closure while tests collect the events into a `Vec`.
///
/// Ordering: every job is reported `Queued` up front (so the modal can
/// render all rows immediately), then jobs are processed sequentially —
/// each flips to `Working`, then either emits `AssetImported` followed
/// by `Done`, or terminates with `ImportFailed`. Files are independent:
/// one failure never aborts the rest of the batch.
pub fn run_pool_import(
    jobs: &[(AssetId, String)],
    project_dir: &Path,
    engine_rate: u32,
    mut emit: impl FnMut(AudioEvent),
) {
    for (asset_id, path) in jobs {
        emit(AudioEvent::ImportProgress {
            asset_id: *asset_id,
            path: path.clone(),
            stage: ImportStage::Queued,
        });
    }

    for (asset_id, path) in jobs {
        emit(AudioEvent::ImportProgress {
            asset_id: *asset_id,
            path: path.clone(),
            stage: ImportStage::Working,
        });
        match import_one_to_pool(*asset_id, path, project_dir, engine_rate) {
            Ok(outcome) => {
                emit(AudioEvent::AssetImported {
                    asset_id: outcome.asset_id,
                    project_relative_path: outcome.project_relative_path,
                    original_path: outcome.original_path,
                    format: outcome.format,
                    channels: outcome.channels,
                    source_sample_rate: outcome.source_sample_rate,
                    duration_frames: outcome.duration_frames,
                    peaks: outcome.peaks,
                });
                emit(AudioEvent::ImportProgress {
                    asset_id: *asset_id,
                    path: path.clone(),
                    stage: ImportStage::Done,
                });
            }
            Err(reason) => {
                emit(AudioEvent::ImportFailed {
                    asset_id: *asset_id,
                    path: path.clone(),
                    reason,
                });
            }
        }
    }
}

pub(crate) fn handle_import_audio_to_pool(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    paths: Vec<String>,
) {
    // A project directory is the destination for the transcoded WAVs;
    // startup enforces an active project, so this normally holds. Treat
    // its absence as a single setup error (mirrors `handle_import_clip`)
    // rather than a per-file failure — it's a precondition, not a
    // problem with any one source file.
    let project_dir = match state.project_dir.clone() {
        Some(dir) => dir,
        None => {
            let _ = ctx.event_tx.send(AudioEvent::Error(
                "Cannot import audio: no project directory set.".into(),
            ));
            return;
        }
    };

    if paths.is_empty() {
        return;
    }

    // Allocate a stable asset id per file on the engine thread so
    // concurrent/back-to-back batches never collide, then move the work
    // list onto the worker.
    let jobs: Vec<(AssetId, String)> = paths
        .into_iter()
        .map(|path| {
            let id = state.next_asset_id;
            state.next_asset_id += 1;
            (id, path)
        })
        .collect();

    let event_tx = ctx.event_tx.clone();
    let engine_rate = ctx.sample_rate;
    let imports_counter = Arc::clone(&state.active_imports);
    imports_counter.fetch_add(1, Ordering::Relaxed);

    let spawn_result = std::thread::Builder::new()
        .name("resonance-pool-import".into())
        .spawn(move || {
            run_pool_import(&jobs, &project_dir, engine_rate, |ev| {
                let _ = event_tx.send(ev);
            });
            imports_counter.fetch_sub(1, Ordering::Relaxed);
        });
    if let Err(e) = spawn_result {
        state.active_imports.fetch_sub(1, Ordering::Relaxed);
        let _ = ctx.event_tx.send(AudioEvent::Error(format!(
            "Failed to spawn pool-import thread: {e}"
        )));
    }
}
