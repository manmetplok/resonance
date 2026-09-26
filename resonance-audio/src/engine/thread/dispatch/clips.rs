//! Audio-clip, warp, automation, and project-directory command dispatch.

use crate::types::*;

use super::super::{publish_automation_snapshot, HandlerCtx, HandlerState};
use super::super::super::{automation, clips, import_pool, vocal_analysis};

pub(super) fn dispatch_clips(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    cmd: AudioCommand,
) {
    match cmd {
        AudioCommand::ImportClip {
            track_id,
            path,
            start_sample,
        } => clips::handle_import_clip(ctx, state, track_id, path, start_sample),
        AudioCommand::ImportAudioToPool { files } => {
            import_pool::handle_import_audio_to_pool(ctx, state, files)
        }
        // Every edit that names an existing clip goes through the
        // load race first (ba doc #276 BUG 1): the clip may still be
        // on the import worker, in which case the command is parked
        // and replayed rather than silently dropped.
        AudioCommand::MoveClip { clip_id, .. }
        | AudioCommand::TrimClip { clip_id, .. }
        | AudioCommand::DeleteClip { clip_id }
        | AudioCommand::SplitClip { clip_id, .. }
        | AudioCommand::SetClipFade { clip_id, .. }
        | AudioCommand::SetClipGain { clip_id, .. }
            if !clips::clip_exists(ctx, clip_id) =>
        {
            clips::defer_clip_command(state, clip_id, cmd)
        }
        AudioCommand::MoveClip {
            clip_id,
            new_start_sample,
            new_track_id,
        } => clips::handle_move_clip(ctx, clip_id, new_start_sample, new_track_id),
        AudioCommand::TrimClip {
            clip_id,
            new_start_sample,
            trim_start_frames,
            trim_end_frames,
        } => clips::handle_trim_clip(
            ctx,
            clip_id,
            new_start_sample,
            trim_start_frames,
            trim_end_frames,
        ),
        AudioCommand::DeleteClip { clip_id } => clips::handle_delete_clip(ctx, clip_id),
        AudioCommand::SplitClip {
            clip_id,
            new_clip_id,
            at_sample,
        } => clips::handle_split_clip(ctx, clip_id, new_clip_id, at_sample),
        AudioCommand::SetClipFade {
            clip_id,
            fade_in_frames,
            fade_in_curve,
            fade_out_frames,
            fade_out_curve,
        } => clips::handle_set_clip_fade(
            ctx,
            clip_id,
            fade_in_frames,
            fade_in_curve,
            fade_out_frames,
            fade_out_curve,
        ),
        AudioCommand::SetClipGain { clip_id, gain_db } => {
            clips::handle_set_clip_gain(ctx, clip_id, gain_db)
        }
        AudioCommand::SetClipWarp {
            clip_id,
            warp_enabled,
            original_bpm,
            transpose_semitones,
            warp_algorithm,
        } => clips::handle_set_clip_warp(
            ctx,
            clip_id,
            warp_enabled,
            original_bpm,
            transpose_semitones,
            warp_algorithm,
        ),
        AudioCommand::SetClipWarpMarkers { clip_id, markers } => {
            clips::handle_set_clip_warp_markers(ctx, clip_id, markers)
        }
        AudioCommand::DetectClipTempo { clip_id } => {
            clips::handle_detect_clip_tempo(ctx, clip_id)
        }
        AudioCommand::AnalyzeClipPitch { clip_id } => {
            vocal_analysis::handle_analyze_clip_pitch(ctx, state, clip_id)
        }
        AudioCommand::SetAutomationLane { lane } => {
            automation::set_automation_lane_in_place(
                &mut state.automation_lanes,
                ctx.event_tx,
                lane,
            );
            publish_automation_snapshot(ctx, &state.automation_lanes);
        }
        AudioCommand::ClearAutomationLane { target } => {
            automation::clear_automation_lane_in_place(
                &mut state.automation_lanes,
                ctx.event_tx,
                target,
            );
            publish_automation_snapshot(ctx, &state.automation_lanes);
        }
        AudioCommand::SetAutomationReadEnabled { target, enabled } => {
            automation::set_automation_read_enabled_in_place(
                &mut state.automation_lanes,
                ctx.event_tx,
                target,
                enabled,
            );
            publish_automation_snapshot(ctx, &state.automation_lanes);
        }
        AudioCommand::SetProjectDir(dir) => {
            clips::start_clip_id_scan(state, &dir);
            state.project_dir = Some(dir);
        }
        AudioCommand::LoadClipFromWav {
            clip_id,
            track_id,
            start_sample,
            path,
            name,
            trim_start_frames,
            trim_end_frames,
        } => clips::handle_load_clip_from_wav(
            ctx,
            state,
            clip_id,
            track_id,
            start_sample,
            path,
            name,
            trim_start_frames,
            trim_end_frames,
        ),
        AudioCommand::SaveClipsToProjectDir => {
            clips::handle_save_clips_to_project_dir(ctx, state)
        }
        AudioCommand::PersistClipWavs => clips::handle_persist_clip_wavs(ctx, state),
        _ => unreachable!("dispatch_clips: unexpected command"),
    }
}
