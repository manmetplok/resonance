//! The engine-event dispatch itself: one `match` routing every
//! `AudioEvent` variant to its per-domain handler module. A free
//! function (not an `impl Resonance` method) per ARCHITECTURE.md's
//! update-handler pattern — `engine_events` was the last historical
//! `impl Resonance` exception.

use iced::Task;
use resonance_audio::types::*;

use crate::message::*;
use crate::Resonance;

use super::{
    automation, aux_sends, clips, freeze, midi, midi_map, plugins, pool, project_io, reference,
    takes, tracks, transport,
};

pub(crate) fn handle_engine_event(r: &mut Resonance, event: AudioEvent) -> Task<Message> {
    let task = route_engine_event(r, event);
    // A take that landed mid-drag closed the open gesture; re-open it on
    // the post-take state (FU-A2a).
    r.resume_split_gesture();
    task
}

fn route_engine_event(r: &mut Resonance, event: AudioEvent) -> Task<Message> {
    use AudioEvent as E;
    match event {
        // Transport / clock / device events
        E::PlayheadMoved(pos) => r.transport.playhead = pos,
        E::SampleRateDetected { sample_rate } => {
            r.sample_rate = sample_rate;
            // The tempo map's bar table is denominated in samples, so it
            // must be rebuilt at the detected rate — construction built it
            // at the 44.1k placeholder.
            r.rebuild_tempo_map();
        }
        E::Stopped => transport::stopped(r),
        E::TransportRefused => transport::refused(r),
        // The engine echoes a key route change back. It is the authority
        // on what is actually keyed, so reconcile the GUI mirror to the
        // echo rather than trusting the optimistic write the dispatching
        // handler made — and so a route the engine dropped on its own
        // (plugin or source removed) leaves the mirror too, instead of
        // surviving into the next save (ba todo #1311).
        E::SidechainRouteChanged {
            plugin,
            source,
            enabled,
        } => plugins::sidechain_route_changed(r, plugin, source, enabled),
        E::Error(e) => transport::error(r, e),
        E::InputDevicesListed { devices, default_name } => {
            transport::input_devices_listed(r, devices, default_name)
        }
        E::RecordingStarted { start_sample } => transport::recording_started(r, start_sample),
        E::RecordingOverflow { dropped_frames } => {
            transport::recording_overflow(r, dropped_frames)
        }
        // I/O latency report (doc #260 finding #13): no UI surface yet —
        // logged for diagnosability until a round-trip readout lands.
        E::IoLatencyReport {
            capture_samples,
            playback_samples,
            round_trip_samples,
        } => {
            tracing::info!(
                capture_samples,
                playback_samples,
                round_trip_samples,
                "audio: io latency (samples)"
            );
        }
        E::BounceComplete { path } => transport::bounce_complete(r, path),
        E::BounceError { kind, message } => transport::bounce_error(r, kind, message),
        E::TrackBounceError(e) => transport::track_bounce_error(r, e),
        E::TrackBounceCancelled { target_track_id } => {
            transport::track_bounce_cancelled(r, target_track_id)
        }
        E::BounceProgress { fraction } => {
            transport::bounce_progress(r, fraction)
        }
        // Stem-export plumbing (ba todo #325): the engine emits this
        // multi-target queue; wiring it into the export modal's progress
        // UI is a follow-up todo, so consume the events here for now.
        E::StemExportError(_)
        | E::StemExportProgress { .. }
        | E::StemExportTargetDone { .. }
        | E::StemExportTargetError { .. }
        | E::StemExportComplete { .. }
        | E::StemExportCancelled { .. } => {}
        // Mix measurement (ba doc #273, todos #1218 / #1219). These are
        // the terminal events of `AudioCommand::MeasureMix`, which only
        // the control API's `meter.*` issues; the GUI has no measurement
        // surface, so they resolve the control job named by `measure_id`
        // (ba todo #1243) and nothing else. A measurement nobody asked
        // for over the control socket is logged rather than dropped
        // silently.
        E::MixMeasured {
            measure_id,
            results,
        } => crate::update::control::mix_measured(r, measure_id, results),
        E::MixMeasureError {
            measure_id,
            message,
        } => crate::update::control::mix_measure_error(r, measure_id, message),
        E::ExportProgress { phase, fraction } => transport::export_progress(r, phase, fraction),
        E::ExportComplete { path, bytes, .. } => transport::export_complete(r, path, bytes),
        E::ExportError { kind, message } => transport::export_error(r, kind, message),
        E::MidiInputDevicesListed { devices } => transport::midi_input_devices(r, devices),
        E::MidiOutputDevicesListed { devices } => transport::midi_output_devices(r, devices),
        E::MidiClockStarted => transport::midi_clock_started(r),
        E::MidiClockContinued => transport::midi_clock_continued(r),
        E::MidiClockStopped => transport::midi_clock_stopped(r),
        E::MidiClockTempoDetected { bpm } => transport::midi_clock_tempo_detected(r, bpm),
        // Confirms the engine stored a track's device-param map
        // (`SetTrackDeviceParams`, epic #40, doc #201 §4). Mirror the applied
        // param ids onto the track's external-instrument state so the app can
        // confirm the dispatch and reconstruct after a project-load replay.
        E::TrackDeviceParamsApplied { track_id, param_ids } => {
            super::external_instrument::device_params_applied(r, track_id, param_ids)
        }

        // Audio clip events
        E::ClipImported {
            clip_id,
            track_id,
            start_sample,
            duration_samples,
            name,
            waveform_peaks,
        } => clips::imported(
            r,
            clip_id,
            track_id,
            start_sample,
            duration_samples,
            name,
            waveform_peaks,
        ),
        E::ClipDeleted { clip_id } => clips::deleted_echo(r, clip_id),
        E::ClipMoved {
            clip_id,
            new_start_sample,
            new_track_id,
        } => clips::moved(r, clip_id, new_start_sample, new_track_id),
        E::ClipTrimmed {
            clip_id,
            new_start_sample,
            new_duration_samples,
            trim_start_frames,
            trim_end_frames,
        } => clips::trimmed(
            r,
            clip_id,
            new_start_sample,
            new_duration_samples,
            trim_start_frames,
            trim_end_frames,
        ),
        // Clip fade/gain mirroring (todo #316): one-way engine→app sync of
        // the engine-clamped fade/gain values into the matching `ClipState`.
        E::ClipFadeChanged {
            clip_id,
            fade_in_frames,
            fade_in_curve,
            fade_out_frames,
            fade_out_curve,
        } => clips::fade_changed(
            r,
            clip_id,
            fade_in_frames,
            fade_in_curve,
            fade_out_frames,
            fade_out_curve,
        ),
        E::ClipGainChanged { clip_id, gain_db } => clips::gain_changed(r, clip_id, gain_db),
        // Clip warp / follow-tempo events (engine todo #418). Mirroring
        // these into `ClipState` is todo #421; until it lands these arms
        // accept the events without acting, keeping the workspace
        // compiling now that the engine emits them.
        E::ClipWarpChanged { .. } | E::ClipWarpMarkersChanged { .. } => {}
        // Clip tempo/BPM detection reply (engine todo #420). The detector
        // emits this so the command/event boundary is complete; mirroring
        // the detected BPM into the app is a follow-up todo, so accept it
        // without acting for now.
        E::ClipTempoDetected { .. } => {}
        // Media-pool import lifecycle (engine todo #592). `AssetImported`
        // mirrors the asset into the pool and, for a drop, places it as a
        // clip (todo #598, `engine_events::pool`); `ImportFailed` drops the
        // queued placement and surfaces the error. `ImportProgress` updates
        // the per-file progress tracker for the transcode modal (todo #597).
        E::ImportProgress { asset_id, path, stage } => {
            pool::import_progress(r, asset_id, path, stage)
        }
        E::AssetImported {
            asset_id,
            project_relative_path,
            original_path,
            format,
            channels,
            source_sample_rate,
            duration_frames,
            peaks,
        } => pool::asset_imported(
            r,
            asset_id,
            project_relative_path,
            original_path,
            format,
            channels,
            source_sample_rate,
            duration_frames,
            peaks,
        ),
        E::ImportFailed {
            asset_id,
            path,
            reason,
        } => pool::import_failed(r, asset_id, path, reason),
        // Vocal pitch analysis (todo #357) emits the detected contour/notes
        // here; mirror them into the clip's app-side `VocalTuning` (todo
        // #359) so the pitch editor reads them without a read-back.
        E::ClipPitchDetected {
            clip_id,
            notes,
            contour,
        } => clips::pitch_detected(r, clip_id, notes, contour),
        E::RecordingFinished {
            clip_id,
            track_id,
            start_sample,
            duration_samples,
            name,
            waveform_peaks,
        } => clips::recording_finished(
            r,
            clip_id,
            track_id,
            start_sample,
            duration_samples,
            name,
            waveform_peaks,
        ),
        // Cycle-record take capture (epic #15). The engine emits one
        // `TakeCaptured` per loop pass with its take-group/slot id. This
        // is the *only* news the app gets about a take: no
        // `RecordingFinished` is emitted for a take clip, so the clip never
        // reaches `Resonance::clips` and `extent` is the app's sole account
        // of what the pass actually recorded (todo #1396).
        E::TakeCaptured {
            group_id,
            take_id,
            track_id,
            slot,
            pass_index,
            extent,
            content,
        } => takes::take_captured(
            r, group_id, take_id, track_id, slot, pass_index, extent, content,
        ),
        // Comp / active-take echoes (epic #15, todo #411). The engine has
        // already applied these to what it plays and bounces, so the
        // mirror adopts them verbatim — including when they merely confirm
        // the optimistic update an update handler already made.
        E::TakeCompChanged { group_id, segments } => takes::comp_changed(r, group_id, segments),
        E::ActiveTakeChanged { group_id, take_id } => {
            takes::active_take_changed(r, group_id, take_id)
        }
        // Removal echoes (ba todo #1397). Same shape: the engine has
        // already dropped the take (or the whole lane) from what it plays,
        // and the re-covered comp follows as a `TakeCompChanged`.
        E::TakeRemoved { group_id, take_id } => takes::take_removed(r, group_id, take_id),
        E::TakeGroupRemoved { group_id } => takes::take_group_removed(r, group_id),

        // MIDI clip + note events
        E::MidiClipCreated {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            name,
            notes,
            trim_start_ticks,
            trim_end_ticks,
        } => midi::clip_created(
            r,
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            name,
            notes,
            trim_start_ticks,
            trim_end_ticks,
        ),
        E::MidiClipMoved {
            clip_id,
            new_start_sample,
            new_track_id,
        } => midi::clip_moved(r, clip_id, new_start_sample, new_track_id),
        E::MidiClipTrimmed {
            clip_id,
            new_start_sample,
            trim_start_ticks,
            trim_end_ticks,
        } => midi::clip_trimmed(
            r,
            clip_id,
            new_start_sample,
            trim_start_ticks,
            trim_end_ticks,
        ),
        E::MidiClipDeleted { clip_id } => midi::clip_deleted_echo(r, clip_id),
        E::MidiNoteAdded { clip_id, note } => midi::note_added(r, clip_id, note),
        E::MidiNoteRemoved {
            clip_id,
            note_index,
        } => midi::note_removed(r, clip_id, note_index),
        E::MidiNoteMoved {
            clip_id,
            note_index,
            new_start_tick,
            new_note,
        } => midi::note_moved(r, clip_id, note_index, new_start_tick, new_note),
        E::MidiNoteResized {
            clip_id,
            note_index,
            new_duration_ticks,
        } => midi::note_resized(r, clip_id, note_index, new_duration_ticks),
        E::MidiNoteVelocitySet {
            clip_id,
            note_index,
            velocity,
        } => midi::note_velocity_set(r, clip_id, note_index, velocity),

        // Bulk MIDI edits from quantize/humanize/groove ops (doc #163, epic #25).
        // The engine emits one `MidiNotesEdited` carrying the full resulting note
        // array (replacing the clip's notes wholesale, no per-note churn), and
        // `GrooveExtracted` for groove extraction (added to the app groove library).
        E::MidiNotesEdited { clip_id, notes } => midi::notes_edited(r, clip_id, notes),
        E::GrooveExtracted { template } => midi::groove_extracted(r, template),

        // MIDI Learn & hardware control-surface mapping (doc #167 §3 A1).
        // App state is a pure projection of these events; the active
        // binding set is rebuilt from MidiBindingChanged / Cleared alone.
        E::MidiLearnCaptured { target, source } => midi_map::learn_captured(r, target, source),
        E::MidiBindingChanged { binding } => midi_map::binding_changed(r, binding),
        E::MidiBindingCleared { id } => midi_map::binding_cleared(r, id),
        E::ControlSurfaceParamChanged { target, value_norm } => {
            midi_map::param_changed(r, target, value_norm)
        }
        E::ControlSurfaceDevicesChanged { inputs } => midi_map::devices_changed(r, inputs),

        // Track / bus lifecycle
        E::TrackAdded { track_id } => tracks::added(r, track_id),
        E::InstrumentTrackAdded { track_id } => tracks::instrument_added(r, track_id),
        E::VocalTrackAdded { track_id } => tracks::vocal_added(r, track_id),
        E::TrackRemoved { track_id } => tracks::removed_echo(r, track_id),
        E::TrackBounceCompleted {
            source_track_id,
            target_track_id,
            clip,
        } => tracks::bounce_completed(r, source_track_id, target_track_id, clip),
        E::TrackFxBypassChanged { track_id, bypassed } => {
            tracks::fx_bypass_changed(r, track_id, bypassed)
        }
        // External-instrument playback source echo (doc #257): mirror
        // the engine-owned mode into the track state, whether it came
        // from the inspector toggle or the auto-switch after a take.
        E::TrackPlaybackSourceChanged { track_id, source } => {
            tracks::playback_source_changed(r, track_id, source)
        }
        E::BusAdded { bus_id, name } => tracks::bus_added(r, bus_id, name),
        E::BusRemoved { bus_id } => tracks::bus_removed(r, bus_id),
        E::BusFxBypassChanged { bus_id, bypassed } => {
            tracks::bus_fx_bypass_changed(r, bus_id, bypassed)
        }

        // Aux send / return-bus events. Mirrored into app state purely
        // from these events (todo #478) — the engine-side data model,
        // commands, and cyclic-route validation landed in todo #475. The
        // mixer view that surfaces sends/returns is a separate follow-up.
        E::BusRoleChanged { bus_id, is_return } => {
            aux_sends::bus_role_changed(r, bus_id, is_return)
        }
        E::AuxSendChanged {
            send_id,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        } => aux_sends::send_changed(r, send_id, source, dest, level_db, pre_fader, enabled),
        E::AuxSendRemoved { send_id } => aux_sends::send_removed(r, send_id),
        E::AuxSendRejected {
            source,
            dest,
            reason,
        } => aux_sends::send_rejected(r, source, dest, reason),

        // Plugin lifecycle
        E::PluginAdded {
            track_id,
            instance_id,
            plugin_name,
            clap_plugin_id,
            clap_file_path,
            params,
            has_gui,
            has_sidechain_input,
            output_port_count,
            output_port_names,
        } => plugins::track_added(
            r,
            track_id,
            instance_id,
            plugin_name,
            clap_plugin_id,
            clap_file_path,
            params,
            has_gui,
            has_sidechain_input,
            output_port_count,
            output_port_names,
        ),
        E::PluginRemoved {
            track_id,
            instance_id,
        } => plugins::track_removed_echo(r, track_id, instance_id),
        E::PluginMoved {
            track_id,
            instance_id,
            to_index,
        } => plugins::track_moved(r, track_id, instance_id, to_index),
        E::PluginsScanned { plugins } => plugins::scanned(r, plugins),
        E::PluginLoadFailed {
            instance_id,
            clap_plugin_id,
            clap_file_path,
            reason,
        } => plugins::load_failed(r, instance_id, clap_plugin_id, clap_file_path, reason),
        E::PluginParamText {
            instance_id,
            param_id,
            value,
            text,
        } => plugins::param_text(r, instance_id, param_id, value, text),
        E::PluginScanFailed { failures } => plugins::scan_failed(r, failures),
        E::PluginStateSaved { instance_id, data } => {
            plugins::state_saved(r, instance_id, data)
        }
        // The engine's real editor open/failed/closed report (ba doc
        // #283, ba todo #1347). This is the ONLY thing that moves
        // `PluginSlotState.editor_open`; `update/plugin.rs` no longer
        // sets it optimistically.
        E::PluginEditorState {
            instance_id,
            open,
            failure,
        } => plugins::editor_state(r, instance_id, open, failure),
        E::BusPluginAdded {
            bus_id,
            instance_id,
            plugin_name,
            clap_plugin_id,
            clap_file_path,
            params,
            has_gui,
            has_sidechain_input,
        } => plugins::bus_added(
            r,
            bus_id,
            instance_id,
            plugin_name,
            clap_plugin_id,
            clap_file_path,
            params,
            has_gui,
            has_sidechain_input,
        ),
        E::BusPluginRemoved {
            bus_id,
            instance_id,
        } => plugins::bus_removed_echo(r, bus_id, instance_id),
        E::BusPluginMoved {
            bus_id,
            instance_id,
            to_index,
        } => plugins::bus_moved(r, bus_id, instance_id, to_index),
        E::MasterPluginAdded {
            instance_id,
            plugin_name,
            clap_plugin_id,
            clap_file_path,
            params,
            has_gui,
            has_sidechain_input,
        } => plugins::master_added(
            r,
            instance_id,
            plugin_name,
            clap_plugin_id,
            clap_file_path,
            params,
            has_gui,
            has_sidechain_input,
        ),
        E::MasterPluginRemoved { instance_id } => plugins::master_removed_echo(r, instance_id),
        E::MasterPluginMoved {
            instance_id,
            to_index,
        } => plugins::master_moved(r, instance_id, to_index),
        E::MasterFxBypassChanged { bypassed } => {
            plugins::master_fx_bypass_changed(r, bypassed)
        }
        // Per-slot bypass echo (ba doc #275 finding X3). The engine half
        // (todo #1304) is complete and reachable over
        // `AudioCommand::SetPluginBypass`; the GUI toggle, project
        // persistence and `track.set_fx_bypass` control method are todo
        // #1305, which is where this event gets mirrored into app state.
        // Consumed for exhaustiveness until then.
        E::PluginBypassChanged {
            instance_id,
            bypassed,
            own_bypass_param,
        } => plugins::bypass_changed(r, instance_id, bypassed, own_bypass_param),

        // Peak meter snapshot — drive the VU decay+update from the
        // engine's view of the world. See `update::tick`.
        E::PeakSnapshot {
            track_peaks,
            bus_peaks,
            master_peak_l,
            master_peak_r,
        } => crate::update::tick::apply_peak_snapshot(
            r,
            track_peaks,
            bus_peaks,
            master_peak_l,
            master_peak_r,
        ),

        // Audition preview events: mirror the engine's playhead position into
        // the browser's scrub bar, and clear the playing row when the engine
        // naturally stops (end of a non-looping file, or after StopAudition).
        // Both are transient UI state — not undoable, not persisted (doc #175,
        // ba todo #597).
        E::AuditionPosition { frame } => {
            r.media.browser.audition.position_frame = frame;
        }
        E::AuditionStopped => {
            r.media.browser.audition.playing = None;
            r.media.browser.audition.position_frame = 0;
        }
        // Freeze progress / lifecycle (ba todo #575). The engine renders
        // off-thread (todo #571/#572) and reports back through these
        // events; the mirror folds them into per-track freeze status and
        // advances the batch queue.
        E::FreezeProgress { track_id, fraction } => freeze::progress(r, track_id, fraction),
        E::FreezeCompleted { track_id, cache_ref } => {
            freeze::completed(r, track_id, cache_ref)
        }
        E::FreezeError { track_id, message } => freeze::error(r, track_id, message),
        E::FreezeCancelled { track_id } => freeze::cancelled(r, track_id),

        // Project save / load — these return a Task<Message>.
        E::ClipsSavedToProjectDir { clip_files } => {
            return project_io::clips_saved(r, clip_files)
        }
        E::AllPluginStatesSaved { states } => {
            return project_io::all_plugin_states_saved(r, states)
        }
        E::AllCleared => return project_io::all_cleared(r),

        // Automation lanes (doc #162 §3, todo #378): one-way engine→app
        // mirror of lane state into `AutomationState`, plus the throttled
        // live automated value into the transient live-value map.
        E::AutomationLaneChanged { lane } => automation::lane_changed(r, lane),
        E::AutomationLaneCleared { target } => automation::lane_cleared(r, target),
        E::AutomatedValue { target, value_norm } => {
            automation::automated_value(r, target, value_norm)
        }

        // Reference-track (A/B) events fold into `Resonance::reference`.
        E::ReferenceAnalysisProgress { id, stage } => reference::analysis_progress(r, id, stage),
        E::ReferenceLoaded {
            id,
            name,
            path,
            integrated_lufs,
            waveform_peaks,
            length_samples,
        } => reference::loaded(r, id, name, path, integrated_lufs, waveform_peaks, length_samples),
        E::ReferenceLoadFailed { path, reason } => reference::load_failed(r, path, reason),
        E::ReferenceRemoved { id } => reference::removed(r, id),
        E::ActiveReferenceChanged { id } => reference::active_changed(r, id),
        E::ABSourceChanged { source } => reference::ab_source_changed(r, source),
        E::RefLoudnessMatchChanged { enabled, offset_db } => {
            reference::loudness_match_changed(r, enabled, offset_db)
        }
        E::RefTrimChanged { db } => reference::trim_changed(r, db),
        E::RefMarkerAdded {
            ref_id,
            marker_id,
            position_samples,
            label,
        } => reference::marker_added(r, ref_id, marker_id, position_samples, label),
        E::RefMarkerRemoved { ref_id, marker_id } => {
            reference::marker_removed(r, ref_id, marker_id)
        }
        E::RefPositionChanged {
            ref_id,
            position_samples,
        } => reference::position_changed(r, ref_id, position_samples),
        E::RefLoopToMixChanged { enabled } => reference::loop_to_mix_changed(r, enabled),
        E::ABMeterSnapshot { mix, reference: ref_meter } => {
            reference::ab_meter_snapshot(r, mix, ref_meter)
        }

        // External-instrument config + device-offline events: mirror the
        // engine's stored config and device status into the app's
        // `external_instruments` map (doc #169, epic #39).
        E::ExternalInstrumentChanged { config } => {
            super::external_instrument::changed(r, config)
        }
        E::ExternalInstrumentCleared { track_id } => {
            super::external_instrument::cleared(r, track_id)
        }
        E::ExternalInstrumentMidiOutOffline { track_id, .. } => {
            super::external_instrument::midi_out_offline(r, track_id)
        }
        E::ExternalInstrumentReturnInputOffline { track_id, .. } => {
            super::external_instrument::return_input_offline(r, track_id)
        }
        E::ExternalInstrumentLatencyMeasured {
            track_id,
            latency_samples,
            ..
        } => super::external_instrument::latency_measured(r, track_id, latency_samples),
        E::ExternalInstrumentLatencyDetectFailed { track_id, reason } => {
            super::external_instrument::latency_detect_failed(r, track_id, reason)
        }
    }
    Task::none()
}
