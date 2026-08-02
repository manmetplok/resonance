//! Bounce / export / stems / freeze command dispatch.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::types::*;

use super::super::{HandlerCtx, HandlerState};
use super::super::super::{bounce, bounce_realtime};

/// Shared helper: spawn an offline export job with the given path, settings,
/// and reporter variant (distinguishes the legacy `Bounce*` events from the
/// generalized `Export*` family).
fn dispatch_export(
    ctx: &HandlerCtx,
    path: String,
    settings: ExportSettings,
    reporter: bounce::ExportReporter,
) {
    bounce::export_spawn(
        path,
        settings,
        reporter,
        Arc::clone(ctx.shared),
        Arc::clone(ctx.tracks),
        Arc::clone(ctx.busses),
        Arc::clone(ctx.master),
        Arc::clone(ctx.clips),
        Arc::clone(ctx.midi_clips),
        Arc::clone(ctx.plugins),
        Arc::clone(ctx.tempo_map),
        ctx.automation.load_full(),
        ctx.sample_rate,
        ctx.event_tx.clone(),
    );
}

pub(super) fn dispatch_bounce(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    cmd: AudioCommand,
) {
    match cmd {
        // Legacy WAV bounce: a thin shim over the generalized export path
        // with default 32-bit-float WAV settings (doc #196).
        AudioCommand::BounceToWav { path } => dispatch_export(
            ctx,
            path,
            ExportSettings::default_wav(),
            bounce::ExportReporter::Bounce,
        ),
        AudioCommand::ExportAudio { path, settings } => {
            dispatch_export(ctx, path, settings, bounce::ExportReporter::Export)
        }
        AudioCommand::BounceTrackToAudio {
            source_track_id,
            target_track_id,
            target_clip_id,
            name,
        } => bounce::to_audio_clip_spawn(
            source_track_id,
            target_track_id,
            target_clip_id,
            name,
            Arc::clone(ctx.shared),
            Arc::clone(ctx.tracks),
            Arc::clone(ctx.busses),
            Arc::clone(ctx.master),
            Arc::clone(ctx.clips),
            Arc::clone(ctx.midi_clips),
            Arc::clone(ctx.plugins),
            Arc::clone(ctx.tempo_map),
            ctx.automation.load_full(),
            ctx.sample_rate,
            ctx.event_tx.clone(),
        ),
        AudioCommand::BounceTrackRealtimeToAudio {
            source_track_id,
            target_track_id,
            input_device_name,
            input_port_index,
            mono,
        } => bounce_realtime::handle_bounce_track_realtime(
            ctx,
            state,
            source_track_id,
            target_track_id,
            input_device_name,
            input_port_index,
            mono,
        ),
        AudioCommand::CancelBounce => {
            // Set the cooperative cancel flag for both bounce paths.
            // The offline renderers run on worker threads and poll the
            // flag between chunks; the realtime path picks it up on the
            // next engine-loop iteration via `poll_pending_bounce`.
            ctx.shared
                .bounce_cancel
                .store(true, Ordering::Relaxed);
        }
        AudioCommand::ExportStems {
            targets,
            range,
            sample_rate,
            bit_depth,
            include_fx_tail,
        } => bounce::export_stems_spawn(
            targets,
            range,
            sample_rate,
            bit_depth,
            include_fx_tail,
            Arc::clone(ctx.shared),
            Arc::clone(ctx.tracks),
            Arc::clone(ctx.busses),
            Arc::clone(ctx.master),
            Arc::clone(ctx.clips),
            Arc::clone(ctx.midi_clips),
            Arc::clone(ctx.plugins),
            Arc::clone(ctx.tempo_map),
            ctx.sample_rate,
            ctx.event_tx.clone(),
        ),
        AudioCommand::MeasureMix {
            measure_id,
            targets,
            range,
            source,
        } => bounce::measure_mix_spawn(
            measure_id,
            targets,
            range,
            source,
            Arc::clone(ctx.shared),
            Arc::clone(ctx.tracks),
            Arc::clone(ctx.busses),
            Arc::clone(ctx.master),
            Arc::clone(ctx.clips),
            Arc::clone(ctx.midi_clips),
            Arc::clone(ctx.plugins),
            Arc::clone(ctx.tempo_map),
            ctx.sample_rate,
            ctx.event_tx.clone(),
        ),
        AudioCommand::CancelStemExport => {
            // Shares the cooperative cancel flag with the bounce paths;
            // the stem-export worker polls it between targets.
            ctx.shared
                .bounce_cancel
                .store(true, Ordering::Relaxed);
        }
        AudioCommand::FreezeTrack {
            track_id,
            cache_path,
        } => bounce::to_freeze_cache_spawn(
            track_id,
            cache_path,
            Arc::clone(ctx.shared),
            Arc::clone(ctx.tracks),
            Arc::clone(ctx.busses),
            Arc::clone(ctx.master),
            Arc::clone(ctx.clips),
            Arc::clone(ctx.midi_clips),
            Arc::clone(ctx.plugins),
            Arc::clone(ctx.tempo_map),
            ctx.sample_rate,
            ctx.event_tx.clone(),
        ),
        AudioCommand::CancelFreeze => {
            // Freeze reuses the shared bounce-cancel atomic (the offline
            // freeze renderer polls it between chunks, same as the bounce
            // renderers). The worker drops the partial cache file and
            // emits `FreezeCancelled`.
            ctx.shared
                .bounce_cancel
                .store(true, Ordering::Relaxed);
        }
        _ => unreachable!("dispatch_bounce: unexpected command"),
    }
}
