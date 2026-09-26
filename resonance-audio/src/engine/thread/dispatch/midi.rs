//! MIDI command dispatch: clips, notes, live play, external instruments, clocks.

use crate::types::*;

use super::super::{HandlerCtx, HandlerState};
use super::super::super::{external_instrument, external_instrument_ping, midi};

pub(super) fn dispatch_midi(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    cmd: AudioCommand,
) {
    match cmd {
        AudioCommand::AddInstrumentTrack { id, name } => {
            midi::handle_add_instrument_track(ctx, id, name);
            // The app enables external mode on an id it allocated itself,
            // so `SetExternalInstrument` can arrive before (or after) the
            // track exists. Re-assert the stored modes now that it does.
            external_instrument::mark_external_tracks(
                &state.external_instruments,
                &ctx.tracks.read(),
            );
        }
        AudioCommand::AddVocalTrack { id, name } => midi::handle_add_vocal_track(ctx, id, name),
        AudioCommand::CreateMidiClip {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            name,
        } => midi::handle_create_midi_clip(ctx, clip_id, track_id, start_sample, duration_ticks, name),
        AudioCommand::LoadMidiClipDirect {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            notes,
            name,
            trim_start_ticks,
            trim_end_ticks,
        } => midi::handle_load_midi_clip_direct(
            ctx,
            state,
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            notes,
            name,
            trim_start_ticks,
            trim_end_ticks,
        ),
        AudioCommand::MoveMidiClip {
            clip_id,
            new_start_sample,
            new_track_id,
        } => midi::handle_move_midi_clip(ctx, clip_id, new_start_sample, new_track_id),
        AudioCommand::TrimMidiClip {
            clip_id,
            new_start_sample,
            trim_start_ticks,
            trim_end_ticks,
        } => midi::handle_trim_midi_clip(
            ctx,
            clip_id,
            new_start_sample,
            trim_start_ticks,
            trim_end_ticks,
        ),
        AudioCommand::DeleteMidiClip { clip_id } => midi::handle_delete_midi_clip(ctx, clip_id),
        AudioCommand::AddMidiNote { clip_id, note } => {
            midi::handle_add_midi_note(ctx, clip_id, note)
        }
        AudioCommand::RemoveMidiNote {
            clip_id,
            note_index,
        } => midi::handle_remove_midi_note(ctx, clip_id, note_index),
        AudioCommand::MoveMidiNote {
            clip_id,
            note_index,
            new_start_tick,
            new_note,
        } => midi::handle_move_midi_note(ctx, clip_id, note_index, new_start_tick, new_note),
        AudioCommand::ResizeMidiNote {
            clip_id,
            note_index,
            new_duration_ticks,
        } => midi::handle_resize_midi_note(ctx, clip_id, note_index, new_duration_ticks),
        AudioCommand::SetMidiNoteVelocity {
            clip_id,
            note_index,
            velocity,
        } => midi::handle_set_midi_note_velocity(ctx, clip_id, note_index, velocity),
        AudioCommand::SetMidiClipNotes { clip_id, notes } => {
            midi::handle_set_midi_clip_notes(ctx, clip_id, notes)
        }
        AudioCommand::QuantizeMidiNotes {
            clip_id,
            indices,
            grid,
            strength,
            swing,
            mode,
            quantize_ends,
            iterative,
        } => midi::handle_quantize_midi_notes(
            ctx,
            clip_id,
            indices,
            grid,
            strength,
            swing,
            mode,
            quantize_ends,
            iterative,
        ),
        AudioCommand::HumanizeMidiNotes {
            clip_id,
            indices,
            timing_ticks,
            vel_amt,
            seed,
        } => midi::handle_humanize_midi_notes(ctx, clip_id, indices, timing_ticks, vel_amt, seed),
        AudioCommand::ApplyGrooveToClip {
            clip_id,
            indices,
            template,
            strength,
        } => midi::handle_apply_groove_to_clip(ctx, clip_id, indices, template, strength),
        AudioCommand::ExtractGrooveFromClip { clip_id, grid } => {
            midi::handle_extract_groove_from_clip(ctx, clip_id, grid)
        }
        // GUI-originated notes carry no arrival timestamp; offset 0
        // (start of the next block) is the earliest delivery anyway.
        AudioCommand::SendNoteOn {
            track_id,
            note,
            velocity,
        } => midi::handle_send_note_on(ctx, state, track_id, note, velocity, 0),
        AudioCommand::SendNoteOff { track_id, note } => {
            midi::handle_send_note_off(ctx, state, track_id, note, 0)
        }
        AudioCommand::ListMidiInputDevices => midi::handle_list_midi_inputs(ctx, state),
        AudioCommand::ListMidiOutputDevices => midi::handle_list_midi_outputs(ctx, state),
        AudioCommand::SetTrackMidiInput {
            track_id,
            device,
            channel,
        } => midi::handle_set_track_midi_input(ctx, state, track_id, device, channel),
        AudioCommand::SetTrackMidiOutput {
            track_id,
            device,
            channel,
        } => midi::handle_set_track_midi_output(ctx, state, track_id, device, channel),
        AudioCommand::SetTrackDeviceParams { track_id, params } => {
            midi::handle_set_track_device_params(ctx, track_id, params)
        }
        AudioCommand::SetExternalInstrument { config } => {
            external_instrument::set_external_instrument_in_place(
                &mut state.external_instruments,
                &ctx.tracks.read(),
                ctx.event_tx,
                config,
            )
        }
        AudioCommand::ClearExternalInstrument { track_id } => {
            external_instrument::clear_external_instrument_in_place(
                &mut state.external_instruments,
                &ctx.tracks.read(),
                ctx.event_tx,
                track_id,
            )
        }
        AudioCommand::SetExternalInstrumentPatch {
            track_id,
            bank,
            program,
        } => external_instrument::handle_set_patch(ctx, state, track_id, bank, program),
        AudioCommand::SetExternalInstrumentLatencyOffset {
            track_id,
            latency_offset_samples,
        } => external_instrument::set_external_instrument_latency_in_place(
            &mut state.external_instruments,
            ctx.event_tx,
            track_id,
            latency_offset_samples,
        ),
        AudioCommand::CheckExternalInstrumentDevices { track_id } => {
            external_instrument::handle_check_devices(ctx, state, track_id)
        }
        AudioCommand::ResendExternalInstrumentPatches => {
            external_instrument::handle_resend_patches(ctx, state)
        }
        AudioCommand::DetectExternalInstrumentLatency { track_id } => {
            external_instrument_ping::handle_detect_latency(ctx, state, track_id)
        }
        AudioCommand::SetMidiClockOutput { device, enabled } => {
            midi::handle_set_midi_clock_output(ctx, state, device, enabled)
        }
        AudioCommand::SetMidiClockInput { device, enabled } => {
            midi::handle_set_midi_clock_input(ctx, state, device, enabled)
        }
        _ => unreachable!("dispatch_midi: unexpected command"),
    }
}
