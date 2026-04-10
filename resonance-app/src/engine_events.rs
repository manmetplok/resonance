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
                self.playhead = pos;
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
                if self.tracks.iter().any(|t| t.id == track_id) {
                    return Task::none();
                }
                let order = self.next_track_order;
                self.next_track_order += 1;
                self.tracks.push(TrackState {
                    id: track_id,
                    name: format!("Track {}", track_id),
                    volume: 0.0,
                    pan: 0.0,
                    muted: false,
                    soloed: false,
                    order,
                    record_armed: false,
                    monitor_enabled: false,
                    mono: true,
                    input_device_name: None,
                    plugins: Vec::new(),
                    level_l: 0.0,
                    level_r: 0.0,
                    track_type: TrackType::Audio,
                    output: TrackOutput::Master,
                    instrument_type: InstrumentType::Synth,
                    instrument_icon: InstrumentIcon::Music,
                    sub_track: None,
                    input_port_index: 0,
                });
            }
            AudioEvent::TrackRemoved { track_id } => {
                if let Some(sel_clip_id) = self.selected_clip {
                    if self.clips.iter().any(|c| c.id == sel_clip_id && c.track_id == track_id) {
                        self.selected_clip = None;
                    }
                }
                if let Some(sel_plugin_id) = self.selected_plugin {
                    if self.tracks.iter()
                        .filter(|t| t.id == track_id)
                        .any(|t| t.plugins.iter().any(|p| p.instance_id == sel_plugin_id))
                    {
                        self.selected_plugin = None;
                    }
                }
                self.tracks.retain(|t| t.id != track_id);
                self.clips.retain(|c| c.track_id != track_id);
                // Also drop any sub-tracks whose parent just went away.
                self.tracks.retain(|t| {
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
                if !self.loading {
                    self.playing = false;
                    self.recording = false;
                    self.playhead = 0;
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
                self.recording = true;
                self.recording_start_sample = start_sample;
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
                self.recording = false;
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
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
                    if let Some(slot) = track.plugins.iter_mut().find(|p| p.instance_id == instance_id) {
                        slot.params = params;
                        slot.has_gui = has_gui;
                    } else {
                        let custom = match clap_plugin_id.as_str() {
                            "com.resonance.amp" => PluginCustomState::Amp(Default::default()),
                            "com.resonance.ir" => PluginCustomState::Ir(Default::default()),
                            _ => PluginCustomState::Generic,
                        };
                        track.plugins.push(PluginSlotState {
                            instance_id,
                            plugin_name,
                            clap_plugin_id,
                            clap_file_path,
                            params,
                            custom,
                            has_gui,
                            editor_open: false,
                        });
                    }
                }

                // Auto-create sub-tracks for multi-output plugins.
                // Skips ports already represented (so project load, which
                // replays saved sub-tracks before the PluginAdded event,
                // doesn't double up). Parent track's own existing name is
                // prefixed onto each sub-track ("Drums → Kick").
                if output_port_count > 1 {
                    let parent_name = self
                        .tracks
                        .iter()
                        .find(|t| t.id == track_id)
                        .map(|t| t.name.clone())
                        .unwrap_or_else(|| format!("Track {}", track_id));
                    for port_idx in 1..output_port_count {
                        let already = self.tracks.iter().any(|t| {
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
                        let sub_id = self.next_sub_track_id;
                        self.next_sub_track_id += 1;
                        let order = self.next_track_order;
                        self.next_track_order += 1;
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
                        self.tracks.push(TrackState {
                            id: sub_id,
                            name: sub_name,
                            volume: 0.0,
                            pan: 0.0,
                            muted: false,
                            soloed: false,
                            order,
                            record_armed: false,
                            monitor_enabled: false,
                            mono: false,
                            input_device_name: None,
                            plugins: Vec::new(),
                            level_l: 0.0,
                            level_r: 0.0,
                            track_type: TrackType::Instrument,
                            output: TrackOutput::Master,
                            instrument_type: InstrumentType::Synth,
                            instrument_icon: InstrumentIcon::Music,
                            sub_track: Some(crate::state::SubTrackLink {
                                parent_track_id: track_id,
                                output_port_index: port_idx as u32,
                            }),
                            input_port_index: 0,
                        });
                    }
                }
            }
            AudioEvent::PluginRemoved {
                track_id,
                instance_id,
            } => {
                if self.selected_plugin == Some(instance_id) {
                    self.selected_plugin = None;
                }
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
                    track.plugins.retain(|p| p.instance_id != instance_id);
                }
            }
            AudioEvent::PluginsScanned { plugins } => {
                self.available_plugins = plugins;
            }
            AudioEvent::BounceComplete { path } => {
                self.bouncing = false;
                eprintln!("Bounce complete: {path}");
            }
            AudioEvent::BounceError(e) => {
                self.bouncing = false;
                self.error_message = Some(format!("Bounce failed: {e}"));
            }
            AudioEvent::PluginStateSaved { instance_id, data } => {
                // If we have a pending path to inject, modify the state and reload.
                if let Some((pending_id, ref key, ref path)) = self.pending_plugin_path.clone() {
                    if pending_id == instance_id {
                        if let Ok(mut state) =
                            serde_json::from_slice::<serde_json::Value>(&data)
                        {
                            state[&key] = serde_json::Value::String(path.clone());
                            if let Ok(new_data) = serde_json::to_vec(&state) {
                                self.engine.send(AudioCommand::LoadPluginState {
                                    instance_id,
                                    data: new_data,
                                });
                            }
                        }
                        self.pending_plugin_path = None;
                    }
                }
            }
            // --- Project save events ---
            AudioEvent::ClipDataExported { clip_id, data } => {
                if let Some(ref mut save) = self.save_state {
                    save.clip_data.insert(clip_id, data);
                }
            }
            AudioEvent::AllClipDataExported => {
                if let Some(ref mut save) = self.save_state {
                    save.clips_done = true;
                }
                return self.try_finish_save();
            }
            AudioEvent::AllPluginStatesSaved { states } => {
                if let Some(ref mut save) = self.save_state {
                    save.plugin_states = states;
                    save.plugins_done = true;
                }
                return self.try_finish_save();
            }
            // --- Project load events ---
            AudioEvent::AllCleared => {
                if let Some(loaded) = self.pending_load.take() {
                    // Extract project_path before replay (replay clears it)
                    let path = self.project_path.clone();
                    self.replay_loaded_project(loaded);
                    self.project_path = path;
                    self.loading = false;
                }
            }

            // -- Instrument track events --
            AudioEvent::InstrumentTrackAdded { track_id } => {
                // Idempotent: skip if the track already exists (created by project load).
                if self.tracks.iter().any(|t| t.id == track_id) {
                    return Task::none();
                }
                let order = self.tracks.len();
                self.tracks.push(TrackState {
                    id: track_id,
                    name: format!("Instrument {}", track_id),
                    volume: 1.0,
                    pan: 0.0,
                    muted: false,
                    soloed: false,
                    order,
                    record_armed: false,
                    monitor_enabled: false,
                    mono: false,
                    input_device_name: None,
                    plugins: Vec::new(),
                    level_l: 0.0,
                    level_r: 0.0,
                    track_type: TrackType::Instrument,
                    output: TrackOutput::Master,
                    instrument_type: InstrumentType::Synth,
                    instrument_icon: InstrumentIcon::Music,
                    sub_track: None,
                    input_port_index: 0,
                });
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
                if self.busses.iter().any(|b| b.id == bus_id) {
                    return Task::none();
                }
                let order = self.next_bus_order;
                self.next_bus_order += 1;
                self.busses.push(BusState {
                    id: bus_id,
                    name,
                    order,
                    volume: 1.0,
                    pan: 0.0,
                    muted: false,
                    plugins: Vec::new(),
                    level_l: 0.0,
                    level_r: 0.0,
                });
            }
            AudioEvent::BusRemoved { bus_id } => {
                if let Some(sel) = self.selected_plugin {
                    if self.busses.iter()
                        .filter(|b| b.id == bus_id)
                        .any(|b| b.plugins.iter().any(|p| p.instance_id == sel))
                    {
                        self.selected_plugin = None;
                    }
                }
                self.busses.retain(|b| b.id != bus_id);
                // Any track that was routed to the removed bus falls back
                // to Master locally (the engine did the same server-side).
                for track in &mut self.tracks {
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
                if let Some(bus) = self.busses.iter_mut().find(|b| b.id == bus_id) {
                    if let Some(slot) =
                        bus.plugins.iter_mut().find(|p| p.instance_id == instance_id)
                    {
                        slot.params = params;
                        slot.has_gui = has_gui;
                    } else {
                        bus.plugins.push(PluginSlotState {
                            instance_id,
                            plugin_name,
                            clap_plugin_id,
                            clap_file_path,
                            params,
                            custom: PluginCustomState::Generic,
                            has_gui,
                            editor_open: false,
                        });
                    }
                }
            }
            AudioEvent::BusPluginRemoved { bus_id, instance_id } => {
                if let Some(bus) = self.busses.iter_mut().find(|b| b.id == bus_id) {
                    bus.plugins.retain(|p| p.instance_id != instance_id);
                }
                if self.selected_plugin == Some(instance_id) {
                    self.selected_plugin = None;
                }
            }
        }
        Task::none()
    }

    fn try_finish_save(&mut self) -> Task<Message> {
        let both_done = self.save_state.as_ref()
            .map(|s| s.clips_done && s.plugins_done)
            .unwrap_or(false);

        if !both_done {
            return Task::none();
        }

        let save = self.save_state.take().unwrap();
        let project_file = self.build_project_file();
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
