/// Engine event handling for the Resonance application.
use crate::state::*;
use resonance_audio::types::*;

impl crate::Resonance {
    pub(crate) fn handle_engine_event(&mut self, event: AudioEvent) {
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
            AudioEvent::TrackAdded { track_id } => {
                let order = self.next_track_order;
                self.next_track_order += 1;
                self.tracks.push(TrackState {
                    id: track_id,
                    name: format!("Track {}", track_id),
                    volume: 0.0, // 0 dB = unity gain
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
                });
            }
            AudioEvent::TrackRemoved { track_id } => {
                self.tracks.retain(|t| t.id != track_id);
                self.clips.retain(|c| c.track_id != track_id);
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
                self.playing = false;
                self.recording = false;
                self.playhead = 0;
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
                params,
            } => {
                let custom = match clap_plugin_id.as_str() {
                    "com.resonance.drums" => PluginCustomState::Drums(Default::default()),
                    "com.resonance.amp" => PluginCustomState::Amp(Default::default()),
                    "com.resonance.ir" => PluginCustomState::Ir(Default::default()),
                    _ => PluginCustomState::Generic,
                };
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
                    track.plugins.push(PluginSlotState {
                        instance_id,
                        plugin_name,
                        clap_plugin_id,
                        params,
                        custom,
                    });
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
            AudioEvent::PluginStateSaved { instance_id, data } => {
                // If we have a pending path to inject, modify the state and reload.
                // State format is plain JSON: { "params": {...}, "model_path": "...", ... }
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
        }
    }
}
