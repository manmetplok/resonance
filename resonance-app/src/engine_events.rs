/// Engine event handling for the Resonance application.
use crate::message::Message;
use crate::project;
use crate::state::*;
use iced::Task;
use resonance_audio::types::*;

impl crate::Resonance {
    pub(crate) fn handle_engine_event(&mut self, event: AudioEvent) -> Task<Message> {
        match event {
            AudioEvent::PlayheadMoved(pos) => {
                self.transport.playhead = pos;
            }
            AudioEvent::SampleRateDetected { sample_rate } => {
                self.sample_rate = sample_rate;
            }
            AudioEvent::ClipImported {
                clip_id,
                track_id,
                start_sample,
                duration_samples,
                name,
                waveform_peaks,
            } => {
                // Idempotent: if the clip already exists (created by project load),
                // just update its waveform and total frames. Otherwise push new.
                if let Some(clip) = self.clips.iter_mut().find(|c| c.id == clip_id) {
                    clip.waveform_peaks = waveform_peaks;
                    clip.total_frames = duration_samples + clip.trim_start_frames + clip.trim_end_frames;
                } else {
                    self.clips.push(ClipState {
                        id: clip_id,
                        track_id,
                        start_sample,
                        duration_samples,
                        name,
                        total_frames: duration_samples,
                        trim_start_frames: 0,
                        trim_end_frames: 0,
                        waveform_peaks,
                    });
                }
            }
            AudioEvent::TrackAdded { track_id } => {
                // Idempotent: skip if the track already exists (created by project load).
                if self.registry.tracks.iter().any(|t| t.id == track_id) {
                    return Task::none();
                }
                let order = self.registry.next_track_order;
                self.registry.next_track_order += 1;
                self.registry.tracks.push(TrackState::new_audio(track_id, order));
            }
            AudioEvent::TrackRemoved { track_id } => {
                if let Some(sel_clip_id) = self.interaction.selected_clip {
                    if self.clips.iter().any(|c| c.id == sel_clip_id && c.track_id == track_id) {
                        self.interaction.selected_clip = None;
                    }
                }
                if let Some(sel_plugin_id) = self.mixer.selected_plugin {
                    if self.registry.tracks.iter()
                        .filter(|t| t.id == track_id)
                        .any(|t| t.plugins.iter().any(|p| p.instance_id == sel_plugin_id))
                    {
                        self.mixer.selected_plugin = None;
                    }
                }
                self.registry.tracks.retain(|t| t.id != track_id);
                self.clips.retain(|c| c.track_id != track_id);
                // Also drop any sub-tracks whose parent just went away.
                self.registry.tracks.retain(|t| {
                    t.sub_track
                        .map(|l| l.parent_track_id != track_id)
                        .unwrap_or(true)
                });
            }
            AudioEvent::ClipDeleted { clip_id } => {
                self.clips.retain(|c| c.id != clip_id);
            }
            AudioEvent::ClipMoved {
                clip_id,
                new_start_sample,
                new_track_id,
            } => {
                if let Some(clip) = self.clips.iter_mut().find(|c| c.id == clip_id) {
                    clip.start_sample = new_start_sample;
                    clip.track_id = new_track_id;
                }
            }
            AudioEvent::ClipTrimmed {
                clip_id,
                new_start_sample,
                new_duration_samples,
                trim_start_frames,
                trim_end_frames,
            } => {
                if let Some(clip) = self.clips.iter_mut().find(|c| c.id == clip_id) {
                    clip.start_sample = new_start_sample;
                    clip.duration_samples = new_duration_samples;
                    clip.trim_start_frames = trim_start_frames;
                    clip.trim_end_frames = trim_end_frames;
                }
            }
            AudioEvent::Stopped => {
                if !self.io.loading {
                    self.transport.playing = false;
                    self.transport.recording = false;
                    self.transport.playhead = 0;
                }
            }
            AudioEvent::Error(e) => {
                eprintln!("Audio engine error: {}", e);
                self.error_message = Some(e);
            }
            AudioEvent::InputDevicesListed {
                devices,
                default_name,
            } => {
                self.input_devices = devices;
                self.default_input_device_name = default_name;
            }
            AudioEvent::RecordingStarted { start_sample } => {
                self.transport.recording = true;
                self.transport.recording_start_sample = start_sample;
            }
            AudioEvent::RecordingFinished {
                clip_id,
                track_id,
                start_sample,
                duration_samples,
                name,
                waveform_peaks,
            } => {
                self.clips.push(ClipState {
                    id: clip_id,
                    track_id,
                    start_sample,
                    duration_samples,
                    name,
                    total_frames: duration_samples,
                    trim_start_frames: 0,
                    trim_end_frames: 0,
                    waveform_peaks,
                });
                self.transport.recording = false;
            }
            AudioEvent::PluginAdded {
                track_id,
                instance_id,
                plugin_name,
                clap_plugin_id,
                clap_file_path,
                params,
                has_gui,
                output_port_count,
                output_port_names,
            } => {
                // Idempotent: if the plugin slot already exists (created by project load),
                // just update its params and has_gui. Otherwise push a new slot.
                if let Some(track) = self.registry.tracks.iter_mut().find(|t| t.id == track_id) {
                    if let Some(slot) = track.plugins.iter_mut().find(|p| p.instance_id == instance_id) {
                        slot.params = params;
                        slot.has_gui = has_gui;
                    } else {
                        track.plugins.push(PluginSlotState::new(
                            instance_id,
                            plugin_name,
                            clap_plugin_id,
                            clap_file_path,
                            params,
                            has_gui,
                        ));
                    }
                }

                // Auto-create sub-tracks for multi-output plugins.
                // Skips ports already represented (so project load, which
                // replays saved sub-tracks before the PluginAdded event,
                // doesn't double up). Parent track's own existing name is
                // prefixed onto each sub-track ("Drums → Kick").
                if output_port_count > 1 {
                    let parent_name = self
                        .registry
                        .tracks
                        .iter()
                        .find(|t| t.id == track_id)
                        .map(|t| t.name.clone())
                        .unwrap_or_else(|| format!("Track {}", track_id));
                    for port_idx in 1..output_port_count {
                        let already = self.registry.tracks.iter().any(|t| {
                            t.sub_track
                                .map(|l| l.parent_track_id == track_id && l.output_port_index == port_idx as u32)
                                .unwrap_or(false)
                        });
                        if already {
                            continue;
                        }
                        let port_label = output_port_names
                            .get(port_idx)
                            .cloned()
                            .unwrap_or_else(|| format!("Port {}", port_idx));
                        let sub_id = self.registry.next_sub_track_id;
                        self.registry.next_sub_track_id += 1;
                        let order = self.registry.next_track_order;
                        self.registry.next_track_order += 1;
                        let sub_name = format!("{} \u{2192} {}", parent_name, port_label);
                        // Register the sub-track with the engine so its
                        // fader / pan / mute / bus routing atomics live
                        // alongside the parent track and the mixer's
                        // existing SetTrackVolume / SetTrackOutput / ...
                        // commands work unchanged.
                        self.engine.send(AudioCommand::CreateSubTrack {
                            sub_id,
                            parent_track_id: track_id,
                            output_port_index: port_idx as u32,
                            name: sub_name.clone(),
                        });
                        self.registry.tracks.push(TrackState::new_sub_track(
                            sub_id,
                            order,
                            sub_name,
                            track_id,
                            port_idx as u32,
                        ));
                    }
                }
            }
            AudioEvent::PluginRemoved {
                track_id,
                instance_id,
            } => {
                if self.mixer.selected_plugin == Some(instance_id) {
                    self.mixer.selected_plugin = None;
                }
                if let Some(track) = self.registry.tracks.iter_mut().find(|t| t.id == track_id) {
                    track.plugins.retain(|p| p.instance_id != instance_id);
                }
            }
            AudioEvent::PluginsScanned { plugins } => {
                self.available_plugins = plugins;
            }
            AudioEvent::BounceComplete { path } => {
                self.io.bouncing = false;
                eprintln!("Bounce complete: {path}");
            }
            AudioEvent::BounceError(e) => {
                self.io.bouncing = false;
                self.error_message = Some(format!("Bounce failed: {e}"));
            }
            AudioEvent::PluginStateSaved { instance_id: _, data: _ } => {
                // Used only by the project-save path (SaveCollector).
            }
            // --- Project save events ---
            AudioEvent::ClipDataExported { clip_id, data } => {
                if let Some(ref mut save) = self.io.save_state {
                    save.clip_data.insert(clip_id, data);
                }
            }
            AudioEvent::AllClipDataExported => {
                if let Some(ref mut save) = self.io.save_state {
                    save.clips_done = true;
                }
                return self.try_finish_save();
            }
            AudioEvent::AllPluginStatesSaved { states } => {
                if let Some(ref mut save) = self.io.save_state {
                    save.plugin_states = states;
                    save.plugins_done = true;
                }
                return self.try_finish_save();
            }
            // --- Project load events ---
            AudioEvent::AllCleared => {
                if let Some(loaded) = self.io.pending_load.take() {
                    // Extract project_path before replay (replay clears it)
                    let path = self.io.project_path.clone();
                    crate::update::replay_loaded_project(self, loaded);
                    self.io.project_path = path;
                    self.io.loading = false;
                }
            }

            // -- Instrument track events --
            AudioEvent::InstrumentTrackAdded { track_id } => {
                // Idempotent: skip if the track already exists (created by project load).
                if self.registry.tracks.iter().any(|t| t.id == track_id) {
                    return Task::none();
                }
                let order = self.registry.tracks.len();
                self.registry.tracks
                    .push(TrackState::new_instrument(track_id, order));
            }

            // -- MIDI clip events --
            AudioEvent::MidiClipCreated {
                clip_id, track_id, start_sample, duration_ticks,
                name, notes, trim_start_ticks, trim_end_ticks,
            } => {
                // Idempotent: skip if the MIDI clip already exists (created by project load).
                if self.midi_clips.iter().any(|c| c.id == clip_id) {
                    return Task::none();
                }
                self.midi_clips.push(MidiClipState {
                    id: clip_id,
                    track_id,
                    start_sample,
                    duration_ticks,
                    name,
                    notes,
                    trim_start_ticks,
                    trim_end_ticks,
                });
            }
            AudioEvent::MidiClipMoved { clip_id, new_start_sample, new_track_id } => {
                if let Some(clip) = self.midi_clips.iter_mut().find(|c| c.id == clip_id) {
                    clip.start_sample = new_start_sample;
                    clip.track_id = new_track_id;
                }
            }
            AudioEvent::MidiClipTrimmed { clip_id, new_start_sample, trim_start_ticks, trim_end_ticks } => {
                if let Some(clip) = self.midi_clips.iter_mut().find(|c| c.id == clip_id) {
                    clip.start_sample = new_start_sample;
                    clip.trim_start_ticks = trim_start_ticks;
                    clip.trim_end_ticks = trim_end_ticks;
                }
            }
            AudioEvent::MidiClipDeleted { clip_id } => {
                self.midi_clips.retain(|c| c.id != clip_id);
            }

            // -- MIDI note editing events --
            AudioEvent::MidiNoteAdded { clip_id, note } => {
                if let Some(clip) = self.midi_clips.iter_mut().find(|c| c.id == clip_id) {
                    let pos = clip.notes.partition_point(|n| n.start_tick <= note.start_tick);
                    clip.notes.insert(pos, note);
                }
            }
            AudioEvent::MidiNoteRemoved { clip_id, note_index } => {
                if let Some(clip) = self.midi_clips.iter_mut().find(|c| c.id == clip_id) {
                    if note_index < clip.notes.len() {
                        clip.notes.remove(note_index);
                    }
                }
            }
            AudioEvent::MidiNoteMoved { clip_id, note_index, new_start_tick, new_note } => {
                if let Some(clip) = self.midi_clips.iter_mut().find(|c| c.id == clip_id) {
                    if note_index < clip.notes.len() {
                        clip.notes[note_index].start_tick = new_start_tick;
                        clip.notes[note_index].note = new_note;
                        clip.notes.sort_by_key(|n| n.start_tick);
                    }
                }
            }
            AudioEvent::MidiNoteResized { clip_id, note_index, new_duration_ticks } => {
                if let Some(clip) = self.midi_clips.iter_mut().find(|c| c.id == clip_id) {
                    if note_index < clip.notes.len() {
                        clip.notes[note_index].duration_ticks = new_duration_ticks;
                    }
                }
            }
            AudioEvent::MidiNoteVelocitySet { clip_id, note_index, velocity } => {
                if let Some(clip) = self.midi_clips.iter_mut().find(|c| c.id == clip_id) {
                    if note_index < clip.notes.len() {
                        clip.notes[note_index].velocity = velocity;
                    }
                }
            }

            // -- Bus events --
            AudioEvent::BusAdded { bus_id, name } => {
                if self.registry.busses.iter().any(|b| b.id == bus_id) {
                    return Task::none();
                }
                let order = self.registry.next_bus_order;
                self.registry.next_bus_order += 1;
                self.registry.busses.push(BusState::new(bus_id, order, name));
            }
            AudioEvent::BusRemoved { bus_id } => {
                if let Some(sel) = self.mixer.selected_plugin {
                    if self.registry.busses.iter()
                        .filter(|b| b.id == bus_id)
                        .any(|b| b.plugins.iter().any(|p| p.instance_id == sel))
                    {
                        self.mixer.selected_plugin = None;
                    }
                }
                self.registry.busses.retain(|b| b.id != bus_id);
                // Any track that was routed to the removed bus falls back
                // to Master locally (the engine did the same server-side).
                for track in &mut self.registry.tracks {
                    if track.output == TrackOutput::Bus(bus_id) {
                        track.output = TrackOutput::Master;
                    }
                }
            }
            AudioEvent::BusPluginAdded {
                bus_id,
                instance_id,
                plugin_name,
                clap_plugin_id,
                clap_file_path,
                params,
                has_gui,
            } => {
                if let Some(bus) = self.registry.busses.iter_mut().find(|b| b.id == bus_id) {
                    if let Some(slot) =
                        bus.plugins.iter_mut().find(|p| p.instance_id == instance_id)
                    {
                        slot.params = params;
                        slot.has_gui = has_gui;
                    } else {
                        bus.plugins.push(PluginSlotState::new(
                            instance_id,
                            plugin_name,
                            clap_plugin_id,
                            clap_file_path,
                            params,
                            has_gui,
                        ));
                    }
                }
            }
            AudioEvent::BusPluginRemoved { bus_id, instance_id } => {
                if let Some(bus) = self.registry.busses.iter_mut().find(|b| b.id == bus_id) {
                    bus.plugins.retain(|p| p.instance_id != instance_id);
                }
                if self.mixer.selected_plugin == Some(instance_id) {
                    self.mixer.selected_plugin = None;
                }
            }
        }
        Task::none()
    }

    fn try_finish_save(&mut self) -> Task<Message> {
        let both_done = self.io.save_state.as_ref()
            .map(|s| s.clips_done && s.plugins_done)
            .unwrap_or(false);

        if !both_done {
            return Task::none();
        }

        let save = self.io.save_state.take().unwrap();
        let project_file = crate::update::build_project_file(self);
        let path = save.path.clone();
        let clip_data = save.clip_data;
        let plugin_states = save.plugin_states;

        Task::perform(
            async move {
                project::save_project(&path, &project_file, &clip_data, &plugin_states)
            },
            Message::ProjectSaved,
        )
    }
}
