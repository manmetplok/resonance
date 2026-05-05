//! Engine → GUI event dispatch.
//!
//! Each `AudioEvent` variant is routed to a per-domain handler module.
//! The dispatch itself stays thin so it's easy to find which file owns
//! a given event. Helpers shared by multiple handlers (`finalize_bounce`,
//! `apply_preset_to_track`, `finish_preset_save`, `try_finish_save`)
//! live in this file because they cross domains.

mod clips;
mod midi;
mod plugins;
mod project_io;
mod tracks;
mod transport;

use iced::Task;
use resonance_audio::types::*;

use crate::message::*;
use crate::project;
use crate::state::*;
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

// ---------------------------------------------------------------------------
// Cross-domain helpers
// ---------------------------------------------------------------------------

/// Shared post-bounce wrap-up: mute the source, send the engine the
/// matching `SetTrackMute`, and reorder the bounce target so it sits
/// right under the source. Called from both the offline
/// (`TrackBouncedToAudio`) and realtime (`TrackBounceCompleted`)
/// completion handlers.
pub(super) fn finalize_bounce(
    r: &mut Resonance,
    source_track_id: TrackId,
    target_track_id: TrackId,
) {
    if let Some(track) = r
        .registry
        .tracks
        .iter_mut()
        .find(|t| t.id == source_track_id)
    {
        track.muted = true;
    }
    r.engine.send(AudioCommand::SetTrackMute {
        track_id: source_track_id,
        muted: true,
    });
    let source_order = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == source_track_id)
        .map(|t| t.order);
    if let Some(src_order) = source_order {
        // Bump every track at order > src_order by 1 so we can slot the
        // new track at src_order + 1.
        for t in r.registry.tracks.iter_mut() {
            if t.id != target_track_id && t.order > src_order {
                t.order += 1;
            }
        }
        if let Some(t) = r
            .registry
            .tracks
            .iter_mut()
            .find(|t| t.id == target_track_id)
        {
            t.order = src_order + 1;
        }
        r.registry.next_track_order += 1;
    }
}

/// Apply a preset's settings and plugin chain to a newly created track.
/// Called from the `TrackAdded`/`InstrumentTrackAdded` handlers when
/// `pending_track_preset` was set.
pub(super) fn apply_preset_to_track(
    r: &mut Resonance,
    track: &mut TrackState,
    preset: &crate::presets::TrackPreset,
) {
    track.name = preset.name.clone();
    track.volume = preset.volume;
    track.pan = preset.pan;
    track.mono = preset.mono;
    track.instrument_type = preset.instrument_type;
    track.instrument_icon = preset.instrument_icon;
    track.role = preset.role;

    let track_id = track.id;

    // Push mixer settings to the engine.
    r.engine.send(AudioCommand::SetTrackVolume {
        track_id,
        volume: crate::util::db_to_gain(preset.volume),
    });
    r.engine.send(AudioCommand::SetTrackPan {
        track_id,
        pan: preset.pan,
    });
    r.engine.send(AudioCommand::SetTrackMono {
        track_id,
        mono: preset.mono,
    });

    // Add preset plugins to the track.
    for pp in &preset.plugins {
        r.engine.send(AudioCommand::AddPlugin {
            track_id,
            clap_file_path: pp.clap_file_path.clone(),
            clap_plugin_id: pp.clap_plugin_id.clone(),
            id_hint: None,
        });
    }

    // Plugin state loading is deferred: we don't know the instance ids
    // yet (they're assigned by the engine). The PluginAdded event will
    // fire for each plugin. We store the preset plugin states so we can
    // match them up. For now, state loading for preset plugins relies
    // on the plugin states being sent after the AddPlugin command
    // returns via PluginAdded — we store them for deferred application.
    //
    // We stash the pending states in a simple list keyed by the order
    // (index) in the preset's plugin chain, so when PluginAdded fires
    // for this track we can pop the next state and apply it.
    if preset.plugins.iter().any(|p| p.state.is_some()) {
        let states: Vec<Option<Vec<u8>>> =
            preset.plugins.iter().map(|p| p.state.clone()).collect();
        r.pending_preset_plugin_states = Some((track_id, states));
    }
}

/// Complete a "Save track as preset" operation. Builds a `TrackPreset`
/// from the track's current state and the freshly-captured plugin
/// state blobs, then writes it to disk.
pub(super) fn finish_preset_save(r: &mut Resonance, track_id: TrackId) {
    let track = match r.registry.tracks.iter().find(|t| t.id == track_id) {
        Some(t) => t,
        None => return,
    };

    let plugins: Vec<crate::presets::PresetPlugin> = track
        .plugins
        .iter()
        .map(|p| crate::presets::PresetPlugin {
            plugin_name: p.plugin_name.clone(),
            clap_plugin_id: p.clap_plugin_id.clone(),
            clap_file_path: p.clap_file_path.clone(),
            state: r.plugin_state_cache.get(&p.instance_id).cloned(),
        })
        .collect();

    let preset = crate::presets::TrackPreset {
        name: track.name.clone(),
        track_type: match track.track_type {
            TrackType::Audio => "audio".to_string(),
            TrackType::Instrument => "instrument".to_string(),
        },
        volume: track.volume,
        pan: track.pan,
        mono: track.mono,
        instrument_type: track.instrument_type,
        instrument_icon: track.instrument_icon,
        role: track.role,
        plugins,
    };

    match crate::presets::save_user_preset(&preset) {
        Ok(_) => {
            r.user_presets = crate::presets::load_user_presets();
        }
        Err(e) => {
            r.error_message = Some(format!("Save preset: {e}"));
        }
    }
}

pub(super) fn try_finish_save(r: &mut Resonance) -> Task<Message> {
    let both_done = r
        .io
        .save_state
        .as_ref()
        .map(|s| s.clips_done && s.plugins_done)
        .unwrap_or(false);

    if !both_done {
        return Task::none();
    }

    let save = r.io.save_state.take().unwrap();
    let project_file = crate::update::build_project_file(r);
    let path = save.path.clone();
    let plugin_states = save.plugin_states;

    // Snapshot MIDI clips by id so the async save task can write them
    // as `.mid` files without touching `Resonance`.
    let midi_clips: Vec<(ClipId, Vec<MidiNote>)> = r
        .midi_clips
        .iter()
        .map(|mc| (mc.id, mc.notes.clone()))
        .collect();

    Task::perform(
        async move { project::save_project(&path, &project_file, &plugin_states, &midi_clips) },
        |r| Message::ProjectIo(ProjectIoMessage::ProjectSaved(r)),
    )
}
