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

use resonance_common::probe_audio_file;
use thiserror::Error;

use crate::decode;
use crate::types::*;

use super::clips::{transcode_to_wav_new, TranscodeError};
use super::thread::{HandlerCtx, HandlerState};

/// Failure importing one source file into the pool
/// ([`import_one_to_pool`]). `Probe`/`Decode` wrap `resonance_common`'s
/// typed errors (C-4); `Transcode` wraps the engine's own typed write
/// error; `Panic` is the decoder-panic fallback text from
/// [`run_pool_import_with`].
#[derive(Debug, Error)]
pub enum ImportError {
    #[error(transparent)]
    Probe(#[from] resonance_common::AudioProbeError),
    #[error(transparent)]
    Decode(#[from] resonance_common::WavDecodeError),
    #[error(transparent)]
    Transcode(#[from] TranscodeError),
    #[error("Import failed: decoder panicked: {0}")]
    Panic(String),
    /// Short-circuit for a batch job whose project closed mid-run
    /// (`stale()` in [`handle_import_audio_to_pool`]'s worker closure);
    /// immediately superseded by [`POOL_IMPORT_CANCELLED`] once the
    /// event reaches [`cancellation`], never shown to the user as-is.
    #[error("project closed")]
    Stale,
}

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
/// be unit-tested directly. Returns a typed error on any failure
/// (missing/corrupt file, decode error, write error).
pub fn import_one_to_pool(
    asset_id: AssetId,
    src_path: &str,
    project_dir: &Path,
    engine_rate: u32,
) -> Result<PoolImportOutcome, ImportError> {
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
    // `create_new`: the app is the pool's only id allocator (D-7a) and
    // never hands out an id twice, but the engine keeps no registry of
    // its own to check that against, so refusing to clobber an existing
    // file is the one guard it CAN enforce — a stale/orphaned
    // `asset_<id>.wav` a prior undone import left behind (or a duplicate
    // id from an app bug) is reported rather than silently overwritten.
    transcode_to_wav_new(&target, &data, engine_rate)?;

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
/// one failure never aborts the rest of the batch — and neither does a
/// panic (see [`run_pool_import_with`]).
pub fn run_pool_import(
    jobs: &[(AssetId, String)],
    project_dir: &Path,
    engine_rate: u32,
    emit: impl FnMut(AudioEvent),
) {
    run_pool_import_with(jobs, project_dir, engine_rate, import_one_to_pool, emit);
}

/// [`run_pool_import`] with the per-file step injected, so a test can make
/// one file's import panic.
///
/// Each file's import runs under `catch_unwind` (code review ENG-09): a
/// decoder panic on a truncated or crafted file becomes that file's
/// `ImportFailed` — the row resolves instead of sitting at "Working" —
/// and the batch moves on to the next file.
pub fn run_pool_import_with(
    jobs: &[(AssetId, String)],
    project_dir: &Path,
    engine_rate: u32,
    mut import: impl FnMut(AssetId, &str, &Path, u32) -> Result<PoolImportOutcome, ImportError>,
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
        // `AssertUnwindSafe`: a panicking import leaves nothing shared
        // behind — its partial output is only the asset file, which the
        // failure event tells the app to ignore.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            import(*asset_id, path, project_dir, engine_rate)
        }))
        .unwrap_or_else(|payload| {
            Err(ImportError::Panic(
                crate::supervise::panic_message(payload.as_ref()).to_string(),
            ))
        });
        match result {
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
                    reason: reason.to_string(),
                });
            }
        }
    }
}

/// Reason reported for each file of a pool-import batch whose project was
/// closed before it finished (FU-A4b).
pub const POOL_IMPORT_CANCELLED: &str =
    "Import cancelled: the project was closed before it finished.";

/// The one terminal event a stale batch sends for the file `ev` is about,
/// the first time that file comes up after the project closed: an
/// `ImportFailed` with [`POOL_IMPORT_CANCELLED`]. `None` for a file already
/// cancelled, or an event that names no file of this batch.
fn cancellation(
    jobs: &[(AssetId, String)],
    cancelled: &mut Vec<AssetId>,
    ev: &AudioEvent,
) -> Option<AudioEvent> {
    let asset_id = match ev {
        AudioEvent::ImportProgress { asset_id, .. }
        | AudioEvent::ImportFailed { asset_id, .. }
        | AudioEvent::AssetImported { asset_id, .. } => *asset_id,
        _ => return None,
    };
    if cancelled.contains(&asset_id) {
        return None;
    }
    let (_, path) = jobs.iter().find(|(id, _)| *id == asset_id)?;
    cancelled.push(asset_id);
    Some(AudioEvent::ImportFailed {
        asset_id,
        path: path.clone(),
        reason: POOL_IMPORT_CANCELLED.into(),
    })
}

pub(crate) fn handle_import_audio_to_pool(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    files: Vec<PoolImportFile>,
) {
    // A project directory is the destination for the transcoded WAVs;
    // startup enforces an active project, so this normally holds. Treat
    // its absence as a single setup error (mirrors `handle_import_clip`)
    // rather than a per-file failure — it's a precondition, not a
    // problem with any one source file.
    let project_dir = match state.project_dir.clone() {
        Some(dir) => dir,
        None => {
            let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::internal(
                "Cannot import audio: no project directory set.",
            )));
            return;
        }
    };

    if files.is_empty() {
        return;
    }

    // The app allocated every asset id up front (D-7a: the app is the
    // pool's only id allocator); move the work list onto the worker as-is.
    let jobs: Vec<(AssetId, String)> = files
        .into_iter()
        .map(|f| (f.asset_id, f.path))
        .collect();

    let event_tx = ctx.event_tx.clone();
    let engine_rate = ctx.sample_rate;
    // Project fence (FU-M4a, like UPD-09's clip imports): `ClearAll` bumps
    // the generation, then takes `pool_import_fence` once, and sends
    // `AllCleared` after it. Every event of this batch is sent holding that
    // fence after re-checking the generation, so a batch outlived by its
    // project stops emitting before `AllCleared` and never lands in the
    // new project's pool. (The fence used to be a read of the clip list's
    // lock; the clip list is in the render graph since ARCH-02 B-5.) Its unresolved files each get one final
    // `ImportFailed` ("cancelled") instead, so the import modal's rows and
    // any control-API import job waiting on them resolve (FU-A4b).
    let fence = std::sync::Arc::clone(&state.pool_import_fence);
    let clear_generation = std::sync::Arc::clone(&state.clear_generation);
    let generation = clear_generation.load(std::sync::atomic::Ordering::SeqCst);

    // One dedicated thread per batch, deliberately *not* the shared
    // `HandlerState::imports` pool: a pool-import batch is a foreground,
    // user-initiated action with its own progress UI, and queueing it
    // behind a project load's clip backlog would leave the import modal
    // sitting at "Queued". The batch itself is sequential, so a single
    // extra thread is the whole cost.
    let spawn_result = std::thread::Builder::new()
        .name("resonance-pool-import".into())
        .spawn(move || {
            // Per-file panics are contained inside `run_pool_import`; this
            // catches anything left (event emission, bookkeeping) so the
            // thread never dies silently (code review ENG-09).
            let panic_tx = event_tx.clone();
            crate::supervise::run_supervised(
                "pool-import",
                || {
                    let stale =
                        || clear_generation.load(std::sync::atomic::Ordering::SeqCst) != generation;
                    let mut cancelled: Vec<AssetId> = Vec::new();
                    run_pool_import_with(
                        &jobs,
                        &project_dir,
                        engine_rate,
                        |asset_id, path, dir, rate| {
                            // Skip the decode of the rest of a stale batch.
                            if stale() {
                                return Err(ImportError::Stale);
                            }
                            import_one_to_pool(asset_id, path, dir, rate)
                        },
                        |ev| {
                            let _fence = fence.lock();
                            if stale() {
                                // The WAV went into the old project's folder
                                // and nothing will reference it.
                                if let AudioEvent::AssetImported {
                                    project_relative_path,
                                    ..
                                } = &ev
                                {
                                    let _ = std::fs::remove_file(
                                        project_dir.join(project_relative_path),
                                    );
                                }
                                if let Some(ev) = cancellation(&jobs, &mut cancelled, &ev) {
                                    let _ = event_tx.send(ev);
                                }
                                return;
                            }
                            let _ = event_tx.send(ev);
                        },
                    );
                },
                |message| {
                    let _ = panic_tx.send(AudioEvent::Error(EngineError::internal(message)));
                },
            );
        });
    if let Err(e) = spawn_result {
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::io(format!(
            "Failed to spawn pool-import thread: {e}"
        ))));
    }
}
