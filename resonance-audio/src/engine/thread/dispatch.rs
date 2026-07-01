//! Per-category command dispatch sub-functions.
//!
//! [`dispatch`] is the single entry point called from the engine loop. It
//! routes each [`AudioCommand`] to the appropriate category handler below:
//!
//! | Sub-dispatcher          | Commands handled                              |
//! |-------------------------|-----------------------------------------------|
//! | [`dispatch_transport`]  | Play/Pause/Stop/Seek/BPM/loop                 |
//! | [`dispatch_clips`]      | Audio clips, warp, automation, project dir    |
//! | [`dispatch_tracks`]     | Track add/remove/volume/pan/mute/arm/freeze   |
//! | [`dispatch_plugins`]    | CLAP plugin add/remove/param/editor/state     |
//! | [`dispatch_bounce`]     | Bounce, export, stems, freeze                 |
//! | [`dispatch_midi`]       | MIDI clips, notes, live play, ext instruments |
//! | [`dispatch_busses`]     | Busses, aux sends, master FX chain            |
//! | [`dispatch_audition`]   | Audition preview                              |
//! | [`dispatch_midi_map`]   | MIDI learn & hardware controller mapping      |
//! | [`dispatch_reference`]  | Reference track A/B comparison               |

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::{HandlerCtx, HandlerState};
use crate::types::*;

use super::super::{
    audition, automation, bounce, bounce_realtime, busses, clips, external_instrument,
    external_instrument_ping, import_pool, master, midi, midi_map, plugins, reference, scan,
    tracks, transport, vocal_analysis,
};

// ---------------------------------------------------------------------------
// Top-level router
// ---------------------------------------------------------------------------

/// Route `cmd` to the appropriate category sub-dispatcher.
///
/// Every variant of [`AudioCommand`] is handled exactly once. `ShutDown` is
/// included for exhaustiveness but is unreachable here — the engine loop
/// breaks on it before calling this function.
pub(super) fn dispatch(ctx: &HandlerCtx, state: &mut HandlerState, cmd: AudioCommand) {
    match cmd {
        // Transport
        AudioCommand::Play
        | AudioCommand::Record { .. }
        | AudioCommand::Pause
        | AudioCommand::Stop
        | AudioCommand::SeekTo(..)
        | AudioCommand::SetBpm { .. }
        | AudioCommand::SetTempoEvents { .. }
        | AudioCommand::SetTimeSignature { .. }
        | AudioCommand::SetMetronomeEnabled { .. }
        | AudioCommand::SetLoopRange { .. }
        | AudioCommand::SetLoopRecordMode(..) => dispatch_transport(ctx, state, cmd),

        // Audio clips + automation + project
        AudioCommand::ImportClip { .. }
        | AudioCommand::ImportAudioToPool { .. }
        | AudioCommand::MoveClip { .. }
        | AudioCommand::TrimClip { .. }
        | AudioCommand::DeleteClip { .. }
        | AudioCommand::SetClipFade { .. }
        | AudioCommand::SetClipGain { .. }
        | AudioCommand::SetClipWarp { .. }
        | AudioCommand::SetClipWarpMarkers { .. }
        | AudioCommand::DetectClipTempo { .. }
        | AudioCommand::AnalyzeClipPitch { .. }
        | AudioCommand::SetAutomationLane { .. }
        | AudioCommand::ClearAutomationLane { .. }
        | AudioCommand::SetAutomationReadEnabled { .. }
        | AudioCommand::SetProjectDir(..)
        | AudioCommand::LoadClipFromWav { .. }
        | AudioCommand::SaveClipsToProjectDir => dispatch_clips(ctx, state, cmd),

        // Tracks
        AudioCommand::SetTrackVolume { .. }
        | AudioCommand::SetTrackPan { .. }
        | AudioCommand::SetTrackMute { .. }
        | AudioCommand::SetMasterVolume { .. }
        | AudioCommand::SetTrackSolo { .. }
        | AudioCommand::AddTrack { .. }
        | AudioCommand::CreateSubTrack { .. }
        | AudioCommand::RemoveTrack { .. }
        | AudioCommand::SetTrackRecordArm { .. }
        | AudioCommand::SetTrackMono { .. }
        | AudioCommand::SetTrackMonitor { .. }
        | AudioCommand::SetTrackInputDevice { .. }
        | AudioCommand::SetTrackInputPort { .. }
        | AudioCommand::ListInputDevices
        | AudioCommand::ClearAll
        | AudioCommand::SetTrackFrozenSource { .. }
        | AudioCommand::UnfreezeTrack { .. }
        | AudioCommand::SetTrackFxBypass { .. } => dispatch_tracks(ctx, state, cmd),

        // Plugins
        AudioCommand::AddPlugin { .. }
        | AudioCommand::RemovePlugin { .. }
        | AudioCommand::ScanPlugins
        | AudioCommand::SetPluginParam { .. }
        | AudioCommand::OpenPluginEditor { .. }
        | AudioCommand::ClosePluginEditor { .. }
        | AudioCommand::SavePluginState { .. }
        | AudioCommand::LoadPluginState { .. }
        | AudioCommand::SaveAllPluginStates => dispatch_plugins(ctx, state, cmd),

        // Bounce / export / freeze
        AudioCommand::BounceToWav { .. }
        | AudioCommand::ExportAudio { .. }
        | AudioCommand::BounceTrackToAudio { .. }
        | AudioCommand::BounceTrackRealtimeToAudio { .. }
        | AudioCommand::CancelBounce
        | AudioCommand::ExportStems { .. }
        | AudioCommand::CancelStemExport
        | AudioCommand::FreezeTrack { .. }
        | AudioCommand::CancelFreeze => dispatch_bounce(ctx, state, cmd),

        // MIDI clips, notes, live play, external instruments
        AudioCommand::AddInstrumentTrack { .. }
        | AudioCommand::AddVocalTrack { .. }
        | AudioCommand::CreateMidiClip { .. }
        | AudioCommand::LoadMidiClipDirect { .. }
        | AudioCommand::MoveMidiClip { .. }
        | AudioCommand::TrimMidiClip { .. }
        | AudioCommand::DeleteMidiClip { .. }
        | AudioCommand::AddMidiNote { .. }
        | AudioCommand::RemoveMidiNote { .. }
        | AudioCommand::MoveMidiNote { .. }
        | AudioCommand::ResizeMidiNote { .. }
        | AudioCommand::SetMidiNoteVelocity { .. }
        | AudioCommand::QuantizeMidiNotes { .. }
        | AudioCommand::HumanizeMidiNotes { .. }
        | AudioCommand::ApplyGrooveToClip { .. }
        | AudioCommand::ExtractGrooveFromClip { .. }
        | AudioCommand::SendNoteOn { .. }
        | AudioCommand::SendNoteOff { .. }
        | AudioCommand::ListMidiInputDevices
        | AudioCommand::ListMidiOutputDevices
        | AudioCommand::SetTrackMidiInput { .. }
        | AudioCommand::SetTrackMidiOutput { .. }
        | AudioCommand::SetExternalInstrument { .. }
        | AudioCommand::ClearExternalInstrument { .. }
        | AudioCommand::SetExternalInstrumentPatch { .. }
        | AudioCommand::SetExternalInstrumentLatencyOffset { .. }
        | AudioCommand::CheckExternalInstrumentDevices { .. }
        | AudioCommand::ResendExternalInstrumentPatches
        | AudioCommand::DetectExternalInstrumentLatency { .. }
        | AudioCommand::SetMidiClockOutput { .. }
        | AudioCommand::SetMidiClockInput { .. } => dispatch_midi(ctx, state, cmd),

        // Busses, aux sends, master FX chain
        AudioCommand::AddBus { .. }
        | AudioCommand::RemoveBus { .. }
        | AudioCommand::SetBusVolume { .. }
        | AudioCommand::SetBusPan { .. }
        | AudioCommand::SetBusMute { .. }
        | AudioCommand::SetBusName { .. }
        | AudioCommand::SetTrackOutput { .. }
        | AudioCommand::AddPluginToBus { .. }
        | AudioCommand::RemovePluginFromBus { .. }
        | AudioCommand::SetBusRole { .. }
        | AudioCommand::SetAuxSend { .. }
        | AudioCommand::RemoveAuxSend { .. }
        | AudioCommand::AddPluginToMaster { .. }
        | AudioCommand::RemovePluginFromMaster { .. }
        | AudioCommand::SetBusFxBypass { .. }
        | AudioCommand::SetMasterFxBypass { .. } => dispatch_busses(ctx, state, cmd),

        // Audition preview
        AudioCommand::AuditionFile { .. }
        | AudioCommand::StopAudition
        | AudioCommand::SetAuditionOptions { .. } => dispatch_audition(ctx, cmd),

        // MIDI Learn & hardware controller mapping
        AudioCommand::SetMidiBinding { .. }
        | AudioCommand::ClearMidiBinding { .. }
        | AudioCommand::SetControllerMap { .. }
        | AudioCommand::ClearAllMidiBindings
        | AudioCommand::SetControlSurfaceInput { .. }
        | AudioCommand::EnterMidiLearn { .. }
        | AudioCommand::CancelMidiLearn => dispatch_midi_map(ctx, cmd),

        // Reference track A/B
        AudioCommand::LoadReferenceTrack { .. }
        | AudioCommand::ReferenceAnalyzed { .. }
        | AudioCommand::RemoveReferenceTrack { .. }
        | AudioCommand::SetActiveReference { .. }
        | AudioCommand::SetABSource { .. }
        | AudioCommand::SetRefLoudnessMatch { .. }
        | AudioCommand::SetRefTrim { .. }
        | AudioCommand::AddRefMarker { .. }
        | AudioCommand::RemoveRefMarker { .. }
        | AudioCommand::SetRefPosition { .. }
        | AudioCommand::SetRefLoopToMix { .. }
        | AudioCommand::PollABMeters => dispatch_reference(ctx, state, cmd),

        AudioCommand::PollPeaks => handle_poll_peaks(ctx),

        AudioCommand::ShutDown => {
            // Handled in the engine_thread loop directly; this arm is
            // unreachable in practice but keeps the match exhaustive.
        }
    }
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

fn dispatch_transport(ctx: &HandlerCtx, state: &mut HandlerState, cmd: AudioCommand) {
    match cmd {
        AudioCommand::Play => transport::handle_play(ctx, state),
        AudioCommand::Record { precount_bars } => {
            transport::handle_record(ctx, state, precount_bars)
        }
        AudioCommand::Pause => transport::handle_pause(ctx, state),
        AudioCommand::Stop => transport::handle_stop(ctx, state),
        AudioCommand::SeekTo(pos) => transport::handle_seek_to(ctx, state, pos),
        AudioCommand::SetBpm { bpm } => transport::handle_set_bpm(ctx, bpm),
        AudioCommand::SetTempoEvents { tempo, signature } => {
            transport::handle_set_tempo_events(ctx, tempo, signature)
        }
        AudioCommand::SetTimeSignature {
            numerator,
            denominator,
        } => transport::handle_set_time_signature(ctx, numerator, denominator),
        AudioCommand::SetMetronomeEnabled { enabled } => {
            transport::handle_set_metronome_enabled(ctx, enabled)
        }
        AudioCommand::SetLoopRange {
            enabled,
            loop_in,
            loop_out,
        } => transport::handle_set_loop_range(ctx, state, enabled, loop_in, loop_out),
        AudioCommand::SetLoopRecordMode(on) => transport::handle_set_loop_record_mode(state, on),
        _ => unreachable!("dispatch_transport: unexpected command"),
    }
}

// ---------------------------------------------------------------------------
// Audio clips + automation + project directory
// ---------------------------------------------------------------------------

fn dispatch_clips(ctx: &HandlerCtx, state: &mut HandlerState, cmd: AudioCommand) {
    match cmd {
        AudioCommand::ImportClip {
            track_id,
            path,
            start_sample,
        } => clips::handle_import_clip(ctx, state, track_id, path, start_sample),
        AudioCommand::ImportAudioToPool { paths } => {
            import_pool::handle_import_audio_to_pool(ctx, state, paths)
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
            )
        }
        AudioCommand::ClearAutomationLane { target } => {
            automation::clear_automation_lane_in_place(
                &mut state.automation_lanes,
                ctx.event_tx,
                target,
            )
        }
        AudioCommand::SetAutomationReadEnabled { target, enabled } => {
            automation::set_automation_read_enabled_in_place(
                &mut state.automation_lanes,
                ctx.event_tx,
                target,
                enabled,
            )
        }
        AudioCommand::SetProjectDir(dir) => {
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
        _ => unreachable!("dispatch_clips: unexpected command"),
    }
}

// ---------------------------------------------------------------------------
// Tracks
// ---------------------------------------------------------------------------

fn dispatch_tracks(ctx: &HandlerCtx, state: &mut HandlerState, cmd: AudioCommand) {
    match cmd {
        AudioCommand::SetTrackVolume { track_id, volume } => {
            tracks::handle_set_track_volume(ctx, track_id, volume)
        }
        AudioCommand::SetTrackPan { track_id, pan } => {
            tracks::handle_set_track_pan(ctx, track_id, pan)
        }
        AudioCommand::SetTrackMute { track_id, muted } => {
            tracks::handle_set_track_mute(ctx, track_id, muted)
        }
        AudioCommand::SetMasterVolume { volume } => tracks::handle_set_master_volume(ctx, volume),
        AudioCommand::SetTrackSolo { track_id, soloed } => {
            tracks::handle_set_track_solo(ctx, track_id, soloed)
        }
        AudioCommand::AddTrack { id_hint, name } => {
            tracks::handle_add_track(ctx, state, id_hint, name)
        }
        AudioCommand::CreateSubTrack {
            sub_id,
            parent_track_id,
            output_port_index,
            name,
        } => tracks::handle_create_sub_track(
            ctx,
            state,
            sub_id,
            parent_track_id,
            output_port_index,
            name,
        ),
        AudioCommand::RemoveTrack { track_id } => {
            tracks::handle_remove_track(ctx, state, track_id)
        }
        AudioCommand::SetTrackRecordArm { track_id, armed } => {
            tracks::handle_set_track_record_arm(ctx, track_id, armed)
        }
        AudioCommand::SetTrackMono { track_id, mono } => {
            tracks::handle_set_track_mono(ctx, state, track_id, mono)
        }
        AudioCommand::SetTrackMonitor { track_id, enabled } => {
            tracks::handle_set_track_monitor(ctx, state, track_id, enabled)
        }
        AudioCommand::SetTrackInputDevice {
            track_id,
            device_name,
        } => tracks::handle_set_track_input_device(ctx, state, track_id, device_name),
        AudioCommand::SetTrackInputPort {
            track_id,
            port_index,
        } => tracks::handle_set_track_input_port(ctx, state, track_id, port_index),
        AudioCommand::ListInputDevices => tracks::handle_list_input_devices(ctx),
        AudioCommand::ClearAll => tracks::handle_clear_all(ctx, state),
        AudioCommand::SetTrackFrozenSource { track_id, source } => {
            tracks::handle_set_track_frozen_source(ctx, track_id, source)
        }
        AudioCommand::UnfreezeTrack { track_id } => {
            tracks::handle_unfreeze_track(ctx, track_id)
        }
        AudioCommand::SetTrackFxBypass { track_id, bypassed } => {
            tracks::handle_set_track_fx_bypass(ctx, track_id, bypassed)
        }
        _ => unreachable!("dispatch_tracks: unexpected command"),
    }
}

// ---------------------------------------------------------------------------
// Plugins
// ---------------------------------------------------------------------------

fn dispatch_plugins(ctx: &HandlerCtx, state: &mut HandlerState, cmd: AudioCommand) {
    match cmd {
        AudioCommand::AddPlugin {
            track_id,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        } => plugins::handle_add_plugin(
            ctx,
            state,
            track_id,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        ),
        AudioCommand::RemovePlugin {
            track_id,
            instance_id,
        } => plugins::handle_remove_plugin(ctx, track_id, instance_id),
        AudioCommand::ScanPlugins => {
            scan::scan_plugins(ctx.plugins, ctx.tracks, &mut state.bundles, ctx.event_tx)
        }
        AudioCommand::SetPluginParam {
            instance_id,
            param_id,
            value,
        } => plugins::handle_set_plugin_param(ctx, instance_id, param_id, value),
        AudioCommand::OpenPluginEditor { instance_id } => {
            plugins::handle_open_plugin_editor(ctx, instance_id)
        }
        AudioCommand::ClosePluginEditor { instance_id } => {
            plugins::handle_close_plugin_editor(ctx, instance_id)
        }
        AudioCommand::SavePluginState { instance_id } => {
            plugins::handle_save_plugin_state(ctx, instance_id)
        }
        AudioCommand::LoadPluginState { instance_id, data } => {
            plugins::handle_load_plugin_state(ctx, instance_id, data)
        }
        AudioCommand::SaveAllPluginStates => plugins::handle_save_all_plugin_states(ctx),
        _ => unreachable!("dispatch_plugins: unexpected command"),
    }
}

// ---------------------------------------------------------------------------
// Bounce / export / stems / freeze
// ---------------------------------------------------------------------------

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
        ctx.sample_rate,
        ctx.event_tx.clone(),
    );
}

fn dispatch_bounce(ctx: &HandlerCtx, state: &mut HandlerState, cmd: AudioCommand) {
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

// ---------------------------------------------------------------------------
// MIDI: clips, notes, live play, external instruments, clocks
// ---------------------------------------------------------------------------

fn dispatch_midi(ctx: &HandlerCtx, state: &mut HandlerState, cmd: AudioCommand) {
    match cmd {
        AudioCommand::AddInstrumentTrack { id_hint, name } => {
            midi::handle_add_instrument_track(ctx, state, id_hint, name)
        }
        AudioCommand::AddVocalTrack { id_hint, name } => {
            midi::handle_add_vocal_track(ctx, state, id_hint, name)
        }
        AudioCommand::CreateMidiClip {
            track_id,
            start_sample,
            duration_ticks,
            name,
        } => midi::handle_create_midi_clip(
            ctx,
            state,
            track_id,
            start_sample,
            duration_ticks,
            name,
        ),
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
        AudioCommand::SetExternalInstrument { config } => {
            external_instrument::set_external_instrument_in_place(
                &mut state.external_instruments,
                ctx.event_tx,
                config,
            )
        }
        AudioCommand::ClearExternalInstrument { track_id } => {
            external_instrument::clear_external_instrument_in_place(
                &mut state.external_instruments,
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

// ---------------------------------------------------------------------------
// Busses, aux sends, master FX chain + bypass
// ---------------------------------------------------------------------------

fn dispatch_busses(ctx: &HandlerCtx, state: &mut HandlerState, cmd: AudioCommand) {
    match cmd {
        AudioCommand::AddBus { id_hint, name } => {
            busses::handle_add_bus(ctx, state, id_hint, name)
        }
        AudioCommand::RemoveBus { bus_id } => busses::handle_remove_bus(ctx, bus_id),
        AudioCommand::SetBusVolume { bus_id, volume } => {
            busses::handle_set_bus_volume(ctx, bus_id, volume)
        }
        AudioCommand::SetBusPan { bus_id, pan } => busses::handle_set_bus_pan(ctx, bus_id, pan),
        AudioCommand::SetBusMute { bus_id, muted } => {
            busses::handle_set_bus_mute(ctx, bus_id, muted)
        }
        AudioCommand::SetBusName { bus_id, name } => {
            busses::handle_set_bus_name(ctx, bus_id, name)
        }
        AudioCommand::SetTrackOutput { track_id, output } => {
            busses::handle_set_track_output(ctx, track_id, output)
        }
        AudioCommand::AddPluginToBus {
            bus_id,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        } => busses::handle_add_plugin_to_bus(
            ctx,
            state,
            bus_id,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        ),
        AudioCommand::RemovePluginFromBus {
            bus_id,
            instance_id,
        } => busses::handle_remove_plugin_from_bus(ctx, bus_id, instance_id),
        AudioCommand::SetBusRole { bus_id, is_return } => {
            busses::handle_set_bus_role(ctx, bus_id, is_return)
        }
        AudioCommand::SetAuxSend {
            id_hint,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        } => busses::handle_set_aux_send(
            ctx, state, id_hint, source, dest, level_db, pre_fader, enabled,
        ),
        AudioCommand::RemoveAuxSend { send_id } => {
            busses::handle_remove_aux_send(ctx, state, send_id)
        }
        AudioCommand::AddPluginToMaster {
            clap_file_path,
            clap_plugin_id,
            id_hint,
        } => master::handle_add_plugin_to_master(
            ctx,
            state,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        ),
        AudioCommand::RemovePluginFromMaster { instance_id } => {
            master::handle_remove_plugin_from_master(ctx, instance_id)
        }
        AudioCommand::SetBusFxBypass { bus_id, bypassed } => {
            busses::handle_set_bus_fx_bypass(ctx, bus_id, bypassed)
        }
        AudioCommand::SetMasterFxBypass { bypassed } => {
            master::handle_set_master_fx_bypass(ctx, bypassed)
        }
        _ => unreachable!("dispatch_busses: unexpected command"),
    }
}

// ---------------------------------------------------------------------------
// Audition preview
// ---------------------------------------------------------------------------

fn dispatch_audition(ctx: &HandlerCtx, cmd: AudioCommand) {
    match cmd {
        AudioCommand::AuditionFile { path, start_frame } => {
            audition::handle_audition_file(ctx, path, start_frame)
        }
        AudioCommand::StopAudition => {
            if audition::stop_audition_in_place(ctx.shared) {
                let _ = ctx.event_tx.send(AudioEvent::AuditionStopped);
            }
        }
        AudioCommand::SetAuditionOptions {
            loop_enabled,
            sync_to_tempo,
        } => {
            let bpm = ctx.tempo_map.load().bpm as f64;
            audition::set_audition_options_in_place(ctx.shared, bpm, loop_enabled, sync_to_tempo);
        }
        _ => unreachable!("dispatch_audition: unexpected command"),
    }
}

// ---------------------------------------------------------------------------
// MIDI Learn & hardware controller mapping
// ---------------------------------------------------------------------------

fn dispatch_midi_map(ctx: &HandlerCtx, cmd: AudioCommand) {
    match cmd {
        AudioCommand::SetMidiBinding { binding } => {
            midi_map::handle_set_midi_binding(ctx, binding)
        }
        AudioCommand::ClearMidiBinding { id } => midi_map::handle_clear_midi_binding(ctx, id),
        AudioCommand::SetControllerMap { map } => midi_map::handle_set_controller_map(ctx, map),
        AudioCommand::ClearAllMidiBindings => midi_map::handle_clear_all_midi_bindings(ctx),
        AudioCommand::SetControlSurfaceInput { device } => {
            midi_map::handle_set_control_surface_input(ctx, device)
        }
        AudioCommand::EnterMidiLearn { target } => midi_map::handle_enter_midi_learn(ctx, target),
        AudioCommand::CancelMidiLearn => midi_map::handle_cancel_midi_learn(ctx),
        _ => unreachable!("dispatch_midi_map: unexpected command"),
    }
}

// ---------------------------------------------------------------------------
// Reference track A/B comparison
// ---------------------------------------------------------------------------

fn dispatch_reference(ctx: &HandlerCtx, state: &mut HandlerState, cmd: AudioCommand) {
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

// ---------------------------------------------------------------------------
// Peak metering
// ---------------------------------------------------------------------------

/// Snapshot and clear every peak meter (per-track, per-bus, master L/R)
/// and dispatch a `PeakSnapshot` event. Runs on the engine thread, so the
/// `try_read` calls compete only with the audio callback's brief
/// `try_read` — same window as the old direct getter but now off the GUI
/// thread, and the GUI side reads its result via the regular event queue.
fn handle_poll_peaks(ctx: &HandlerCtx) {
    let track_peaks = ctx
        .tracks
        .try_read()
        .map(|guard| {
            guard
                .values()
                .map(|t| (t.id, t.swap_peak_l(), t.swap_peak_r()))
                .collect()
        })
        .unwrap_or_default();
    let bus_peaks = ctx
        .busses
        .try_read()
        .map(|guard| {
            guard
                .values()
                .map(|b| (b.id, b.swap_peak_l(), b.swap_peak_r()))
                .collect()
        })
        .unwrap_or_default();
    let master_peak_l =
        f32::from_bits(ctx.shared.master_peak_l_bits.swap(0, Ordering::AcqRel));
    let master_peak_r =
        f32::from_bits(ctx.shared.master_peak_r_bits.swap(0, Ordering::AcqRel));
    let _ = ctx.event_tx.send(AudioEvent::PeakSnapshot {
        track_peaks,
        bus_peaks,
        master_peak_l,
        master_peak_r,
    });
}
