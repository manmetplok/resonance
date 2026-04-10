/// Update logic and subscription for the Resonance application.
use crate::message::Message;
use crate::project::{self, LoadedProject, ProjectBus, ProjectClip, ProjectFile, ProjectMidiClip, ProjectMidiNote, ProjectPlugin, ProjectTrack, SaveCollector};
use crate::state::*;
use crate::theme;
use crate::util::db_to_gain;
use iced::{keyboard, Subscription, Task};
use resonance_audio::types::*;
use std::collections::HashMap;

pub mod compose;
pub mod drumroll;

impl crate::Resonance {
    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Compose(m) => {
                crate::update::compose::handle(self, m);
            }
            Message::Play => {
                self.engine.send(AudioCommand::Play);
                self.playing = true;
            }
            Message::Record => {
                // Only meaningful when at least one track is armed; the UI
                // disables the button otherwise.
                if self.tracks.iter().any(|t| t.record_armed) {
                    self.engine.send(AudioCommand::Record);
                    self.playing = true;
                    // self.recording flips true when RecordingStarted arrives.
                }
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
            Message::SeekToSample(pos) => {
                self.engine.send(AudioCommand::SeekTo(pos));
                self.playhead = pos;
            }
            Message::AddTrack => {
                self.engine.send(AudioCommand::AddTrack);
                self.add_track_menu_open = false;
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
            Message::SetTrackName(track_id, name) => {
                if let Some(t) = self.tracks.iter_mut().find(|t| t.id == track_id) {
                    t.name = name;
                }
            }
            Message::SetInstrumentType(track_id, ty) => {
                if let Some(t) = self.tracks.iter_mut().find(|t| t.id == track_id) {
                    t.instrument_type = ty;
                    t.instrument_icon = crate::state::InstrumentIcon::default_for(ty);
                }
            }
            Message::SetInstrumentIcon(track_id, icon) => {
                if let Some(t) = self.tracks.iter_mut().find(|t| t.id == track_id) {
                    t.instrument_icon = icon;
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
                    // Reset port selection to the first channel pair
                    // whenever the device changes — the old port may not
                    // exist on the new card.
                    track.input_port_index = 0;
                    self.engine.send(AudioCommand::SetTrackInputDevice {
                        track_id: id,
                        device_name,
                    });
                    self.engine.send(AudioCommand::SetTrackInputPort {
                        track_id: id,
                        port_index: 0,
                    });
                }
            }
            Message::SetTrackInputPort(id, port_index) => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == id) {
                    track.input_port_index = port_index;
                    self.engine.send(AudioCommand::SetTrackInputPort {
                        track_id: id,
                        port_index,
                    });
                }
            }
            Message::ToggleSubTracksVisible(id) => {
                if !self.collapsed_sub_track_parents.insert(id) {
                    // Already present — the insert was a no-op, so toggle
                    // to the expanded state by removing.
                    self.collapsed_sub_track_parents.remove(&id);
                }
            }
            Message::SetBpmText(s) => {
                // Accept any keystroke so the user can type freely; only
                // commit on Enter via CommitBpm.
                self.bpm_input = s;
            }
            Message::CommitBpm => {
                match self.bpm_input.trim().parse::<f32>() {
                    Ok(parsed) => {
                        self.bpm = parsed.clamp(20.0, 300.0);
                        self.engine.send(AudioCommand::SetBpm { bpm: self.bpm });
                    }
                    Err(_) => {}
                }
                // Always rewrite the buffer from the current (possibly clamped
                // or reverted) BPM so the field shows a sane value.
                self.bpm_input = format!("{:.0}", self.bpm);
            }
            Message::CyclePrecountBars => {
                // Cycle through common pre-count lengths.
                self.precount_bars = match self.precount_bars {
                    0 => 1,
                    1 => 2,
                    2 => 4,
                    _ => 0,
                };
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
                if self.selected_plugin == Some(instance_id) {
                    self.selected_plugin = None;
                } else {
                    self.selected_plugin = Some(instance_id);
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
            Message::OpenPluginEditor(instance_id) => {
                self.engine
                    .send(AudioCommand::OpenPluginEditor { instance_id });
                for track in &mut self.tracks {
                    if let Some(p) = track
                        .plugins
                        .iter_mut()
                        .find(|p| p.instance_id == instance_id)
                    {
                        p.editor_open = true;
                        break;
                    }
                }
            }
            Message::ClosePluginEditor(instance_id) => {
                self.engine
                    .send(AudioCommand::ClosePluginEditor { instance_id });
                for track in &mut self.tracks {
                    if let Some(p) = track
                        .plugins
                        .iter_mut()
                        .find(|p| p.instance_id == instance_id)
                    {
                        p.editor_open = false;
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
                    let ext = self.tracks.iter()
                        .flat_map(|t| t.plugins.iter())
                        .find(|p| p.instance_id == instance_id)
                        .map(|p| match p.clap_plugin_id.as_str() {
                            "com.resonance.amp" => "nam",
                            "com.resonance.ir" => "wav",
                            _ => "",
                        })
                        .unwrap_or("")
                        .to_string();

                    // Scan sibling files asynchronously to avoid blocking the UI
                    return Task::perform(
                        async move {
                            let dir = std::path::Path::new(&path).parent().map(|d| d.to_path_buf());
                            let files = if let Some(dir) = dir {
                                let mut files: Vec<String> = std::fs::read_dir(dir)
                                    .into_iter()
                                    .flatten()
                                    .filter_map(|e| e.ok())
                                    .filter(|e| {
                                        e.path().extension()
                                            .map(|x| x.eq_ignore_ascii_case(ext.as_str()))
                                            .unwrap_or(false)
                                    })
                                    .map(|e| e.path().to_string_lossy().into_owned())
                                    .collect();
                                files.sort();
                                files
                            } else {
                                Vec::new()
                            };
                            (path, files)
                        },
                        move |(path, files)| Message::PluginFileScanComplete(instance_id, Some(path), files),
                    );
                }
            }
            Message::PluginPrevFile(instance_id) => {
                self.step_plugin_file(instance_id, -1);
            }
            Message::PluginNextFile(instance_id) => {
                self.step_plugin_file(instance_id, 1);
            }
            Message::ScrollX(delta) => {
                let max_x =
                    (self.timeline_content_width - self.viewport_width).max(0.0);
                self.scroll_offset =
                    (self.scroll_offset + delta).clamp(0.0, max_x);
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
            Message::OpenAddTrackMenu => {
                self.add_track_menu_open = true;
            }
            Message::CloseAddTrackMenu => {
                self.add_track_menu_open = false;
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
                if self.punch_in > self.punch_out {
                    std::mem::swap(&mut self.punch_in, &mut self.punch_out);
                }
                if self.punch_enabled {
                    self.engine.send(AudioCommand::SetPunchRange {
                        enabled: true,
                        punch_in: self.punch_in,
                        punch_out: self.punch_out,
                    });
                }
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
                let mut tasks = Vec::new();
                while let Some(event) = self.engine.try_recv() {
                    let task = self.handle_engine_event(event);
                    tasks.push(task);
                }
                // Update VU meter levels
                {
                    let (track_peaks, bus_peaks, master_peak_l, master_peak_r) =
                        self.engine.read_and_clear_peaks();
                    const PEAK_DECAY: f32 = 0.85;
                    for track in &mut self.tracks {
                        track.level_l *= PEAK_DECAY;
                        track.level_r *= PEAK_DECAY;
                    }
                    for bus in &mut self.busses {
                        bus.level_l *= PEAK_DECAY;
                        bus.level_r *= PEAK_DECAY;
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
                    for (bus_id, pl, pr) in bus_peaks {
                        if let Some(bus) =
                            self.busses.iter_mut().find(|b| b.id == bus_id)
                        {
                            if pl > bus.level_l {
                                bus.level_l = pl;
                            }
                            if pr > bus.level_r {
                                bus.level_r = pr;
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
                    let visible_width = self.viewport_width;
                    if playhead_x > visible_width * 0.8 {
                        self.scroll_offset = playhead_seconds as f32 * self.zoom - visible_width * 0.5;
                    } else if playhead_x < 0.0 {
                        self.scroll_offset = (playhead_seconds as f32 * self.zoom - visible_width * 0.2).max(0.0);
                    }
                }
                if !tasks.is_empty() {
                    return Task::batch(tasks);
                }
            }
            Message::PluginFileScanComplete(instance_id, path, files) => {
                if let Some(path) = path {
                    let idx = files.iter().position(|f| f == &path).unwrap_or(0);

                    self.engine.send(AudioCommand::SavePluginState { instance_id });

                    for track in &mut self.tracks {
                        if let Some(p) = track.plugins.iter_mut().find(|p| p.instance_id == instance_id) {
                            match &mut p.custom {
                                PluginCustomState::Amp(ref mut state) => {
                                    state.model_name = std::path::Path::new(&path)
                                        .file_stem()
                                        .map(|s| s.to_string_lossy().into_owned())
                                        .unwrap_or_default();
                                    state.file_list = files.clone();
                                    state.current_index = idx;
                                }
                                PluginCustomState::Ir(ref mut state) => {
                                    state.ir_name = std::path::Path::new(&path)
                                        .file_stem()
                                        .map(|s| s.to_string_lossy().into_owned())
                                        .unwrap_or_default();
                                    state.file_list = files.clone();
                                    state.current_index = idx;
                                }
                                _ => {}
                            }
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

                    let persist_key = self.tracks.iter()
                        .flat_map(|t| t.plugins.iter())
                        .find(|p| p.instance_id == instance_id)
                        .map(|p| match p.clap_plugin_id.as_str() {
                            "com.resonance.amp" => "model_path",
                            "com.resonance.ir" => "ir_path",
                            _ => "",
                        })
                        .unwrap_or("");

                    if !persist_key.is_empty() {
                        self.pending_plugin_path = Some((instance_id, persist_key.to_string(), path));
                    }
                }
            }
            Message::ViewportWidth(w) => {
                self.viewport_width = w;
            }
            Message::TimelineContentSize(w, h) => {
                self.timeline_content_width = w;
                self.timeline_content_height = h;
                // Re-clamp scroll offsets if content shrank.
                let max_x = (w - self.viewport_width).max(0.0);
                if self.scroll_offset > max_x {
                    self.scroll_offset = max_x;
                }
                let max_y = (h - 1.0).max(0.0);
                if self.scroll_offset_y > max_y {
                    self.scroll_offset_y = max_y;
                }
            }
            Message::ScrollToX(x) => {
                let max_x = (self.timeline_content_width - self.viewport_width).max(0.0);
                self.scroll_offset = x.clamp(0.0, max_x);
            }
            Message::ScrollToY(y) => {
                self.scroll_offset_y = y.max(0.0);
                let max_y =
                    (self.tracks.len() as f32 * theme::TRACK_HEIGHT).max(0.0);
                self.scroll_offset_y = self.scroll_offset_y.min(max_y);
            }
            Message::BounceToWav => {
                return Task::perform(
                    async move {
                        let result = rfd::AsyncFileDialog::new()
                            .add_filter("WAV Audio", &["wav"])
                            .set_title("Bounce to WAV")
                            .set_file_name("bounce.wav")
                            .save_file()
                            .await;
                        result.map(|f| f.path().to_string_lossy().to_string())
                    },
                    Message::BouncePathSelected,
                );
            }
            Message::BouncePathSelected(Some(path)) => {
                self.bouncing = true;
                self.engine.send(AudioCommand::BounceToWav { path });
            }
            Message::BouncePathSelected(None) => {}
            Message::SaveProject => {
                if self.project_path.is_some() {
                    return self.start_save();
                } else {
                    return self.update(Message::SaveProjectAs);
                }
            }
            Message::SaveProjectAs => {
                return Task::perform(
                    async move {
                        let result = rfd::AsyncFileDialog::new()
                            .set_title("Save Project")
                            .set_file_name("MyProject.rproj")
                            .save_file()
                            .await;
                        result.map(|f| f.path().to_string_lossy().to_string())
                    },
                    Message::SavePathSelected,
                );
            }
            Message::SavePathSelected(Some(path)) => {
                // Ensure path ends with .rproj
                let path = if path.ends_with(".rproj") {
                    std::path::PathBuf::from(path)
                } else {
                    std::path::PathBuf::from(format!("{path}.rproj"))
                };
                self.project_path = Some(path);
                return self.start_save();
            }
            Message::SavePathSelected(None) => {}
            Message::OpenProject => {
                return Task::perform(
                    async move {
                        let result = rfd::AsyncFileDialog::new()
                            .set_title("Open Project")
                            .add_filter("Resonance Project", &["rproj"])
                            .pick_folder()
                            .await;
                        result.map(|f| f.path().to_string_lossy().to_string())
                    },
                    Message::OpenPathSelected,
                );
            }
            Message::OpenPathSelected(Some(path)) => {
                let path = std::path::PathBuf::from(path);
                self.project_path = Some(path.clone());
                return Task::perform(
                    async move {
                        project::load_project(&path)
                            .map(|p| Box::new(p))
                    },
                    Message::ProjectLoaded,
                );
            }
            Message::OpenPathSelected(None) => {}
            Message::ProjectSaved(Ok(())) => {
                self.save_state = None;
            }
            Message::ProjectSaved(Err(e)) => {
                self.save_state = None;
                self.error_message = Some(format!("Save failed: {e}"));
            }
            Message::ProjectLoaded(Ok(loaded)) => {
                // Stop playback, clear state, then replay
                self.engine.send(AudioCommand::Stop);
                self.playing = false;
                self.recording = false;
                self.loading = true;
                self.pending_load = Some(loaded);
                self.engine.send(AudioCommand::ClearAll);
            }
            Message::ProjectLoaded(Err(e)) => {
                self.error_message = Some(format!("Load failed: {e}"));
            }
            Message::AddInstrumentTrack => {
                self.engine.send(AudioCommand::AddInstrumentTrack);
                self.add_track_menu_open = false;
            }
            Message::AddBus => {
                self.engine.send(AudioCommand::AddBus);
            }
            Message::RemoveBus(bus_id) => {
                self.engine.send(AudioCommand::RemoveBus { bus_id });
                // Locally clear any track routings pointing here; the engine
                // does the same. Mirrors how TrackRemoved clears refs.
                for track in &mut self.tracks {
                    if track.output == TrackOutput::Bus(bus_id) {
                        track.output = TrackOutput::Master;
                    }
                }
            }
            Message::SetBusVolume(bus_id, vol_db) => {
                self.engine.send(AudioCommand::SetBusVolume {
                    bus_id,
                    volume: db_to_gain(vol_db),
                });
                if let Some(bus) = self.busses.iter_mut().find(|b| b.id == bus_id) {
                    bus.volume = vol_db;
                }
            }
            Message::SetBusPan(bus_id, pan) => {
                self.engine.send(AudioCommand::SetBusPan { bus_id, pan });
                if let Some(bus) = self.busses.iter_mut().find(|b| b.id == bus_id) {
                    bus.pan = pan;
                }
            }
            Message::ToggleBusMute(bus_id) => {
                if let Some(bus) = self.busses.iter_mut().find(|b| b.id == bus_id) {
                    bus.muted = !bus.muted;
                    self.engine.send(AudioCommand::SetBusMute {
                        bus_id,
                        muted: bus.muted,
                    });
                }
            }
            Message::SetTrackOutput(track_id, output) => {
                self.engine.send(AudioCommand::SetTrackOutput { track_id, output });
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
                    track.output = output;
                }
            }
            Message::AddPluginToBus(bus_id, plugin) => {
                self.engine.send(AudioCommand::AddPluginToBus {
                    bus_id,
                    clap_file_path: plugin.clap_file_path,
                    clap_plugin_id: plugin.clap_plugin_id,
                });
            }
            Message::RemovePluginFromBus(bus_id, instance_id) => {
                self.engine.send(AudioCommand::RemovePluginFromBus {
                    bus_id,
                    instance_id,
                });
            }
            Message::CreateMidiClip(track_id) => {
                let duration_ticks = 4 * self.time_sig_num as u64 * TICKS_PER_QUARTER_NOTE;
                self.engine.send(AudioCommand::CreateMidiClip {
                    track_id,
                    start_sample: self.playhead,
                    duration_ticks,
                    name: "MIDI Clip".to_string(),
                });
            }
            Message::DeleteMidiClip(id) => {
                self.engine.send(AudioCommand::DeleteMidiClip { clip_id: id });
                if self.selected_midi_clip == Some(id) {
                    self.selected_midi_clip = None;
                }
            }
            Message::StartMidiClipDrag { clip_id, grab_offset_x, start_x, start_y } => {
                if let Some(clip) = self.midi_clips.iter().find(|c| c.id == clip_id) {
                    self.selected_midi_clip = Some(clip_id);
                    self.selected_clip = None;
                    self.midi_clip_drag = Some(MidiClipDragState {
                        clip_id,
                        grab_offset_x,
                        original_track_id: clip.track_id,
                        current_x: start_x,
                        current_y: start_y,
                    });
                }
            }
            Message::UpdateMidiClipDrag(x, y) => {
                if let Some(ref mut drag) = self.midi_clip_drag {
                    drag.current_x = x;
                    drag.current_y = y;
                    let seconds = ((x - drag.grab_offset_x) + self.scroll_offset) / self.zoom;
                    let new_start = if seconds < 0.0 {
                        0u64
                    } else {
                        (seconds as f64 * self.sample_rate as f64) as u64
                    };
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
                    if let Some(clip) = self.midi_clips.iter_mut().find(|c| c.id == drag.clip_id) {
                        clip.start_sample = new_start;
                        clip.track_id = target_track_id;
                    }
                }
            }
            Message::EndMidiClipDrag => {
                if let Some(drag) = self.midi_clip_drag.take() {
                    if let Some(clip) = self.midi_clips.iter().find(|c| c.id == drag.clip_id) {
                        self.engine.send(AudioCommand::MoveMidiClip {
                            clip_id: drag.clip_id,
                            new_start_sample: clip.start_sample,
                            new_track_id: clip.track_id,
                        });
                    }
                }
            }
            Message::StartMidiClipTrim { clip_id, edge, anchor_x } => {
                if let Some(clip) = self.midi_clips.iter().find(|c| c.id == clip_id) {
                    self.selected_midi_clip = Some(clip_id);
                    self.selected_clip = None;
                    self.midi_clip_trim = Some(MidiClipTrimState {
                        clip_id,
                        edge,
                        original_start_sample: clip.start_sample,
                        original_duration_ticks: clip.duration_ticks,
                        original_trim_start_ticks: clip.trim_start_ticks,
                        original_trim_end_ticks: clip.trim_end_ticks,
                        anchor_x,
                    });
                }
            }
            Message::UpdateMidiClipTrim(x) => {
                if let Some(ref trim) = self.midi_clip_trim.clone() {
                    let delta_px = x - trim.anchor_x;
                    let delta_seconds = delta_px / self.zoom;
                    let samples_per_tick = (self.sample_rate as f64 * 60.0 / self.bpm as f64)
                        / TICKS_PER_QUARTER_NOTE as f64;
                    let delta_ticks = ((delta_seconds.abs() as f64 * self.sample_rate as f64)
                        / samples_per_tick) as u64;
                    let total_ticks = trim.original_duration_ticks
                        + trim.original_trim_start_ticks
                        + trim.original_trim_end_ticks;
                    let min_ticks = TICKS_PER_QUARTER_NOTE;

                    match trim.edge {
                        ClipEdge::Left => {
                            let max_trim = total_ticks
                                .saturating_sub(trim.original_trim_end_ticks)
                                .saturating_sub(min_ticks);
                            let new_trim_start = if delta_seconds > 0.0 {
                                (trim.original_trim_start_ticks + delta_ticks).min(max_trim)
                            } else {
                                trim.original_trim_start_ticks.saturating_sub(delta_ticks)
                            };
                            let trim_delta = new_trim_start as i64
                                - trim.original_trim_start_ticks as i64;
                            let sample_delta =
                                (trim_delta as f64 * samples_per_tick) as i64;
                            let new_start =
                                (trim.original_start_sample as i64 + sample_delta).max(0) as u64;
                            let new_duration = total_ticks
                                .saturating_sub(new_trim_start)
                                .saturating_sub(trim.original_trim_end_ticks);
                            if let Some(clip) =
                                self.midi_clips.iter_mut().find(|c| c.id == trim.clip_id)
                            {
                                clip.start_sample = new_start;
                                clip.trim_start_ticks = new_trim_start;
                                clip.duration_ticks = new_duration;
                            }
                        }
                        ClipEdge::Right => {
                            let max_trim = total_ticks
                                .saturating_sub(trim.original_trim_start_ticks)
                                .saturating_sub(min_ticks);
                            let new_trim_end = if delta_seconds < 0.0 {
                                (trim.original_trim_end_ticks + delta_ticks).min(max_trim)
                            } else {
                                trim.original_trim_end_ticks.saturating_sub(delta_ticks)
                            };
                            let new_duration = total_ticks
                                .saturating_sub(trim.original_trim_start_ticks)
                                .saturating_sub(new_trim_end);
                            if let Some(clip) =
                                self.midi_clips.iter_mut().find(|c| c.id == trim.clip_id)
                            {
                                clip.trim_end_ticks = new_trim_end;
                                clip.duration_ticks = new_duration;
                            }
                        }
                    }
                }
            }
            Message::EndMidiClipTrim => {
                if let Some(trim) = self.midi_clip_trim.take() {
                    if let Some(clip) = self.midi_clips.iter().find(|c| c.id == trim.clip_id) {
                        self.engine.send(AudioCommand::TrimMidiClip {
                            clip_id: trim.clip_id,
                            new_start_sample: clip.start_sample,
                            trim_start_ticks: clip.trim_start_ticks,
                            trim_end_ticks: clip.trim_end_ticks,
                        });
                    }
                }
            }
            Message::OpenMidiEditor(clip_id) => {
                self.open_midi_editor(clip_id);
            }
            Message::OpenSelectedMidiClip => {
                if let Some(clip_id) = self.selected_midi_clip {
                    self.open_midi_editor(clip_id);
                }
            }
            Message::CloseMidiEditor => {
                self.editing_midi_clip = None;
            }
            Message::MidiEditorAddNote { clip_id, note, start_tick, duration_ticks, velocity } => {
                self.engine.send(AudioCommand::AddMidiNote {
                    clip_id,
                    note: MidiNote { note, velocity, start_tick, duration_ticks },
                });
            }
            Message::MidiEditorRemoveNote { clip_id, note_index } => {
                self.engine.send(AudioCommand::RemoveMidiNote {
                    clip_id,
                    note_index,
                });
            }
            Message::MidiEditorMoveNote { clip_id, note_index, new_start_tick, new_note } => {
                self.engine.send(AudioCommand::MoveMidiNote {
                    clip_id,
                    note_index,
                    new_start_tick,
                    new_note,
                });
            }
            Message::MidiEditorResizeNote { clip_id, note_index, new_duration_ticks } => {
                self.engine.send(AudioCommand::ResizeMidiNote {
                    clip_id,
                    note_index,
                    new_duration_ticks,
                });
            }
            Message::MidiEditorSelectNote { note_index } => {
                if let Some(ref mut editor) = self.editing_midi_clip {
                    editor.selected_note = note_index;
                }
            }
            Message::MidiEditorPreviewNote(track_id, note) => {
                self.engine.send(AudioCommand::SendNoteOn {
                    track_id,
                    note,
                    velocity: 0.8,
                });
            }
            Message::MidiEditorStopPreview(track_id, note) => {
                self.engine.send(AudioCommand::SendNoteOff {
                    track_id,
                    note,
                });
            }
            Message::MidiEditorScrollX(delta) => {
                if let Some(ref mut editor) = self.editing_midi_clip {
                    editor.scroll_x = (editor.scroll_x + delta).max(0.0);
                }
            }
            Message::MidiEditorScrollY(delta) => {
                if let Some(ref mut editor) = self.editing_midi_clip {
                    editor.scroll_y = (editor.scroll_y + delta).max(0.0);
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
                    PluginCustomState::Amp(state) => {
                        if state.file_list.is_empty() { return; }
                        Self::wrap_index(state.current_index, state.file_list.len(), direction)
                    }
                    PluginCustomState::Ir(state) => {
                        if state.file_list.is_empty() { return; }
                        Self::wrap_index(state.current_index, state.file_list.len(), direction)
                    }
                    _ => return,
                };
                // Update local state
                match &mut p.custom {
                    PluginCustomState::Amp(ref mut state) => {
                        state.current_index = new_idx;
                        state.model_name = std::path::Path::new(&state.file_list[new_idx])
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                    }
                    PluginCustomState::Ir(ref mut state) => {
                        state.current_index = new_idx;
                        state.ir_name = std::path::Path::new(&state.file_list[new_idx])
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

    fn start_save(&mut self) -> Task<Message> {
        let path = match &self.project_path {
            Some(p) => p.clone(),
            None => return Task::none(),
        };
        self.save_state = Some(SaveCollector {
            path,
            clip_data: HashMap::new(),
            plugin_states: Vec::new(),
            clips_done: false,
            plugins_done: false,
        });
        self.engine.send(AudioCommand::ExportAllClipData);
        self.engine.send(AudioCommand::SaveAllPluginStates);
        Task::none()
    }

    /// Build a ProjectFile from current GUI state.
    pub(crate) fn build_project_file(&self) -> ProjectFile {
        let tracks = self.sorted_tracks().iter().map(|t| {
            ProjectTrack {
                id: t.id,
                name: t.name.clone(),
                order: t.order,
                volume: t.volume,
                pan: t.pan,
                muted: t.muted,
                soloed: t.soloed,
                record_armed: t.record_armed,
                monitor_enabled: t.monitor_enabled,
                mono: t.mono,
                input_device_name: t.input_device_name.clone(),
                plugins: t.plugins.iter().map(|p| {
                    ProjectPlugin {
                        instance_id: p.instance_id,
                        plugin_name: p.plugin_name.clone(),
                        clap_plugin_id: p.clap_plugin_id.clone(),
                        clap_file_path: p.clap_file_path.clone(),
                        state_file: format!("plugins/plugin_{}.bin", p.instance_id),
                    }
                }).collect(),
                track_type: match t.track_type {
                    TrackType::Audio => "audio".to_string(),
                    TrackType::Instrument => "instrument".to_string(),
                },
                output_bus: match t.output {
                    TrackOutput::Master => None,
                    TrackOutput::Bus(id) => Some(id),
                },
                instrument_type: t.instrument_type,
                instrument_icon: t.instrument_icon,
                sub_track: t.sub_track,
                input_port_index: Some(t.input_port_index),
            }
        }).collect();

        let busses = self.sorted_busses().iter().map(|b| {
            ProjectBus {
                id: b.id,
                name: b.name.clone(),
                order: b.order,
                volume: b.volume,
                pan: b.pan,
                muted: b.muted,
                plugins: b.plugins.iter().map(|p| {
                    ProjectPlugin {
                        instance_id: p.instance_id,
                        plugin_name: p.plugin_name.clone(),
                        clap_plugin_id: p.clap_plugin_id.clone(),
                        clap_file_path: p.clap_file_path.clone(),
                        state_file: format!("plugins/plugin_{}.bin", p.instance_id),
                    }
                }).collect(),
            }
        }).collect();

        let clips = self.clips.iter().map(|c| {
            ProjectClip {
                id: c.id,
                track_id: c.track_id,
                start_sample: c.start_sample,
                name: c.name.clone(),
                total_frames: c.total_frames,
                trim_start_frames: c.trim_start_frames,
                trim_end_frames: c.trim_end_frames,
                audio_file: format!("audio/clip_{}.raw", c.id),
            }
        }).collect();

        let midi_clips = self.midi_clips.iter().map(|mc| {
            ProjectMidiClip {
                id: mc.id,
                track_id: mc.track_id,
                start_sample: mc.start_sample,
                duration_ticks: mc.duration_ticks,
                name: mc.name.clone(),
                trim_start_ticks: mc.trim_start_ticks,
                trim_end_ticks: mc.trim_end_ticks,
                notes: mc.notes.iter().map(|n| {
                    ProjectMidiNote {
                        note: n.note,
                        velocity: n.velocity,
                        start_tick: n.start_tick,
                        duration_ticks: n.duration_ticks,
                    }
                }).collect(),
            }
        }).collect();

        ProjectFile {
            version: 1,
            sample_rate: self.sample_rate,
            bpm: self.bpm,
            time_sig_num: self.time_sig_num,
            time_sig_den: self.time_sig_den,
            metronome_enabled: self.metronome_enabled,
            master_volume: self.master_volume,
            punch_enabled: self.punch_enabled,
            punch_in: self.punch_in,
            punch_out: self.punch_out,
            tracks,
            clips,
            midi_clips,
            busses,
            section_definitions: self.compose.to_project_definitions(),
            section_placements: self.compose.to_project_placements(),
        }
    }

    /// Replay a loaded project into the engine and rebuild GUI state.
    pub(crate) fn replay_loaded_project(&mut self, loaded: Box<LoadedProject>) {
        let project = &loaded.file;
        self.project_path = None; // Will be set by the caller (OpenPathSelected)

        // Restore global settings
        self.bpm = project.bpm;
        self.time_sig_num = project.time_sig_num;
        self.time_sig_den = project.time_sig_den;
        self.metronome_enabled = project.metronome_enabled;
        self.master_volume = project.master_volume;
        self.punch_enabled = project.punch_enabled;
        self.punch_in = project.punch_in;
        self.punch_out = project.punch_out;
        self.playhead = 0;
        self.scroll_offset = 0.0;
        self.scroll_offset_y = 0.0;
        self.selected_clip = None;
        self.selected_plugin = None;
        self.clip_drag = None;
        self.clip_trim = None;
        self.compose
            .load_from_project(&project.section_definitions, &project.section_placements);

        self.engine.send(AudioCommand::SetBpm { bpm: self.bpm });
        self.engine.send(AudioCommand::SetTimeSignature {
            numerator: self.time_sig_num,
            denominator: self.time_sig_den,
        });
        self.engine.send(AudioCommand::SetMetronomeEnabled {
            enabled: self.metronome_enabled,
        });
        self.engine.send(AudioCommand::SetMasterVolume {
            volume: db_to_gain(self.master_volume),
        });
        self.engine.send(AudioCommand::SetPunchRange {
            enabled: self.punch_enabled,
            punch_in: self.punch_in,
            punch_out: self.punch_out,
        });

        // Clear GUI state
        self.tracks.clear();
        self.busses.clear();
        self.clips.clear();
        self.midi_clips.clear();
        self.next_track_order = 0;
        self.next_bus_order = 0;

        // Bump the app-side sub-track id counter past any persisted ids
        // so new sub-tracks allocated after this load don't collide with
        // restored ones.
        for pt in &project.tracks {
            if let Some(link) = pt.sub_track {
                let _ = link;
                if pt.id >= self.next_sub_track_id {
                    self.next_sub_track_id = pt.id + 1;
                }
            }
        }

        // Replay tracks
        for pt in &project.tracks {
            if let Some(link) = pt.sub_track {
                // Sub-tracks skip the normal AddTrack path — they're
                // registered directly via CreateSubTrack so the engine
                // knows they're fed by their parent's plugin fan-out.
                self.engine.send(AudioCommand::CreateSubTrack {
                    sub_id: pt.id,
                    parent_track_id: link.parent_track_id,
                    output_port_index: link.output_port_index,
                    name: pt.name.clone(),
                });
            } else if pt.track_type == "instrument" {
                self.engine.send(AudioCommand::AddInstrumentTrackWithId {
                    track_id: pt.id,
                    name: pt.name.clone(),
                });
            } else {
                self.engine.send(AudioCommand::AddTrackWithId {
                    track_id: pt.id,
                    name: pt.name.clone(),
                });
            }

            // Set track properties
            self.engine.send(AudioCommand::SetTrackVolume {
                track_id: pt.id,
                volume: db_to_gain(pt.volume),
            });
            self.engine.send(AudioCommand::SetTrackPan {
                track_id: pt.id,
                pan: pt.pan,
            });
            self.engine.send(AudioCommand::SetTrackMute {
                track_id: pt.id,
                muted: pt.muted,
            });
            self.engine.send(AudioCommand::SetTrackSolo {
                track_id: pt.id,
                soloed: pt.soloed,
            });
            self.engine.send(AudioCommand::SetTrackRecordArm {
                track_id: pt.id,
                armed: pt.record_armed,
            });
            self.engine.send(AudioCommand::SetTrackMonitor {
                track_id: pt.id,
                enabled: pt.monitor_enabled,
            });
            self.engine.send(AudioCommand::SetTrackMono {
                track_id: pt.id,
                mono: pt.mono,
            });
            if let Some(ref device) = pt.input_device_name {
                self.engine.send(AudioCommand::SetTrackInputDevice {
                    track_id: pt.id,
                    device_name: Some(device.clone()),
                });
            }
            if let Some(port_index) = pt.input_port_index {
                self.engine.send(AudioCommand::SetTrackInputPort {
                    track_id: pt.id,
                    port_index,
                });
            }

            // Build GUI track state
            let mut gui_plugins = Vec::new();
            for pp in &pt.plugins {
                self.engine.send(AudioCommand::AddPluginWithId {
                    track_id: pt.id,
                    instance_id: pp.instance_id,
                    clap_file_path: pp.clap_file_path.clone(),
                    clap_plugin_id: pp.clap_plugin_id.clone(),
                });
                if let Some(state_data) = loaded.plugin_states.get(&pp.instance_id) {
                    self.engine.send(AudioCommand::LoadPluginState {
                        instance_id: pp.instance_id,
                        data: state_data.clone(),
                    });
                }

                let custom = match pp.clap_plugin_id.as_str() {
                    "com.resonance.amp" => PluginCustomState::Amp(Default::default()),
                    "com.resonance.ir" => PluginCustomState::Ir(Default::default()),
                    _ => PluginCustomState::Generic,
                };
                gui_plugins.push(PluginSlotState {
                    instance_id: pp.instance_id,
                    plugin_name: pp.plugin_name.clone(),
                    clap_plugin_id: pp.clap_plugin_id.clone(),
                    clap_file_path: pp.clap_file_path.clone(),
                    params: Vec::new(), // Will be populated by PluginAdded events
                    custom,
                    has_gui: false, // overwritten by PluginAdded
                    editor_open: false,
                });
            }

            let track_type = if pt.track_type == "instrument" {
                TrackType::Instrument
            } else {
                TrackType::Audio
            };

            self.tracks.push(TrackState {
                id: pt.id,
                name: pt.name.clone(),
                volume: pt.volume,
                pan: pt.pan,
                muted: pt.muted,
                soloed: pt.soloed,
                order: self.next_track_order,
                record_armed: pt.record_armed,
                monitor_enabled: pt.monitor_enabled,
                mono: pt.mono,
                input_device_name: pt.input_device_name.clone(),
                plugins: gui_plugins,
                level_l: 0.0,
                level_r: 0.0,
                track_type,
                output: pt
                    .output_bus
                    .map(TrackOutput::Bus)
                    .unwrap_or(TrackOutput::Master),
                instrument_type: pt.instrument_type,
                instrument_icon: pt.instrument_icon,
                sub_track: pt.sub_track,
                input_port_index: pt.input_port_index.unwrap_or(0),
            });
            self.next_track_order += 1;
        }

        // Replay busses (must come before SetTrackOutput so the target
        // bus exists at the time the routing is set).
        for pb in &project.busses {
            self.engine.send(AudioCommand::AddBusWithId {
                bus_id: pb.id,
                name: pb.name.clone(),
            });
            self.engine.send(AudioCommand::SetBusVolume {
                bus_id: pb.id,
                volume: pb.volume,
            });
            self.engine.send(AudioCommand::SetBusPan {
                bus_id: pb.id,
                pan: pb.pan,
            });
            self.engine.send(AudioCommand::SetBusMute {
                bus_id: pb.id,
                muted: pb.muted,
            });

            let mut gui_plugins = Vec::new();
            for pp in &pb.plugins {
                self.engine.send(AudioCommand::AddPluginToBusWithId {
                    bus_id: pb.id,
                    instance_id: pp.instance_id,
                    clap_file_path: pp.clap_file_path.clone(),
                    clap_plugin_id: pp.clap_plugin_id.clone(),
                });
                if let Some(state_data) = loaded.plugin_states.get(&pp.instance_id) {
                    self.engine.send(AudioCommand::LoadPluginState {
                        instance_id: pp.instance_id,
                        data: state_data.clone(),
                    });
                }
                let custom = match pp.clap_plugin_id.as_str() {
                    "com.resonance.amp" => PluginCustomState::Amp(Default::default()),
                    "com.resonance.ir" => PluginCustomState::Ir(Default::default()),
                    _ => PluginCustomState::Generic,
                };
                gui_plugins.push(PluginSlotState {
                    instance_id: pp.instance_id,
                    plugin_name: pp.plugin_name.clone(),
                    clap_plugin_id: pp.clap_plugin_id.clone(),
                    clap_file_path: pp.clap_file_path.clone(),
                    params: Vec::new(),
                    custom,
                    has_gui: false,
                    editor_open: false,
                });
            }

            self.busses.push(BusState {
                id: pb.id,
                name: pb.name.clone(),
                order: self.next_bus_order,
                volume: pb.volume,
                pan: pb.pan,
                muted: pb.muted,
                plugins: gui_plugins,
                level_l: 0.0,
                level_r: 0.0,
            });
            self.next_bus_order += 1;
        }

        // Now that all busses exist, resolve track → bus routing.
        for pt in &project.tracks {
            if let Some(bus_id) = pt.output_bus {
                self.engine.send(AudioCommand::SetTrackOutput {
                    track_id: pt.id,
                    output: TrackOutput::Bus(bus_id),
                });
            }
        }

        // Replay audio clips
        for pc in &project.clips {
            if let Some(data) = loaded.audio_data.get(&pc.id) {
                self.engine.send(AudioCommand::LoadClipDirect {
                    clip_id: pc.id,
                    track_id: pc.track_id,
                    start_sample: pc.start_sample,
                    data: data.clone(),
                    name: pc.name.clone(),
                    trim_start_frames: pc.trim_start_frames,
                    trim_end_frames: pc.trim_end_frames,
                });

                let duration_samples = pc.total_frames
                    .saturating_sub(pc.trim_start_frames)
                    .saturating_sub(pc.trim_end_frames);
                self.clips.push(ClipState {
                    id: pc.id,
                    track_id: pc.track_id,
                    start_sample: pc.start_sample,
                    duration_samples,
                    name: pc.name.clone(),
                    total_frames: pc.total_frames,
                    trim_start_frames: pc.trim_start_frames,
                    trim_end_frames: pc.trim_end_frames,
                    waveform_peaks: Vec::new(), // Will be populated by ClipImported event
                });
            }
        }

        // Replay MIDI clips
        for pmc in &project.midi_clips {
            let notes: Vec<MidiNote> = pmc.notes.iter().map(|n| {
                MidiNote {
                    note: n.note,
                    velocity: n.velocity,
                    start_tick: n.start_tick,
                    duration_ticks: n.duration_ticks,
                }
            }).collect();

            self.engine.send(AudioCommand::LoadMidiClipDirect {
                clip_id: pmc.id,
                track_id: pmc.track_id,
                start_sample: pmc.start_sample,
                duration_ticks: pmc.duration_ticks,
                notes: notes.clone(),
                name: pmc.name.clone(),
                trim_start_ticks: pmc.trim_start_ticks,
                trim_end_ticks: pmc.trim_end_ticks,
            });

            self.midi_clips.push(MidiClipState {
                id: pmc.id,
                track_id: pmc.track_id,
                start_sample: pmc.start_sample,
                duration_ticks: pmc.duration_ticks,
                name: pmc.name.clone(),
                notes,
                trim_start_ticks: pmc.trim_start_ticks,
                trim_end_ticks: pmc.trim_end_ticks,
            });
        }

        self.punch_range_set = self.punch_enabled;
    }

    /// Initialize the piano roll editor state for the given MIDI clip.
    fn open_midi_editor(&mut self, clip_id: ClipId) {
        if let Some(clip) = self.midi_clips.iter().find(|c| c.id == clip_id) {
            self.editing_midi_clip = Some(MidiEditorState {
                clip_id,
                track_id: clip.track_id,
                scroll_x: 0.0,
                scroll_y: 60.0 * 5.0,
                zoom_x: 0.5,
                zoom_y: 12.0,
                snap_ticks: TICKS_PER_QUARTER_NOTE / 4,
                selected_note: None,
            });
        }
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        let tick = iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::Tick);
        let keys = keyboard::on_key_press(|key, modifiers| {
            if modifiers.command() {
                match key {
                    keyboard::Key::Character(ref c) if c.as_str() == "s" => {
                        if modifiers.shift() {
                            Some(Message::SaveProjectAs)
                        } else {
                            Some(Message::SaveProject)
                        }
                    }
                    keyboard::Key::Character(ref c) if c.as_str() == "o" => {
                        Some(Message::OpenProject)
                    }
                    _ => None,
                }
            } else {
                match key {
                    keyboard::Key::Named(keyboard::key::Named::Enter) => {
                        Some(Message::OpenSelectedMidiClip)
                    }
                    _ => None,
                }
            }
        });
        Subscription::batch([tick, keys])
    }
}
