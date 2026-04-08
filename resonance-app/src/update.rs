/// Update logic and subscription for the Resonance application.
use crate::message::Message;
use crate::state::*;
use crate::theme;
use crate::util::db_to_gain;
use iced::{Subscription, Task};
use resonance_audio::types::*;

impl crate::Resonance {
    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Play => {
                self.engine.send(AudioCommand::Play);
                self.playing = true;
            }
            Message::Pause => {
                self.engine.send(AudioCommand::Pause);
                self.playing = false;
            }
            Message::Stop => {
                self.engine.send(AudioCommand::Stop);
                self.playing = false;
                self.playhead = 0;
            }
            Message::SkipBack => {
                let skip = self.sample_rate as u64 * 5;
                let new_pos = self.playhead.saturating_sub(skip);
                self.engine.send(AudioCommand::SeekTo(new_pos));
                self.playhead = new_pos;
            }
            Message::SkipForward => {
                let skip = self.sample_rate as u64 * 5;
                let new_pos = self.playhead + skip;
                self.engine.send(AudioCommand::SeekTo(new_pos));
                self.playhead = new_pos;
            }
            Message::AddTrack => {
                self.engine.send(AudioCommand::AddTrack);
            }
            Message::RemoveTrack(id) => {
                self.engine.send(AudioCommand::RemoveTrack { track_id: id });
            }
            Message::SetTrackVolume(id, vol_db) => {
                self.engine.send(AudioCommand::SetTrackVolume {
                    track_id: id,
                    volume: db_to_gain(vol_db),
                });
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == id) {
                    track.volume = vol_db;
                }
            }
            Message::SetTrackPan(id, pan) => {
                self.engine.send(AudioCommand::SetTrackPan {
                    track_id: id,
                    pan,
                });
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == id) {
                    track.pan = pan;
                }
            }
            Message::SetMasterVolume(vol_db) => {
                self.engine.send(AudioCommand::SetMasterVolume {
                    volume: db_to_gain(vol_db),
                });
                self.master_volume = vol_db;
            }
            Message::ToggleMute(id) => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == id) {
                    track.muted = !track.muted;
                    self.engine.send(AudioCommand::SetTrackMute {
                        track_id: id,
                        muted: track.muted,
                    });
                }
            }
            Message::ToggleSolo(id) => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == id) {
                    track.soloed = !track.soloed;
                    self.engine.send(AudioCommand::SetTrackSolo {
                        track_id: id,
                        soloed: track.soloed,
                    });
                }
            }
            Message::ImportFile(track_id) => {
                return Task::perform(
                    async move {
                        let result = rfd::AsyncFileDialog::new()
                            .add_filter("Audio", &["wav", "mp3", "flac", "ogg"])
                            .set_title("Import Audio File")
                            .pick_file()
                            .await;
                        let path = result.map(|f| f.path().to_string_lossy().to_string());
                        (track_id, path)
                    },
                    move |(tid, path)| Message::FileSelected(tid, path),
                );
            }
            Message::FileSelected(track_id, Some(path)) => {
                self.engine.send(AudioCommand::ImportClip {
                    track_id,
                    path,
                    start_sample: self.playhead,
                });
            }
            Message::FileSelected(_, None) => {}
            Message::DeleteClip(id) => {
                self.engine.send(AudioCommand::DeleteClip { clip_id: id });
                if self.selected_clip == Some(id) {
                    self.selected_clip = None;
                }
            }
            Message::ZoomIn => {
                self.zoom = (self.zoom * 1.5).min(1000.0);
            }
            Message::ZoomOut => {
                self.zoom = (self.zoom / 1.5).max(10.0);
            }
            Message::ToggleMonitor(id) => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == id) {
                    track.monitor_enabled = !track.monitor_enabled;
                    self.engine.send(AudioCommand::SetTrackMonitor {
                        track_id: id,
                        enabled: track.monitor_enabled,
                    });
                }
            }
            Message::ToggleTrackMono(id) => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == id) {
                    track.mono = !track.mono;
                    self.engine.send(AudioCommand::SetTrackMono {
                        track_id: id,
                        mono: track.mono,
                    });
                }
            }
            Message::ToggleRecordArm(id) => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == id) {
                    track.record_armed = !track.record_armed;
                    // Auto-select default input device when arming if none set
                    if track.record_armed && track.input_device_name.is_none() {
                        if let Some(default) = &self.default_input_device_name {
                            track.input_device_name = Some(default.clone());
                            self.engine.send(AudioCommand::SetTrackInputDevice {
                                track_id: id,
                                device_name: Some(default.clone()),
                            });
                        }
                    }
                    self.engine.send(AudioCommand::SetTrackRecordArm {
                        track_id: id,
                        armed: track.record_armed,
                    });
                }
            }
            Message::SetTrackInputDevice(id, device_name) => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == id) {
                    track.input_device_name = device_name.clone();
                    self.engine.send(AudioCommand::SetTrackInputDevice {
                        track_id: id,
                        device_name,
                    });
                }
            }
            Message::SetBpm(bpm) => {
                self.bpm = bpm.clamp(20.0, 999.0);
                self.engine.send(AudioCommand::SetBpm { bpm: self.bpm });
            }
            Message::ToggleMetronome => {
                self.metronome_enabled = !self.metronome_enabled;
                self.engine.send(AudioCommand::SetMetronomeEnabled {
                    enabled: self.metronome_enabled,
                });
            }
            Message::CycleTimeSignature => {
                // Cycle through common time signatures
                let (num, den) = match (self.time_sig_num, self.time_sig_den) {
                    (4, 4) => (3, 4),
                    (3, 4) => (6, 8),
                    (6, 8) => (5, 4),
                    (5, 4) => (7, 8),
                    (7, 8) => (2, 4),
                    _ => (4, 4),
                };
                self.time_sig_num = num;
                self.time_sig_den = den;
                self.engine.send(AudioCommand::SetTimeSignature {
                    numerator: num,
                    denominator: den,
                });
            }
            Message::AddPluginToTrack(track_id, plugin) => {
                self.engine.send(AudioCommand::AddPlugin {
                    track_id,
                    clap_file_path: plugin.clap_file_path,
                    clap_plugin_id: plugin.clap_plugin_id,
                });
            }
            Message::RemovePluginFromTrack(track_id, instance_id) => {
                self.engine.send(AudioCommand::RemovePlugin {
                    track_id,
                    instance_id,
                });
            }
            Message::TogglePluginPanel(instance_id) => {
                for track in &mut self.tracks {
                    if let Some(p) = track.plugins.iter_mut().find(|p| p.instance_id == instance_id) {
                        p.expanded = !p.expanded;
                        break;
                    }
                }
            }
            Message::SetPluginParam(instance_id, param_id, value) => {
                self.engine.send(AudioCommand::SetPluginParam {
                    instance_id,
                    param_id,
                    value,
                });
                // Update local param state for immediate UI feedback
                for track in &mut self.tracks {
                    if let Some(p) = track.plugins.iter_mut().find(|p| p.instance_id == instance_id) {
                        if let Some(param) = p.params.iter_mut().find(|pp| pp.id == param_id) {
                            param.current_value = value;
                        }
                        break;
                    }
                }
            }
            Message::DrumPadSelect(instance_id, pad_idx) => {
                for track in &mut self.tracks {
                    if let Some(p) = track.plugins.iter_mut().find(|p| p.instance_id == instance_id) {
                        if let PluginCustomState::Drums { ref mut selected_pad } = p.custom {
                            *selected_pad = pad_idx;
                        }
                        break;
                    }
                }
            }
            Message::PluginBrowseFile(instance_id) => {
                // Find the plugin's clap_plugin_id to determine file filter
                let filter = self.tracks.iter()
                    .flat_map(|t| t.plugins.iter())
                    .find(|p| p.instance_id == instance_id)
                    .map(|p| p.clap_plugin_id.clone());

                let task = Task::perform(
                    async move {
                        let mut dialog = rfd::AsyncFileDialog::new();
                        dialog = match filter.as_deref() {
                            Some("com.resonance.amp") => dialog.add_filter("NAM Model", &["nam"]),
                            Some("com.resonance.ir") => dialog.add_filter("WAV Audio", &["wav"]),
                            _ => dialog,
                        };
                        dialog.pick_file().await.map(|f| f.path().to_string_lossy().into_owned())
                    },
                    move |path| Message::PluginFileSelected(instance_id, path),
                );
                return task;
            }
            Message::PluginFileSelected(instance_id, path) => {
                if let Some(path) = path {
                    // Scan directory for sibling files
                    let dir = std::path::Path::new(&path).parent();
                    if let Some(dir) = dir {
                        let ext = self.tracks.iter()
                            .flat_map(|t| t.plugins.iter())
                            .find(|p| p.instance_id == instance_id)
                            .map(|p| match p.clap_plugin_id.as_str() {
                                "com.resonance.amp" => "nam",
                                "com.resonance.ir" => "wav",
                                _ => "",
                            })
                            .unwrap_or("");

                        let mut files: Vec<String> = std::fs::read_dir(dir)
                            .into_iter()
                            .flatten()
                            .filter_map(|e| e.ok())
                            .filter(|e| {
                                e.path().extension()
                                    .map(|x| x.eq_ignore_ascii_case(ext))
                                    .unwrap_or(false)
                            })
                            .map(|e| e.path().to_string_lossy().into_owned())
                            .collect();
                        files.sort();

                        let idx = files.iter().position(|f| f == &path).unwrap_or(0);

                        // Save state with new path, then load it back into the plugin
                        // First, save current state
                        self.engine.send(AudioCommand::SavePluginState { instance_id });

                        // Store the pending file info so we can act on it when state arrives
                        for track in &mut self.tracks {
                            if let Some(p) = track.plugins.iter_mut().find(|p| p.instance_id == instance_id) {
                                match &mut p.custom {
                                    PluginCustomState::Amp { model_name, file_list, current_index } => {
                                        let name = std::path::Path::new(&path)
                                            .file_stem()
                                            .map(|s| s.to_string_lossy().into_owned())
                                            .unwrap_or_default();
                                        *model_name = name;
                                        *file_list = files;
                                        *current_index = idx;
                                    }
                                    PluginCustomState::Ir { ir_name, file_list, current_index, .. } => {
                                        let name = std::path::Path::new(&path)
                                            .file_stem()
                                            .map(|s| s.to_string_lossy().into_owned())
                                            .unwrap_or_default();
                                        *ir_name = name;
                                        *file_list = files;
                                        *current_index = idx;
                                    }
                                    _ => {}
                                }
                                // Set file_select param to trigger loading
                                if let Some(param) = p.params.iter().find(|pp| {
                                    pp.name == "Model Select" || pp.name == "IR Select"
                                }) {
                                    self.engine.send(AudioCommand::SetPluginParam {
                                        instance_id,
                                        param_id: param.id,
                                        value: idx as f64,
                                    });
                                }
                                break;
                            }
                        }

                        // Inject the path via CLAP state
                        let persist_key = self.tracks.iter()
                            .flat_map(|t| t.plugins.iter())
                            .find(|p| p.instance_id == instance_id)
                            .map(|p| match p.clap_plugin_id.as_str() {
                                "com.resonance.amp" => "model-path",
                                "com.resonance.ir" => "ir-path",
                                _ => "",
                            })
                            .unwrap_or("");

                        if !persist_key.is_empty() {
                            // We need to modify the plugin state to include the new path.
                            // We'll handle this in the PluginStateSaved event handler.
                            // Store pending path for when state arrives.
                            self.pending_plugin_path = Some((instance_id, persist_key.to_string(), path));
                        }
                    }
                }
            }
            Message::PluginPrevFile(instance_id) => {
                self.step_plugin_file(instance_id, -1);
            }
            Message::PluginNextFile(instance_id) => {
                self.step_plugin_file(instance_id, 1);
            }
            Message::ScrollX(delta) => {
                self.scroll_offset = (self.scroll_offset + delta).max(0.0);
            }
            Message::ScrollY(delta) => {
                self.scroll_offset_y = (self.scroll_offset_y + delta).max(0.0);
                // Clamp to max content height
                let max_y = (self.tracks.len() as f32 * theme::TRACK_HEIGHT).max(0.0);
                self.scroll_offset_y = self.scroll_offset_y.min(max_y);
            }
            Message::SwitchView(mode) => {
                self.view_mode = mode;
            }
            Message::OpenSettings => {
                self.settings_open = true;
            }
            Message::CloseSettings => {
                self.settings_open = false;
            }
            Message::DismissError => {
                self.error_message = None;
            }
            Message::TogglePunch => {
                self.punch_enabled = !self.punch_enabled;
                // Set sensible defaults if enabling with no range set
                if self.punch_enabled && !self.punch_range_set {
                    // Default: 2 bars from current playhead
                    let spb = self.sample_rate as f64 * 60.0 / self.bpm as f64;
                    let two_bars = (spb * self.time_sig_num as f64 * 2.0) as u64;
                    self.punch_in = self.playhead;
                    self.punch_out = self.playhead + two_bars;
                    self.punch_range_set = true;
                }
                self.engine.send(AudioCommand::SetPunchRange {
                    enabled: self.punch_enabled,
                    punch_in: self.punch_in,
                    punch_out: self.punch_out,
                });
            }
            Message::SetPunchIn(pos) => {
                self.punch_in = pos;
                self.punch_range_set = true;
                if self.punch_enabled {
                    self.engine.send(AudioCommand::SetPunchRange {
                        enabled: true,
                        punch_in: self.punch_in,
                        punch_out: self.punch_out,
                    });
                }
            }
            Message::SetPunchOut(pos) => {
                self.punch_out = pos;
                self.punch_range_set = true;
                if self.punch_enabled {
                    self.engine.send(AudioCommand::SetPunchRange {
                        enabled: true,
                        punch_in: self.punch_in,
                        punch_out: self.punch_out,
                    });
                }
            }
            Message::StartPunchDrag(target) => {
                self.dragging_punch = Some(target);
            }
            Message::UpdatePunchDrag(x) => {
                if self.dragging_punch.is_some() {
                    // Convert pixel x to sample position
                    let seconds = (x + self.scroll_offset) / self.zoom;
                    let sample = (seconds.max(0.0) as f64 * self.sample_rate as f64) as u64;
                    match self.dragging_punch {
                        Some(PunchDragTarget::In) => {
                            self.punch_in = sample;
                        }
                        Some(PunchDragTarget::Out) => {
                            self.punch_out = sample;
                        }
                        None => {}
                    }
                    if self.punch_enabled {
                        self.engine.send(AudioCommand::SetPunchRange {
                            enabled: true,
                            punch_in: self.punch_in,
                            punch_out: self.punch_out,
                        });
                    }
                }
            }
            Message::EndPunchDrag => {
                self.dragging_punch = None;
            }
            Message::SelectClip(id) => {
                self.selected_clip = id;
            }
            Message::StartClipDrag { clip_id, grab_offset_x, start_x, start_y } => {
                if let Some(clip) = self.clips.iter().find(|c| c.id == clip_id) {
                    self.selected_clip = Some(clip_id);
                    self.clip_drag = Some(ClipDragState {
                        clip_id,
                        grab_offset_x,
                        original_start_sample: clip.start_sample,
                        original_track_id: clip.track_id,
                        current_x: start_x,
                        current_y: start_y,
                    });
                }
            }
            Message::UpdateClipDrag(x, y) => {
                if let Some(ref mut drag) = self.clip_drag {
                    drag.current_x = x;
                    drag.current_y = y;
                    // Live-update the clip position for visual feedback
                    let seconds = ((x - drag.grab_offset_x) + self.scroll_offset) / self.zoom;
                    let new_start = if seconds < 0.0 {
                        0u64
                    } else {
                        (seconds as f64 * self.sample_rate as f64) as u64
                    };
                    // Determine target track from y position
                    let ruler_height = 30.0f32;
                    let mut sorted_tracks: Vec<&TrackState> = self.tracks.iter().collect();
                    sorted_tracks.sort_by_key(|t| t.order);
                    let track_idx = ((y - ruler_height + self.scroll_offset_y) / theme::TRACK_HEIGHT)
                        .floor()
                        .max(0.0) as usize;
                    let target_track_id = sorted_tracks
                        .get(track_idx)
                        .map(|t| t.id)
                        .unwrap_or(drag.original_track_id);
                    if let Some(clip) = self.clips.iter_mut().find(|c| c.id == drag.clip_id) {
                        clip.start_sample = new_start;
                        clip.track_id = target_track_id;
                    }
                }
            }
            Message::EndClipDrag => {
                if let Some(drag) = self.clip_drag.take() {
                    if let Some(clip) = self.clips.iter().find(|c| c.id == drag.clip_id) {
                        self.engine.send(AudioCommand::MoveClip {
                            clip_id: drag.clip_id,
                            new_start_sample: clip.start_sample,
                            new_track_id: clip.track_id,
                        });
                    }
                }
            }
            Message::StartClipTrim { clip_id, edge, anchor_x } => {
                if let Some(clip) = self.clips.iter().find(|c| c.id == clip_id) {
                    self.selected_clip = Some(clip_id);
                    self.clip_trim = Some(ClipTrimState {
                        clip_id,
                        edge,
                        original_start_sample: clip.start_sample,
                        original_trim_start: clip.trim_start_frames,
                        original_trim_end: clip.trim_end_frames,
                        original_total_frames: clip.total_frames,
                        anchor_x,
                    });
                }
            }
            Message::UpdateClipTrim(x) => {
                if let Some(ref trim) = self.clip_trim.clone() {
                    let delta_px = x - trim.anchor_x;
                    let delta_seconds = delta_px / self.zoom;
                    let delta_frames = (delta_seconds.abs() as f64 * self.sample_rate as f64) as u64;
                    let min_duration_frames = (0.01 * self.sample_rate as f64) as u64;

                    match trim.edge {
                        ClipEdge::Left => {
                            let max_trim = trim.original_total_frames
                                .saturating_sub(trim.original_trim_end)
                                .saturating_sub(min_duration_frames);
                            let new_trim_start = if delta_seconds > 0.0 {
                                (trim.original_trim_start + delta_frames).min(max_trim)
                            } else {
                                trim.original_trim_start.saturating_sub(delta_frames)
                            };
                            let trim_delta = new_trim_start as i64 - trim.original_trim_start as i64;
                            let new_start = (trim.original_start_sample as i64 + trim_delta).max(0) as u64;
                            let new_duration = trim.original_total_frames
                                .saturating_sub(new_trim_start)
                                .saturating_sub(trim.original_trim_end);
                            if let Some(clip) = self.clips.iter_mut().find(|c| c.id == trim.clip_id) {
                                clip.start_sample = new_start;
                                clip.trim_start_frames = new_trim_start;
                                clip.duration_samples = new_duration;
                            }
                        }
                        ClipEdge::Right => {
                            let max_trim = trim.original_total_frames
                                .saturating_sub(trim.original_trim_start)
                                .saturating_sub(min_duration_frames);
                            let new_trim_end = if delta_seconds < 0.0 {
                                (trim.original_trim_end + delta_frames).min(max_trim)
                            } else {
                                trim.original_trim_end.saturating_sub(delta_frames)
                            };
                            let new_duration = trim.original_total_frames
                                .saturating_sub(trim.original_trim_start)
                                .saturating_sub(new_trim_end);
                            if let Some(clip) = self.clips.iter_mut().find(|c| c.id == trim.clip_id) {
                                clip.trim_end_frames = new_trim_end;
                                clip.duration_samples = new_duration;
                            }
                        }
                    }
                }
            }
            Message::EndClipTrim => {
                if let Some(trim) = self.clip_trim.take() {
                    if let Some(clip) = self.clips.iter().find(|c| c.id == trim.clip_id) {
                        self.engine.send(AudioCommand::TrimClip {
                            clip_id: trim.clip_id,
                            new_start_sample: clip.start_sample,
                            trim_start_frames: clip.trim_start_frames,
                            trim_end_frames: clip.trim_end_frames,
                        });
                    }
                }
            }
            Message::Tick => {
                while let Some(event) = self.engine.try_recv() {
                    self.handle_engine_event(event);
                }
                // Update VU meter levels
                {
                    let (track_peaks, master_peak_l, master_peak_r) =
                        self.engine.read_and_clear_peaks();
                    const PEAK_DECAY: f32 = 0.85;
                    for track in &mut self.tracks {
                        track.level_l *= PEAK_DECAY;
                        track.level_r *= PEAK_DECAY;
                    }
                    for (track_id, pl, pr) in track_peaks {
                        if let Some(track) =
                            self.tracks.iter_mut().find(|t| t.id == track_id)
                        {
                            if pl > track.level_l {
                                track.level_l = pl;
                            }
                            if pr > track.level_r {
                                track.level_r = pr;
                            }
                        }
                    }
                    self.master_level_l =
                        (self.master_level_l * PEAK_DECAY).max(master_peak_l);
                    self.master_level_r =
                        (self.master_level_r * PEAK_DECAY).max(master_peak_r);
                }
                // Auto-follow playhead during playback
                if self.playing {
                    let playhead_seconds = self.playhead as f64 / self.sample_rate as f64;
                    let playhead_x = playhead_seconds as f32 * self.zoom - self.scroll_offset;
                    // Assume ~1000px visible width (we don't have exact bounds here)
                    let visible_width = 1000.0;
                    if playhead_x > visible_width * 0.8 {
                        self.scroll_offset = playhead_seconds as f32 * self.zoom - visible_width * 0.5;
                    } else if playhead_x < 0.0 {
                        self.scroll_offset = (playhead_seconds as f32 * self.zoom - visible_width * 0.2).max(0.0);
                    }
                }
            }
        }
        Task::none()
    }

    /// Step through the plugin file list by `direction` (-1 for previous, 1 for next).
    fn step_plugin_file(&mut self, instance_id: PluginInstanceId, direction: i32) {
        for track in &mut self.tracks {
            if let Some(p) = track.plugins.iter_mut().find(|p| p.instance_id == instance_id) {
                let new_idx = match &p.custom {
                    PluginCustomState::Amp { file_list, current_index, .. } => {
                        if file_list.is_empty() { return; }
                        Self::wrap_index(*current_index, file_list.len(), direction)
                    }
                    PluginCustomState::Ir { file_list, current_index, .. } => {
                        if file_list.is_empty() { return; }
                        Self::wrap_index(*current_index, file_list.len(), direction)
                    }
                    _ => return,
                };
                // Update local state
                match &mut p.custom {
                    PluginCustomState::Amp { model_name, file_list, current_index } => {
                        *current_index = new_idx;
                        *model_name = std::path::Path::new(&file_list[new_idx])
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                    }
                    PluginCustomState::Ir { ir_name, file_list, current_index, .. } => {
                        *current_index = new_idx;
                        *ir_name = std::path::Path::new(&file_list[new_idx])
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                    }
                    _ => {}
                }
                // Set file_select param
                if let Some(param) = p.params.iter().find(|pp| {
                    pp.name == "Model Select" || pp.name == "IR Select"
                }) {
                    self.engine.send(AudioCommand::SetPluginParam {
                        instance_id,
                        param_id: param.id,
                        value: new_idx as f64,
                    });
                }
                break;
            }
        }
    }

    /// Compute the next index wrapping around the list boundaries.
    fn wrap_index(current: usize, len: usize, direction: i32) -> usize {
        if direction < 0 {
            if current == 0 { len - 1 } else { current - 1 }
        } else {
            if current >= len - 1 { 0 } else { current + 1 }
        }
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::Tick)
    }
}
