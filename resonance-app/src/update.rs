/// Update logic and subscription for the Resonance application.
use crate::message::Message;
use crate::state::*;
use crate::theme;
use crate::util::db_to_gain;
use iced::{keyboard, Subscription, Task};
use resonance_audio::types::*;

pub mod clips;
pub mod compose;
pub mod drumroll;
pub mod project_io;
pub mod viewport;

pub(crate) use project_io::{build_project_file, replay_loaded_project};

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
                self.engine.send(AudioCommand::AddTrack {
                    id_hint: None,
                    name: None,
                });
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
                self.with_track_mut(id, |t| t.volume = vol_db);
            }
            Message::SetTrackPan(id, pan) => {
                self.engine
                    .send(AudioCommand::SetTrackPan { track_id: id, pan });
                self.with_track_mut(id, |t| t.pan = pan);
            }
            Message::SetMasterVolume(vol_db) => {
                self.engine.send(AudioCommand::SetMasterVolume {
                    volume: db_to_gain(vol_db),
                });
                self.master_volume = vol_db;
            }
            Message::ToggleMute(id) => {
                let new_muted = self.with_track_mut(id, |t| {
                    t.muted = !t.muted;
                    t.muted
                });
                if let Some(muted) = new_muted {
                    self.engine.send(AudioCommand::SetTrackMute {
                        track_id: id,
                        muted,
                    });
                }
            }
            Message::ToggleSolo(id) => {
                let new_soloed = self.with_track_mut(id, |t| {
                    t.soloed = !t.soloed;
                    t.soloed
                });
                if let Some(soloed) = new_soloed {
                    self.engine.send(AudioCommand::SetTrackSolo {
                        track_id: id,
                        soloed,
                    });
                }
            }
            Message::DeleteClip(id) => {
                self.engine.send(AudioCommand::DeleteClip { clip_id: id });
                if self.selected_clip == Some(id) {
                    self.selected_clip = None;
                }
            }
            Message::ZoomIn => viewport::zoom_in(self),
            Message::ZoomOut => viewport::zoom_out(self),
            Message::ToggleMonitor(id) => {
                let new_enabled = self.with_track_mut(id, |t| {
                    t.monitor_enabled = !t.monitor_enabled;
                    t.monitor_enabled
                });
                if let Some(enabled) = new_enabled {
                    self.engine.send(AudioCommand::SetTrackMonitor {
                        track_id: id,
                        enabled,
                    });
                }
            }
            Message::SetTrackName(track_id, name) => {
                self.with_track_mut(track_id, |t| t.name = name);
            }
            Message::SetInstrumentType(track_id, ty) => {
                self.with_track_mut(track_id, |t| {
                    t.instrument_type = ty;
                    t.instrument_icon = crate::state::InstrumentIcon::default_for(ty);
                });
            }
            Message::SetInstrumentIcon(track_id, icon) => {
                self.with_track_mut(track_id, |t| t.instrument_icon = icon);
            }
            Message::ToggleTrackMono(id) => {
                let new_mono = self.with_track_mut(id, |t| {
                    t.mono = !t.mono;
                    t.mono
                });
                if let Some(mono) = new_mono {
                    self.engine.send(AudioCommand::SetTrackMono {
                        track_id: id,
                        mono,
                    });
                }
            }
            Message::ToggleRecordArm(id) => {
                // Auto-attach default input device when arming if none set.
                let default_device = self.default_input_device_name.clone();
                let auto_device = self.with_track_mut(id, |t| {
                    t.record_armed = !t.record_armed;
                    if t.record_armed && t.input_device_name.is_none() {
                        t.input_device_name = default_device.clone();
                    }
                    (t.record_armed, t.input_device_name.clone())
                });
                if let Some((armed, device)) = auto_device {
                    if armed && device.is_some() {
                        self.engine.send(AudioCommand::SetTrackInputDevice {
                            track_id: id,
                            device_name: device,
                        });
                    }
                    self.engine.send(AudioCommand::SetTrackRecordArm {
                        track_id: id,
                        armed,
                    });
                }
            }
            Message::SetTrackInputDevice(id, device_name) => {
                // Reset port to first channel pair when device changes —
                // the old port may not exist on the new card.
                let updated = self.with_track_mut(id, |t| {
                    t.input_device_name = device_name.clone();
                    t.input_port_index = 0;
                });
                if updated.is_some() {
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
                let updated = self.with_track_mut(id, |t| t.input_port_index = port_index);
                if updated.is_some() {
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
                    id_hint: None,
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
                self.with_plugin_mut(instance_id, |p| {
                    if let Some(param) =
                        p.params.iter_mut().find(|pp| pp.id == param_id)
                    {
                        param.current_value = value;
                    }
                });
            }
            Message::OpenPluginEditor(instance_id) => {
                self.engine
                    .send(AudioCommand::OpenPluginEditor { instance_id });
                self.with_plugin_mut(instance_id, |p| p.editor_open = true);
            }
            Message::ClosePluginEditor(instance_id) => {
                self.engine
                    .send(AudioCommand::ClosePluginEditor { instance_id });
                self.with_plugin_mut(instance_id, |p| p.editor_open = false);
            }
            Message::ScrollX(delta) => viewport::scroll_x_delta(self, delta),
            Message::ScrollY(delta) => viewport::scroll_y_delta(self, delta),
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
            Message::ToggleLoop => {
                self.loop_enabled = !self.loop_enabled;
                // Set sensible defaults if enabling with no range set
                if self.loop_enabled && !self.loop_range_set {
                    // Default: 2 bars from current playhead
                    let spb = self.sample_rate as f64 * 60.0 / self.bpm as f64;
                    let two_bars = (spb * self.time_sig_num as f64 * 2.0) as u64;
                    self.loop_in = self.playhead;
                    self.loop_out = self.playhead + two_bars;
                    self.loop_range_set = true;
                }
                self.engine.send(AudioCommand::SetLoopRange {
                    enabled: self.loop_enabled,
                    loop_in: self.loop_in,
                    loop_out: self.loop_out,
                });
            }
            Message::StartLoopDrag(target) => {
                self.dragging_loop = Some(target);
            }
            Message::UpdateLoopDrag(x) => {
                if self.dragging_loop.is_some() {
                    // Convert pixel x to sample position
                    let seconds = (x + self.scroll_offset) / self.zoom;
                    let sample = (seconds.max(0.0) as f64 * self.sample_rate as f64) as u64;
                    match self.dragging_loop {
                        Some(LoopDragTarget::In) => {
                            self.loop_in = sample;
                        }
                        Some(LoopDragTarget::Out) => {
                            self.loop_out = sample;
                        }
                        None => {}
                    }
                    if self.loop_enabled {
                        self.engine.send(AudioCommand::SetLoopRange {
                            enabled: true,
                            loop_in: self.loop_in,
                            loop_out: self.loop_out,
                        });
                    }
                }
            }
            Message::EndLoopDrag => {
                self.dragging_loop = None;
                if self.loop_in > self.loop_out {
                    std::mem::swap(&mut self.loop_in, &mut self.loop_out);
                }
                if self.loop_enabled {
                    self.engine.send(AudioCommand::SetLoopRange {
                        enabled: true,
                        loop_in: self.loop_in,
                        loop_out: self.loop_out,
                    });
                }
            }
            Message::SelectClip(id) => {
                self.selected_clip = id;
            }
            Message::StartClipDrag { clip_id, grab_offset_x, start_x, start_y } => {
                clips::start_clip_drag(self, clip_id, grab_offset_x, start_x, start_y);
            }
            Message::UpdateClipDrag(x, y) => {
                clips::update_clip_drag(self, x, y);
            }
            Message::EndClipDrag => {
                clips::end_clip_drag(self);
            }
            Message::StartClipTrim { clip_id, edge, anchor_x } => {
                clips::start_clip_trim(self, clip_id, edge, anchor_x);
            }
            Message::UpdateClipTrim(x) => {
                clips::update_clip_trim(self, x);
            }
            Message::EndClipTrim => {
                clips::end_clip_trim(self);
            }
            Message::Tick => {
                return viewport::handle_tick(self);
            }
            Message::ViewportWidth(w) => viewport::viewport_width(self, w),
            Message::TimelineContentSize(w, h) => {
                viewport::timeline_content_size(self, w, h);
            }
            Message::ScrollToX(x) => viewport::scroll_to_x(self, x),
            Message::ScrollToY(y) => viewport::scroll_to_y(self, y),
            Message::BounceToWav => {
                return project_io::bounce_dialog();
            }
            Message::BouncePathSelected(Some(path)) => {
                self.bouncing = true;
                self.engine.send(AudioCommand::BounceToWav { path });
            }
            Message::BouncePathSelected(None) => {}
            Message::SaveProject => {
                if self.project_path.is_some() {
                    return project_io::start_save(self);
                } else {
                    return self.update(Message::SaveProjectAs);
                }
            }
            Message::SaveProjectAs => {
                return project_io::save_project_as_dialog();
            }
            Message::SavePathSelected(Some(path)) => {
                // Ensure path ends with .rproj
                let path = if path.ends_with(".rproj") {
                    std::path::PathBuf::from(path)
                } else {
                    std::path::PathBuf::from(format!("{path}.rproj"))
                };
                self.project_path = Some(path);
                return project_io::start_save(self);
            }
            Message::SavePathSelected(None) => {}
            Message::OpenProject => {
                return project_io::open_project_dialog();
            }
            Message::OpenPathSelected(Some(path)) => {
                let path = std::path::PathBuf::from(path);
                self.project_path = Some(path.clone());
                return project_io::load_project_task(path);
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
                self.engine.send(AudioCommand::AddInstrumentTrack {
                    id_hint: None,
                    name: None,
                });
                self.add_track_menu_open = false;
            }
            Message::AddBus => {
                self.engine.send(AudioCommand::AddBus {
                    id_hint: None,
                    name: None,
                });
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
                self.with_bus_mut(bus_id, |b| b.volume = vol_db);
            }
            Message::SetBusPan(bus_id, pan) => {
                self.engine.send(AudioCommand::SetBusPan { bus_id, pan });
                self.with_bus_mut(bus_id, |b| b.pan = pan);
            }
            Message::ToggleBusMute(bus_id) => {
                let new_muted = self.with_bus_mut(bus_id, |b| {
                    b.muted = !b.muted;
                    b.muted
                });
                if let Some(muted) = new_muted {
                    self.engine
                        .send(AudioCommand::SetBusMute { bus_id, muted });
                }
            }
            Message::SetTrackOutput(track_id, output) => {
                self.engine
                    .send(AudioCommand::SetTrackOutput { track_id, output });
                self.with_track_mut(track_id, |t| t.output = output);
            }
            Message::AddPluginToBus(bus_id, plugin) => {
                self.engine.send(AudioCommand::AddPluginToBus {
                    bus_id,
                    clap_file_path: plugin.clap_file_path,
                    clap_plugin_id: plugin.clap_plugin_id,
                    id_hint: None,
                });
            }
            Message::RemovePluginFromBus(bus_id, instance_id) => {
                self.engine.send(AudioCommand::RemovePluginFromBus {
                    bus_id,
                    instance_id,
                });
            }
            Message::DeleteMidiClip(id) => {
                self.engine.send(AudioCommand::DeleteMidiClip { clip_id: id });
                if self.selected_midi_clip == Some(id) {
                    self.selected_midi_clip = None;
                }
            }
            Message::StartMidiClipDrag { clip_id, grab_offset_x, start_x, start_y } => {
                clips::start_midi_clip_drag(self, clip_id, grab_offset_x, start_x, start_y);
            }
            Message::UpdateMidiClipDrag(x, y) => {
                clips::update_midi_clip_drag(self, x, y);
            }
            Message::EndMidiClipDrag => {
                clips::end_midi_clip_drag(self);
            }
            Message::StartMidiClipTrim { clip_id, edge, anchor_x } => {
                clips::start_midi_clip_trim(self, clip_id, edge, anchor_x);
            }
            Message::UpdateMidiClipTrim(x) => {
                clips::update_midi_clip_trim(self, x);
            }
            Message::EndMidiClipTrim => {
                clips::end_midi_clip_trim(self);
            }
            Message::OpenMidiEditor(clip_id) => {
                clips::open_midi_editor(self, clip_id);
            }
            Message::OpenSelectedMidiClip => {
                if let Some(clip_id) = self.selected_midi_clip {
                    clips::open_midi_editor(self, clip_id);
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


    pub(crate) fn subscription(&self) -> Subscription<Message> {
        let tick =
            iced::time::every(std::time::Duration::from_millis(theme::TICK_INTERVAL_MS))
                .map(|_| Message::Tick);
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
