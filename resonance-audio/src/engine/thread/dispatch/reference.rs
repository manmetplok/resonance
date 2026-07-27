//! Reference track A/B comparison command dispatch.

use crate::types::*;

use super::super::{HandlerCtx, HandlerState};
use super::super::super::reference;

pub(super) fn dispatch_reference(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    cmd: AudioCommand,
) {
    match cmd {
        AudioCommand::LoadReferenceTrack { id_hint, path } => {
            reference::handle_load_reference_track(
                &mut state.reference,
                ctx.event_tx,
                ctx.cmd_tx_retry,
                ctx.sample_rate,
                id_hint,
                path,
            )
        }
        AudioCommand::ReferenceAnalyzed {
            id,
            pcm,
            integrated_lufs,
        } => {
            reference::handle_reference_analyzed(&mut state.reference, id, pcm, integrated_lufs);
            // Decoded PCM just arrived: republish (cursor synced so the
            // active reference starts from the top of its decoded buffer).
            state.reference.publish(&ctx.shared.reference, true);
        }
        AudioCommand::RemoveReferenceTrack { id } => {
            reference::handle_remove_reference_track(&mut state.reference, ctx.event_tx, id);
            // May have cleared the active selection — drop the monitor PCM.
            state.reference.publish(&ctx.shared.reference, false);
        }
        AudioCommand::SetActiveReference { id } => {
            reference::handle_set_active_reference(&mut state.reference, ctx.event_tx, id);
            // New active reference: swap PCM + restart from its cursor.
            state.reference.publish(&ctx.shared.reference, true);
        }
        AudioCommand::SetABSource { source } => {
            reference::handle_set_ab_source(&mut state.reference, ctx.event_tx, source);
            state.reference.publish(&ctx.shared.reference, false);
        }
        AudioCommand::SetRefLoudnessMatch { enabled } => {
            reference::handle_set_ref_loudness_match(
                &mut state.reference,
                ctx.event_tx,
                enabled,
            );
            state.reference.publish(&ctx.shared.reference, false);
        }
        AudioCommand::SetRefTrim { db } => {
            reference::handle_set_ref_trim(&mut state.reference, ctx.event_tx, db);
            state.reference.publish(&ctx.shared.reference, false);
        }
        AudioCommand::AddRefMarker {
            ref_id,
            position_samples,
            label,
        } => reference::handle_add_ref_marker(
            &mut state.reference,
            ctx.event_tx,
            ref_id,
            position_samples,
            label,
        ),
        AudioCommand::RemoveRefMarker { ref_id, marker_id } => {
            reference::handle_remove_ref_marker(
                &mut state.reference,
                ctx.event_tx,
                ref_id,
                marker_id,
            )
        }
        AudioCommand::SetRefPosition {
            ref_id,
            position_samples,
        } => {
            reference::handle_set_ref_position(
                &mut state.reference,
                ctx.event_tx,
                ref_id,
                position_samples,
            );
            // Explicit scrub: re-sync the live cursor to the new position.
            state.reference.publish(&ctx.shared.reference, true);
        }
        AudioCommand::SetRefLoopToMix { enabled } => {
            reference::handle_set_ref_loop_to_mix(&mut state.reference, ctx.event_tx, enabled);
            state.reference.publish(&ctx.shared.reference, false);
        }
        AudioCommand::PollABMeters => reference::handle_poll_ab_meters(
            &state.reference,
            ctx.shared.mix_meter.load(),
            ctx.shared.ref_meter.load(),
            ctx.event_tx,
        ),
        _ => unreachable!("dispatch_reference: unexpected command"),
    }
}
