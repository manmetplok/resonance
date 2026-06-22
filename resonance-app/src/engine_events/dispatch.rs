//! The engine-event dispatch itself: one `match` routing every
//! `AudioEvent` variant to its per-domain handler module. A free
//! function (not an `impl Resonance` method) per ARCHITECTURE.md's
//! update-handler pattern — `engine_events` was the last historical
//! `impl Resonance` exception.

use iced::Task;
use resonance_audio::types::*;

use crate::message::*;
use crate::Resonance;

use super::{clips, midi, plugins, project_io, tracks, transport};

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
        // Media-pool import lifecycle (engine todo #592). Mirroring these
        // into the app's pool + import-progress state is todo #597; until
        // it lands these arms accept the events without acting, keeping
        // the workspace compiling now that the engine emits them.
        E::ImportProgress { .. } | E::AssetImported { .. } | E::ImportFailed { .. } => {}
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
    }
    Task::none()
}
