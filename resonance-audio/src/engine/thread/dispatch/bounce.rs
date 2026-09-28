//! Bounce / export / stems / freeze command dispatch.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::types::*;

use super::super::{HandlerCtx, HandlerState};
use super::super::super::{bounce, bounce_realtime};

/// Flip a render's cancel token, if that render was ever started. Each
/// render polls only its own token (see `HandlerState::bounce_cancel`),
/// so a cancel can never abort a different concurrently-running render
/// and can never be lost to a later render starting.
fn cancel(token: &Option<Arc<AtomicBool>>) {
    if let Some(token) = token {
        token.store(true, Ordering::Relaxed);
    }
}

/// Shared helper: spawn an offline export job with the given path, settings,
/// and reporter variant (distinguishes the legacy `Bounce*` events from the
/// generalized `Export*` family). Returns the spawned render's cancel token.
fn dispatch_export(
    ctx: &HandlerCtx,
    path: String,
    settings: ExportSettings,
    reporter: bounce::ExportReporter,
) -> Arc<AtomicBool> {
    bounce::export_spawn(
        path,
        settings,
        reporter,
        Arc::clone(ctx.shared),
        Arc::clone(ctx.tempo_map),
        ctx.automation.load_full(),
        ctx.sample_rate,
        ctx.event_tx.clone(),
    )
}

pub(super) fn dispatch_bounce(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    cmd: AudioCommand,
) {
    // Every offline render reads the frozen caches: land the ones still
    // being converted to the engine rate first (FU-A4c).
    crate::engine::tracks::settle_frozen_conversions(ctx, state, true);
    match cmd {
        // Legacy WAV bounce: a thin shim over the generalized export path
        // with default 32-bit-float WAV settings (doc #196).
        AudioCommand::BounceToWav { path } => {
            state.bounce_cancel = Some(dispatch_export(
                ctx,
                path,
                ExportSettings::default_wav(),
                bounce::ExportReporter::Bounce,
            ));
        }
        AudioCommand::ExportAudio { path, settings } => {
            state.bounce_cancel =
                Some(dispatch_export(ctx, path, settings, bounce::ExportReporter::Export));
        }
        AudioCommand::BounceTrackToAudio {
            source_track_id,
            target_track_id,
            target_clip_id,
            name,
        } => {
            state.bounce_cancel = Some(bounce::to_audio_clip_spawn(
                source_track_id,
                target_track_id,
                target_clip_id,
                name,
                Arc::clone(ctx.shared),
                Arc::clone(ctx.tempo_map),
                ctx.automation.load_full(),
                ctx.sample_rate,
                ctx.event_tx.clone(),
                ctx.cmd_tx_retry.clone(),
            ));
        }
        AudioCommand::BounceTargetCancelled { target_track_id } => {
            bounce_realtime::remove_cancelled_bounce_target(ctx, target_track_id);
        }
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
            // Flip the current bounce/export render's own token. The
            // offline renderers run on worker threads and poll it
            // between chunks; the realtime path picks it up on the
            // next engine-loop iteration via `poll_pending_bounce`.
            cancel(&state.bounce_cancel);
        }
        AudioCommand::ExportStems {
            targets,
            range,
            sample_rate,
            bit_depth,
            include_fx_tail,
        } => {
            state.stem_cancel = Some(bounce::export_stems_spawn(
                targets,
                range,
                sample_rate,
                bit_depth,
                include_fx_tail,
                Arc::clone(ctx.shared),
                Arc::clone(ctx.tempo_map),
                ctx.automation.load_full(),
                ctx.sample_rate,
                ctx.event_tx.clone(),
            ));
        }
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
            Arc::clone(ctx.tempo_map),
            ctx.automation.load_full(),
            ctx.sample_rate,
            ctx.event_tx.clone(),
        ),
        AudioCommand::CancelStemExport => {
            // The stem-export worker polls its own token between targets.
            cancel(&state.stem_cancel);
        }
        AudioCommand::FreezeTrack {
            track_id,
            cache_path,
        } => {
            state.freeze_cancel = Some(bounce::to_freeze_cache_spawn(
                track_id,
                cache_path,
                Arc::clone(ctx.shared),
                Arc::clone(ctx.tempo_map),
                // Freeze bakes the track's plugin automation, like export
                // and bounce do (code review ENG-08).
                ctx.automation.load_full(),
                ctx.sample_rate,
                ctx.event_tx.clone(),
            ));
        }
        AudioCommand::CancelFreeze => {
            // The freeze render polls its own token between chunks, same
            // as the bounce renderers; a concurrently-running export can
            // neither consume this cancel nor clear it. The worker drops
            // the partial cache file and emits `FreezeCancelled`.
            cancel(&state.freeze_cancel);
        }
        _ => unreachable!("dispatch_bounce: unexpected command"),
    }
}
