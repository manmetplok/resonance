//! The engine-event dispatch itself: one `match` routing every
//! `AudioEvent` variant to its per-domain handler module. A free
//! function (not an `impl Resonance` method) per ARCHITECTURE.md's
//! update-handler pattern — `engine_events` was the last historical
//! `impl Resonance` exception.

use iced::Task;
use resonance_audio::types::*;

use crate::message::*;
use crate::Resonance;

use super::{aux_sends, clips, midi, midi_map, plugins, project_io, reference, tracks, transport};

pub(crate) fn handle_engine_event(r: &mut Resonance, event: AudioEvent) -> Task<Message> {
    use AudioEvent as E;
    match event {
        // Transport / clock / device events
        E::PlayheadMoved(pos) => r.transport.playhead = pos,
        E::SampleRateDetected { sample_rate } => r.sample_rate = sample_rate,
        E::Stopped => transport::stopped(r),
        E::Error(e) => transport::error(r, e),
        E::InputDevicesListed { devices, default_name } => {
            transport::input_devices_listed(r, devices, default_name)
        }
        E::RecordingStarted { start_sample } => transport::recording_started(r, start_sample),
        E::BounceComplete { path } => transport::bounce_complete(r, path),
        E::BounceError(e) => transport::bounce_error(r, e),
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
        E::MidiInputDevicesListed { devices } => transport::midi_input_devices(r, devices),
        E::MidiOutputDevicesListed { devices } => transport::midi_output_devices(r, devices),
        E::MidiClockStarted => transport::midi_clock_started(r),
        E::MidiClockContinued => transport::midi_clock_continued(r),
        E::MidiClockStopped => transport::midi_clock_stopped(r),
        E::MidiClockTempoDetected { bpm } => transport::midi_clock_tempo_detected(r, bpm),

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
        E::ClipDeleted { clip_id } => clips::deleted(r, clip_id),
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
        // Media-pool import lifecycle (engine todo #592). Mirroring these
        // into the app's pool + import-progress state is todo #597; until
        // it lands these arms accept the events without acting, keeping
        // the workspace compiling now that the engine emits them.
        E::ImportProgress { .. } | E::AssetImported { .. } | E::ImportFailed { .. } => {}
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
        // `TakeCaptured` per loop pass with its take-group/slot id; the
        // recorded clips themselves arrive via `RecordingFinished`. GUI
        // take-lane mirroring/comping is a follow-up todo, so for now we
        // accept the event without acting — keeping the match exhaustive.
        E::TakeCaptured { .. } => {}

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
        E::MidiClipDeleted { clip_id } => midi::clip_deleted(r, clip_id),
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
        E::TrackRemoved { track_id } => tracks::removed(r, track_id),
        E::TrackBounceCompleted {
            source_track_id,
            target_track_id,
            clip,
        } => tracks::bounce_completed(r, source_track_id, target_track_id, clip),
        E::TrackFxBypassChanged { track_id, bypassed } => {
            tracks::fx_bypass_changed(r, track_id, bypassed)
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
            output_port_count,
            output_port_names,
        ),
        E::PluginRemoved {
            track_id,
            instance_id,
        } => plugins::track_removed(r, track_id, instance_id),
        E::PluginsScanned { plugins } => plugins::scanned(r, plugins),
        E::PluginStateSaved { instance_id, data } => {
            plugins::state_saved(r, instance_id, data)
        }
        E::BusPluginAdded {
            bus_id,
            instance_id,
            plugin_name,
            clap_plugin_id,
            clap_file_path,
            params,
            has_gui,
        } => plugins::bus_added(
            r,
            bus_id,
            instance_id,
            plugin_name,
            clap_plugin_id,
            clap_file_path,
            params,
            has_gui,
        ),
        E::BusPluginRemoved {
            bus_id,
            instance_id,
        } => plugins::bus_removed(r, bus_id, instance_id),
        E::MasterPluginAdded {
            instance_id,
            plugin_name,
            clap_plugin_id,
            clap_file_path,
            params,
            has_gui,
        } => plugins::master_added(
            r,
            instance_id,
            plugin_name,
            clap_plugin_id,
            clap_file_path,
            params,
            has_gui,
        ),
        E::MasterPluginRemoved { instance_id } => plugins::master_removed(r, instance_id),
        E::MasterFxBypassChanged { bypassed } => {
            plugins::master_fx_bypass_changed(r, bypassed)
        }

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

        // Audition preview position/stopped events round-trip through the
        // engine, but the app does not hold audition UI state yet (the scrub
        // playhead + browser preview controls land with the audition app-state
        // todo, doc #175), so there is nothing to mirror — no-ops for now.
        E::AuditionPosition { .. } | E::AuditionStopped => {}

        // Project save / load — these return a Task<Message>.
        E::ClipsSavedToProjectDir { clip_files } => {
            return project_io::clips_saved(r, clip_files)
        }
        E::AllPluginStatesSaved { states } => {
            return project_io::all_plugin_states_saved(r, states)
        }
        E::AllCleared => project_io::all_cleared(r),

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
    }
    Task::none()
}
