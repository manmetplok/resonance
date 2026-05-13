//! Audio clip handlers: async file import (spawns a decode thread,
//! transcodes to WAV, mmaps), move/trim/delete, mmap-backed load
//! from a WAV file on disk (project load), and
//! ensure-all-clips-have-wav-files (project save).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use hound::{SampleFormat, WavSpec, WavWriter};

use crate::decode;
use crate::types::*;

use super::thread::{HandlerCtx, HandlerState, MAX_CONCURRENT_IMPORTS};

pub(crate) fn handle_import_clip(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    path: String,
    start_sample: u64,
) {
    if state.active_imports.load(Ordering::Relaxed) >= MAX_CONCURRENT_IMPORTS {
        eprintln!(
            "Warning: too many concurrent imports ({MAX_CONCURRENT_IMPORTS}), skipping import of {:?}",
            path
        );
        let _ = ctx.event_tx.send(AudioEvent::Error(
            "Too many concurrent imports, please wait for current imports to finish.".to_string(),
        ));
        return;
    }

    // Import needs a project directory to transcode the decoded
    // samples into. Startup enforces an active project, so this
    // should always hold.
    let project_dir = match state.project_dir.clone() {
        Some(dir) => dir,
        None => {
            let _ = ctx.event_tx.send(AudioEvent::Error(
                "Cannot import clip: no project directory set.".into(),
            ));
            return;
        }
    };

    let clips_arc = Arc::clone(ctx.clips);
    let thread_event_tx = ctx.event_tx.clone();
    let clip_id = state.next_clip_id;
    state.next_clip_id += 1;
    let sr = ctx.sample_rate;
    let imports_counter = Arc::clone(&state.active_imports);
    imports_counter.fetch_add(1, Ordering::Relaxed);

    let spawn_result = std::thread::Builder::new()
        .name("resonance-decode".into())
        .spawn(move || {
            match decode::decode_file(&path, sr) {
                Ok((data, name)) => {
                    let target = project_dir
                        .join("audio")
                        .join(format!("clip_{clip_id}.wav"));
                    match transcode_to_wav(&target, &data, sr) {
                        Ok(()) => match ClipSource::open_wav(&target) {
                            Ok(source) => {
                                let duration = source.frame_count();
                                let waveform_peaks = compute_waveform_peaks(source.as_frames());
                                let clip = AudioClip {
                                    id: clip_id,
                                    track_id,
                                    start_sample,
                                    source,
                                    name: name.clone(),
                                    trim_start_frames: 0,
                                    trim_end_frames: 0,
                                };
                                clips_arc.write().push(clip);
                                let _ = thread_event_tx.send(AudioEvent::ClipImported {
                                    clip_id,
                                    track_id,
                                    start_sample,
                                    duration_samples: duration,
                                    name,
                                    waveform_peaks,
                                });
                            }
                            Err(e) => {
                                let _ = thread_event_tx.send(AudioEvent::Error(format!(
                                    "Failed to mmap imported clip: {e}"
                                )));
                            }
                        },
                        Err(e) => {
                            let _ = thread_event_tx.send(AudioEvent::Error(format!(
                                "Failed to transcode imported clip to WAV: {e}"
                            )));
                        }
                    }
                }
                Err(e) => {
                    let _ = thread_event_tx
                        .send(AudioEvent::Error(format!("Failed to import clip: {}", e)));
                }
            }
            imports_counter.fetch_sub(1, Ordering::Relaxed);
        });
    if let Err(e) = spawn_result {
        state.active_imports.fetch_sub(1, Ordering::Relaxed);
        let _ = ctx.event_tx.send(AudioEvent::Error(format!(
            "Failed to spawn decode thread: {}",
            e
        )));
    }
}

pub(crate) fn handle_move_clip(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    new_start_sample: u64,
    new_track_id: TrackId,
) {
    let mut clips = ctx.clips.write();
    if let Some(clip) = clips.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.track_id = new_track_id;
        let _ = ctx.event_tx.send(AudioEvent::ClipMoved {
            clip_id,
            new_start_sample,
            new_track_id,
        });
    }
}

pub(crate) fn handle_trim_clip(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    new_start_sample: u64,
    trim_start_frames: u64,
    trim_end_frames: u64,
) {
    let mut clips = ctx.clips.write();
    if let Some(clip) = clips.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.trim_start_frames = trim_start_frames;
        clip.trim_end_frames = trim_end_frames;
        let _ = ctx.event_tx.send(AudioEvent::ClipTrimmed {
            clip_id,
            new_start_sample,
            new_duration_samples: clip.duration_frames(),
            trim_start_frames,
            trim_end_frames,
        });
    }
}

pub(crate) fn handle_delete_clip(ctx: &HandlerCtx, clip_id: ClipId) {
    ctx.clips.write().retain(|c| c.id != clip_id);
    let _ = ctx.event_tx.send(AudioEvent::ClipDeleted { clip_id });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_load_clip_from_wav(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    clip_id: ClipId,
    track_id: TrackId,
    start_sample: u64,
    path: PathBuf,
    name: String,
    trim_start_frames: u64,
    trim_end_frames: u64,
) {
    let source = match ClipSource::open_wav(&path) {
        Ok(s) => s,
        Err(e) => {
            let _ = ctx
                .event_tx
                .send(AudioEvent::Error(format!("Failed to load clip WAV: {e}")));
            return;
        }
    };

    let total_frames = source.frame_count();
    let waveform_peaks = compute_waveform_peaks(source.as_frames());
    let duration_samples = total_frames
        .saturating_sub(trim_start_frames)
        .saturating_sub(trim_end_frames);

    let clip = AudioClip {
        id: clip_id,
        track_id,
        start_sample,
        source,
        name: name.clone(),
        trim_start_frames,
        trim_end_frames,
    };
    ctx.clips.write().push(clip);
    state.next_clip_id = state.next_clip_id.max(clip_id + 1);
    let _ = ctx.event_tx.send(AudioEvent::ClipImported {
        clip_id,
        track_id,
        start_sample,
        duration_samples,
        name,
        waveform_peaks,
    });
}

/// Guarantee that every in-engine audio clip has a WAV file on disk
/// at `{project_dir}/audio/clip_{id}.wav`. Recorded and imported
/// clips are already `ClipSource::Mapped` and just need their path
/// returned; any remaining `ClipSource::Memory` clips get transcoded.
pub(crate) fn handle_save_clips_to_project_dir(ctx: &HandlerCtx, state: &mut HandlerState) {
    let project_dir = match state.project_dir.clone() {
        Some(dir) => dir,
        None => {
            let _ = ctx.event_tx.send(AudioEvent::Error(
                "Cannot save clips: no project directory set.".into(),
            ));
            return;
        }
    };

    // Collect the per-clip work list while holding only a read
    // lock so in-memory or cross-directory clips we need to
    // transcode/copy don't block the mixer any longer than
    // necessary.
    //
    // For each clip we categorize as:
    //   Ready   — already at the target path, no work
    //   Copy    — `Mapped` at a different path (save-as case)
    //   Encode  — `Memory` (transient imports)
    enum Action {
        Ready,
        Copy(PathBuf),
        Encode(Vec<f32>),
    }
    let mut entries: Vec<(ClipId, String, Action)> = Vec::new();
    {
        let clips_guard = ctx.clips.read();
        for clip in clips_guard.iter() {
            let rel = format!("audio/clip_{}.wav", clip.id);
            let target = project_dir.join(&rel);
            let action = match &clip.source {
                ClipSource::Mapped { path, .. } => {
                    if path == &target {
                        Action::Ready
                    } else {
                        Action::Copy(path.clone())
                    }
                }
                ClipSource::Memory(v) => Action::Encode(v.clone()),
            };
            entries.push((clip.id, rel, action));
        }
    }

    let sr = ctx.sample_rate;
    let mut needs_remap: Vec<ClipId> = Vec::new();
    for (clip_id, _rel, action) in &entries {
        let target = project_dir
            .join("audio")
            .join(format!("clip_{clip_id}.wav"));
        match action {
            Action::Ready => {}
            Action::Copy(src_path) => {
                if let Some(parent) = target.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        let _ = ctx
                            .event_tx
                            .send(AudioEvent::Error(format!("Create audio dir: {e}")));
                        return;
                    }
                }
                if let Err(e) = std::fs::copy(src_path, &target) {
                    let _ = ctx
                        .event_tx
                        .send(AudioEvent::Error(format!("Copy clip {clip_id} WAV: {e}")));
                    return;
                }
                needs_remap.push(*clip_id);
            }
            Action::Encode(samples) => {
                if let Err(e) = transcode_to_wav(&target, samples, sr) {
                    let _ = ctx.event_tx.send(AudioEvent::Error(format!(
                        "Transcode clip {clip_id} to WAV: {e}"
                    )));
                    return;
                }
                needs_remap.push(*clip_id);
            }
        }
    }

    // Re-open the mmap for each clip whose backing file we just
    // wrote, so playback reads from the file inside the new project
    // dir and future saves are no-ops.
    for clip_id in needs_remap {
        let target = project_dir
            .join("audio")
            .join(format!("clip_{clip_id}.wav"));
        if let Ok(source) = ClipSource::open_wav(&target) {
            let mut clips_guard = ctx.clips.write();
            if let Some(clip) = clips_guard.iter_mut().find(|c| c.id == clip_id) {
                clip.source = source;
            }
        }
    }

    let clip_files: Vec<(ClipId, String)> =
        entries.into_iter().map(|(id, rel, _)| (id, rel)).collect();
    let _ = ctx
        .event_tx
        .send(AudioEvent::ClipsSavedToProjectDir { clip_files });
}

/// Write a stereo-interleaved f32 buffer to a 32-bit float WAV.
/// Creates the target directory if needed. Used by both the import
/// transcode path and the save-time fallback for in-RAM clips.
pub fn transcode_to_wav(path: &Path, samples: &[f32], sample_rate: u32) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let spec = WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer =
        WavWriter::create(path, spec).map_err(|e| format!("create {}: {e}", path.display()))?;
    for &s in samples {
        writer
            .write_sample(s)
            .map_err(|e| format!("write sample: {e}"))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("finalize wav: {e}"))?;
    Ok(())
}
