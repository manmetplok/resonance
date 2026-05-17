//! Engine → GUI event dispatch.
//!
//! Each `AudioEvent` variant is routed to a per-domain handler module.
//! The dispatch itself stays thin so it's easy to find which file owns
//! a given event.

mod clips;
mod midi;
mod plugins;
mod presets;
mod project_io;
mod tracks;
mod transport;

use iced::Task;
use resonance_audio::types::*;

use crate::message::*;
use crate::Resonance;

impl Resonance {
    pub(crate) fn handle_engine_event(&mut self, event: AudioEvent) -> Task<Message> {
        use AudioEvent as E;
        match event {
            // Transport / clock / device events
            E::PlayheadMoved(pos) => self.transport.playhead = pos,
            E::SampleRateDetected { sample_rate } => self.sample_rate = sample_rate,
            E::Stopped => transport::stopped(self),
            E::Error(e) => transport::error(self, e),
            E::InputDevicesListed { devices, default_name } => {
                transport::input_devices_listed(self, devices, default_name)
            }
            E::RecordingStarted { start_sample } => transport::recording_started(self, start_sample),
            E::BounceComplete { path } => transport::bounce_complete(self, path),
            E::BounceError(e) => transport::bounce_error(self, e),
            E::TrackBounceError(e) => transport::track_bounce_error(self, e),
            E::TrackBounceCancelled { target_track_id } => {
                transport::track_bounce_cancelled(self, target_track_id)
            }
            E::BounceProgress { fraction } => {
                transport::bounce_progress(self, fraction)
            }
            E::MidiInputDevicesListed { devices } => transport::midi_input_devices(self, devices),
            E::MidiOutputDevicesListed { devices } => transport::midi_output_devices(self, devices),
            E::MidiClockStarted => transport::midi_clock_started(self),
            E::MidiClockContinued => transport::midi_clock_continued(self),
            E::MidiClockStopped => transport::midi_clock_stopped(self),
            E::MidiClockTempoDetected { bpm } => transport::midi_clock_tempo_detected(self, bpm),

            // Audio clip events
            E::ClipImported {
                clip_id,
                track_id,
                start_sample,
                duration_samples,
                name,
                waveform_peaks,
            } => clips::imported(
                self,
                clip_id,
                track_id,
                start_sample,
                duration_samples,
                name,
                waveform_peaks,
            ),
            E::ClipDeleted { clip_id } => clips::deleted(self, clip_id),
            E::ClipMoved {
                clip_id,
                new_start_sample,
                new_track_id,
            } => clips::moved(self, clip_id, new_start_sample, new_track_id),
            E::ClipTrimmed {
                clip_id,
                new_start_sample,
                new_duration_samples,
                trim_start_frames,
                trim_end_frames,
            } => clips::trimmed(
                self,
                clip_id,
                new_start_sample,
                new_duration_samples,
                trim_start_frames,
                trim_end_frames,
            ),
            E::RecordingFinished {
                clip_id,
                track_id,
                start_sample,
                duration_samples,
                name,
                waveform_peaks,
            } => clips::recording_finished(
                self,
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
                self,
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
            } => midi::clip_moved(self, clip_id, new_start_sample, new_track_id),
            E::MidiClipTrimmed {
                clip_id,
                new_start_sample,
                trim_start_ticks,
                trim_end_ticks,
            } => midi::clip_trimmed(
                self,
                clip_id,
                new_start_sample,
                trim_start_ticks,
                trim_end_ticks,
            ),
            E::MidiClipDeleted { clip_id } => midi::clip_deleted(self, clip_id),
            E::MidiNoteAdded { clip_id, note } => midi::note_added(self, clip_id, note),
            E::MidiNoteRemoved {
                clip_id,
                note_index,
            } => midi::note_removed(self, clip_id, note_index),
            E::MidiNoteMoved {
                clip_id,
                note_index,
                new_start_tick,
                new_note,
            } => midi::note_moved(self, clip_id, note_index, new_start_tick, new_note),
            E::MidiNoteResized {
                clip_id,
                note_index,
                new_duration_ticks,
            } => midi::note_resized(self, clip_id, note_index, new_duration_ticks),
            E::MidiNoteVelocitySet {
                clip_id,
                note_index,
                velocity,
            } => midi::note_velocity_set(self, clip_id, note_index, velocity),

            // Track / bus lifecycle
            E::TrackAdded { track_id } => tracks::added(self, track_id),
            E::InstrumentTrackAdded { track_id } => tracks::instrument_added(self, track_id),
            E::VocalTrackAdded { track_id } => tracks::vocal_added(self, track_id),
            E::TrackRemoved { track_id } => tracks::removed(self, track_id),
            E::TrackBounceCompleted {
                source_track_id,
                target_track_id,
                clip,
            } => tracks::bounce_completed(self, source_track_id, target_track_id, clip),
            E::TrackFxBypassChanged { track_id, bypassed } => {
                tracks::fx_bypass_changed(self, track_id, bypassed)
            }
            E::BusAdded { bus_id, name } => tracks::bus_added(self, bus_id, name),
            E::BusRemoved { bus_id } => tracks::bus_removed(self, bus_id),
            E::BusFxBypassChanged { bus_id, bypassed } => {
                tracks::bus_fx_bypass_changed(self, bus_id, bypassed)
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
                self,
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
            } => plugins::track_removed(self, track_id, instance_id),
            E::PluginsScanned { plugins } => plugins::scanned(self, plugins),
            E::PluginStateSaved { instance_id, data } => {
                plugins::state_saved(self, instance_id, data)
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
                self,
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
            } => plugins::bus_removed(self, bus_id, instance_id),
            E::MasterPluginAdded {
                instance_id,
                plugin_name,
                clap_plugin_id,
                clap_file_path,
                params,
                has_gui,
            } => plugins::master_added(
                self,
                instance_id,
                plugin_name,
                clap_plugin_id,
                clap_file_path,
                params,
                has_gui,
            ),
            E::MasterPluginRemoved { instance_id } => plugins::master_removed(self, instance_id),
            E::MasterFxBypassChanged { bypassed } => {
                plugins::master_fx_bypass_changed(self, bypassed)
            }

            // Peak meter snapshot — drive the VU decay+update from the
            // engine's view of the world. See `update::viewport`.
            E::PeakSnapshot {
                track_peaks,
                bus_peaks,
                master_peak_l,
                master_peak_r,
            } => crate::update::viewport::apply_peak_snapshot(
                self,
                track_peaks,
                bus_peaks,
                master_peak_l,
                master_peak_r,
            ),

            // Project save / load — these return a Task<Message>.
            E::ClipsSavedToProjectDir { clip_files } => {
                return project_io::clips_saved(self, clip_files)
            }
            E::AllPluginStatesSaved { states } => {
                return project_io::all_plugin_states_saved(self, states)
            }
            E::AllCleared => project_io::all_cleared(self),
        }
        Task::none()
    }
}
