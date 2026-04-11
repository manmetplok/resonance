//! Audio clip handlers: async file import (spawns a decode thread),
//! move/trim/delete, direct load from in-memory samples (used by
//! project load), and bulk clip-data export (used by project save).

use std::sync::atomic::Ordering;
use std::sync::Arc;

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
                    let duration = (data.len() / 2) as u64;
                    let waveform_peaks = compute_waveform_peaks(&data);
                    let clip = AudioClip {
                        id: clip_id,
                        track_id,
                        start_sample,
                        data,
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
                    let _ = thread_event_tx
                        .send(AudioEvent::Error(format!("Failed to import clip: {}", e)));
                }
            }
            imports_counter.fetch_sub(1, Ordering::Relaxed);
        });
    if let Err(e) = spawn_result {
        state.active_imports.fetch_sub(1, Ordering::Relaxed);
        let _ = ctx
            .event_tx
            .send(AudioEvent::Error(format!("Failed to spawn decode thread: {}", e)));
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
pub(crate) fn handle_load_clip_direct(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    clip_id: ClipId,
    track_id: TrackId,
    start_sample: u64,
    data: Vec<f32>,
    name: String,
    trim_start_frames: u64,
    trim_end_frames: u64,
) {
    let total_frames = (data.len() / 2) as u64;
    let waveform_peaks = compute_waveform_peaks(&data);
    let duration_samples = total_frames
        .saturating_sub(trim_start_frames)
        .saturating_sub(trim_end_frames);
    let clip = AudioClip {
        id: clip_id,
        track_id,
        start_sample,
        data,
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

pub(crate) fn handle_export_all_clip_data(ctx: &HandlerCtx) {
    let clips_guard = ctx.clips.read();
    for clip in clips_guard.iter() {
        let _ = ctx.event_tx.send(AudioEvent::ClipDataExported {
            clip_id: clip.id,
            data: clip.data.clone(),
        });
    }
    let _ = ctx.event_tx.send(AudioEvent::AllClipDataExported);
}
