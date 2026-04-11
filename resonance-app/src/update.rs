/// Update logic and subscription for the Resonance application.
use crate::message::*;
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

/// While the startup modal is up (no active project), swallow
/// messages that would mutate project state. Engine events don't
/// flow through `update()` (see `engine_events.rs`), so this only
/// needs to think about user-initiated variants.
fn is_gated_message(message: &Message) -> bool {
    match message {
        // Interactive user input: block.
        Message::Compose(_)
        | Message::Transport(_)
        | Message::Track(_)
        | Message::Bus(_)
        | Message::Clip(_)
        | Message::MidiClip(_)
        | Message::MidiEditor(_)
        | Message::Plugin(_)
        | Message::Viewport(_) => true,
        // Tab switches / auxiliary overlays: block so they can't
        // steal focus from the startup modal.
        Message::Ui(UiMessage::SwitchView(_))
        | Message::Ui(UiMessage::OpenSettings)
        | Message::Ui(UiMessage::OpenAddTrackMenu) => true,
        // Benign UI: allow.
        Message::Ui(UiMessage::CloseSettings)
        | Message::Ui(UiMessage::CloseAddTrackMenu)
        | Message::Ui(UiMessage::DismissError)
        | Message::Ui(UiMessage::StartNewProject) => false,
        // Project I/O drives the modal itself: always allow.
        Message::ProjectIo(_) => false,
        // Timer tick: harmless, drives VU meters — allow.
        Message::Tick => false,
    }
}

impl crate::Resonance {
    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        if !self.io.has_active_project && is_gated_message(&message) {
            return Task::none();
        }
        match message {
            Message::Compose(m) => {
                crate::update::compose::handle(self, m);
            }
            Message::Transport(TransportMessage::Play) => {
                self.engine.send(AudioCommand::Play);
                self.transport.playing = true;
            }
            Message::Transport(TransportMessage::Record) => {
                // Only meaningful when at least one track is armed; the UI
                // disables the button otherwise.
                if self.registry.tracks.iter().any(|t| t.record_armed) {
                    self.engine.send(AudioCommand::Record);
                    self.transport.playing = true;
                    // self.transport.recording flips true when RecordingStarted arrives.
                }
            }
            Message::Transport(TransportMessage::Pause) => {
                self.engine.send(AudioCommand::Pause);
                self.transport.playing = false;
            }
            Message::Transport(TransportMessage::Stop) => {
                self.engine.send(AudioCommand::Stop);
                self.transport.playing = false;
                self.transport.playhead = 0;
            }
            Message::Transport(TransportMessage::SkipBack) => {
                let skip = self.sample_rate as u64 * 5;
                let new_pos = self.transport.playhead.saturating_sub(skip);
                self.engine.send(AudioCommand::SeekTo(new_pos));
                self.transport.playhead = new_pos;
            }
            Message::Transport(TransportMessage::SkipForward) => {
                let skip = self.sample_rate as u64 * 5;
                let new_pos = self.transport.playhead + skip;
                self.engine.send(AudioCommand::SeekTo(new_pos));
                self.transport.playhead = new_pos;
            }
            Message::Transport(TransportMessage::SeekToSample(pos)) => {
                self.engine.send(AudioCommand::SeekTo(pos));
                self.transport.playhead = pos;
            }
            Message::Track(TrackMessage::AddTrack) => {
                self.engine.send(AudioCommand::AddTrack {
                    id_hint: None,
                    name: None,
                });
                self.mixer.add_track_menu_open = false;
            }
            Message::Track(TrackMessage::RemoveTrack(id)) => {
                self.engine.send(AudioCommand::RemoveTrack { track_id: id });
            }
            Message::Track(TrackMessage::SetTrackVolume(id, vol_db)) => {
                self.engine.send(AudioCommand::SetTrackVolume {
                    track_id: id,
                    volume: db_to_gain(vol_db),
                });
                self.with_track_mut(id, |t| t.volume = vol_db);
            }
            Message::Track(TrackMessage::SetTrackPan(id, pan)) => {
                self.engine
                    .send(AudioCommand::SetTrackPan { track_id: id, pan });
                self.with_track_mut(id, |t| t.pan = pan);
            }
            Message::Track(TrackMessage::SetMasterVolume(vol_db)) => {
                self.engine.send(AudioCommand::SetMasterVolume {
                    volume: db_to_gain(vol_db),
                });
                self.master_volume = vol_db;
            }
            Message::Track(TrackMessage::ToggleMute(id)) => {
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
            Message::Track(TrackMessage::ToggleSolo(id)) => {
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
            Message::Clip(ClipMessage::DeleteClip(id)) => {
                self.engine.send(AudioCommand::DeleteClip { clip_id: id });
                if self.interaction.selected_clip == Some(id) {
                    self.interaction.selected_clip = None;
                }
            }
            Message::Viewport(ViewportMessage::ZoomIn) => viewport::zoom_in(self),
            Message::Viewport(ViewportMessage::ZoomOut) => viewport::zoom_out(self),
            Message::Track(TrackMessage::ToggleMonitor(id)) => {
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
            Message::Track(TrackMessage::SetTrackName(track_id, name)) => {
                self.with_track_mut(track_id, |t| t.name = name);
            }
            Message::Track(TrackMessage::SetInstrumentType(track_id, ty)) => {
                self.with_track_mut(track_id, |t| {
                    t.instrument_type = ty;
                    t.instrument_icon = crate::state::InstrumentIcon::default_for(ty);
                });
            }
            Message::Track(TrackMessage::SetInstrumentIcon(track_id, icon)) => {
                self.with_track_mut(track_id, |t| t.instrument_icon = icon);
            }
            Message::Track(TrackMessage::ToggleTrackMono(id)) => {
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
            Message::Track(TrackMessage::ToggleRecordArm(id)) => {
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
            Message::Track(TrackMessage::SetTrackInputDevice(id, device_name)) => {
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
            Message::Track(TrackMessage::SetTrackInputPort(id, port_index)) => {
                let updated = self.with_track_mut(id, |t| t.input_port_index = port_index);
                if updated.is_some() {
                    self.engine.send(AudioCommand::SetTrackInputPort {
                        track_id: id,
                        port_index,
                    });
                }
            }
            Message::Track(TrackMessage::ToggleSubTracksVisible(id)) => {
                if !self.mixer.collapsed_sub_track_parents.insert(id) {
                    // Already present — the insert was a no-op, so toggle
                    // to the expanded state by removing.
                    self.mixer.collapsed_sub_track_parents.remove(&id);
                }
            }
            Message::Transport(TransportMessage::SetBpmText(s)) => {
                // Accept any keystroke so the user can type freely; only
                // commit on Enter via CommitBpm.
                self.transport.bpm_input = s;
            }
            Message::Transport(TransportMessage::CommitBpm) => {
                match self.transport.bpm_input.trim().parse::<f32>() {
                    Ok(parsed) => {
                        self.transport.bpm = parsed.clamp(20.0, 300.0);
                        self.engine.send(AudioCommand::SetBpm { bpm: self.transport.bpm });
                    }
                    Err(_) => {}
                }
                // Always rewrite the buffer from the current (possibly clamped
                // or reverted) BPM so the field shows a sane value.
                self.transport.bpm_input = format!("{:.0}", self.transport.bpm);
            }
            Message::Transport(TransportMessage::CyclePrecountBars) => {
                // Cycle through common pre-count lengths.
                self.transport.precount_bars = match self.transport.precount_bars {
                    0 => 1,
                    1 => 2,
                    2 => 4,
                    _ => 0,
                };
            }
            Message::Transport(TransportMessage::ToggleMetronome) => {
                self.transport.metronome_enabled = !self.transport.metronome_enabled;
                self.engine.send(AudioCommand::SetMetronomeEnabled {
                    enabled: self.transport.metronome_enabled,
                });
            }
            Message::Transport(TransportMessage::CycleTimeSignature) => {
                // Cycle through common time signatures
                let (num, den) = match (self.transport.time_sig_num, self.transport.time_sig_den) {
                    (4, 4) => (3, 4),
                    (3, 4) => (6, 8),
                    (6, 8) => (5, 4),
                    (5, 4) => (7, 8),
                    (7, 8) => (2, 4),
                    _ => (4, 4),
                };
                self.transport.time_sig_num = num;
                self.transport.time_sig_den = den;
                self.engine.send(AudioCommand::SetTimeSignature {
                    numerator: num,
                    denominator: den,
                });
            }
            Message::Plugin(PluginMessage::AddPluginToTrack(track_id, plugin)) => {
                self.engine.send(AudioCommand::AddPlugin {
                    track_id,
                    clap_file_path: plugin.clap_file_path,
                    clap_plugin_id: plugin.clap_plugin_id,
                    id_hint: None,
                });
            }
            Message::Plugin(PluginMessage::RemovePluginFromTrack(track_id, instance_id)) => {
                self.engine.send(AudioCommand::RemovePlugin {
                    track_id,
                    instance_id,
                });
            }
            Message::Plugin(PluginMessage::TogglePluginPanel(instance_id)) => {
                if self.mixer.selected_plugin == Some(instance_id) {
                    self.mixer.selected_plugin = None;
                } else {
                    self.mixer.selected_plugin = Some(instance_id);
                }
            }
            Message::Plugin(PluginMessage::SetPluginParam(instance_id, param_id, value)) => {
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
            Message::Plugin(PluginMessage::OpenPluginEditor(instance_id)) => {
                self.engine
                    .send(AudioCommand::OpenPluginEditor { instance_id });
                self.with_plugin_mut(instance_id, |p| p.editor_open = true);
            }
            Message::Plugin(PluginMessage::ClosePluginEditor(instance_id)) => {
                self.engine
                    .send(AudioCommand::ClosePluginEditor { instance_id });
                self.with_plugin_mut(instance_id, |p| p.editor_open = false);
            }
            Message::Viewport(ViewportMessage::ScrollX(delta)) => viewport::scroll_x_delta(self, delta),
            Message::Viewport(ViewportMessage::ScrollY(delta)) => viewport::scroll_y_delta(self, delta),
            Message::Ui(UiMessage::SwitchView(mode)) => {
                self.view_mode = mode;
            }
            Message::Ui(UiMessage::OpenSettings) => {
                self.mixer.settings_open = true;
            }
            Message::Ui(UiMessage::CloseSettings) => {
                self.mixer.settings_open = false;
            }
            Message::Ui(UiMessage::OpenAddTrackMenu) => {
                self.mixer.add_track_menu_open = true;
            }
            Message::Ui(UiMessage::CloseAddTrackMenu) => {
                self.mixer.add_track_menu_open = false;
            }
            Message::Ui(UiMessage::DismissError) => {
                self.error_message = None;
            }
            Message::Ui(UiMessage::StartNewProject) => {
                // Kick straight into the Save-As dialog. When it
                // returns, `SavePathSelected(Some(..))` sets the
                // path and starts saving the empty project; on
                // `ProjectSaved(Ok)` the gate lifts.
                return project_io::save_project_as_dialog();
            }
            Message::Transport(TransportMessage::ToggleLoop) => {
                self.transport.loop_enabled = !self.transport.loop_enabled;
                // Set sensible defaults if enabling with no range set
                if self.transport.loop_enabled && !self.transport.loop_range_set {
                    // Default: 2 bars from current playhead
                    let spb = self.sample_rate as f64 * 60.0 / self.transport.bpm as f64;
                    let two_bars = (spb * self.transport.time_sig_num as f64 * 2.0) as u64;
                    self.transport.loop_in = self.transport.playhead;
                    self.transport.loop_out = self.transport.playhead + two_bars;
                    self.transport.loop_range_set = true;
                }
                self.engine.send(AudioCommand::SetLoopRange {
                    enabled: self.transport.loop_enabled,
                    loop_in: self.transport.loop_in,
                    loop_out: self.transport.loop_out,
                });
            }
            Message::Transport(TransportMessage::StartLoopDrag(target)) => {
                self.transport.dragging_loop = Some(target);
            }
            Message::Transport(TransportMessage::UpdateLoopDrag(x)) => {
                if self.transport.dragging_loop.is_some() {
                    // Convert pixel x to sample position
                    let seconds = (x + self.viewport.scroll_offset) / self.viewport.zoom;
                    let sample = (seconds.max(0.0) as f64 * self.sample_rate as f64) as u64;
                    match self.transport.dragging_loop {
                        Some(LoopDragTarget::In) => {
                            self.transport.loop_in = sample;
                        }
                        Some(LoopDragTarget::Out) => {
                            self.transport.loop_out = sample;
                        }
                        None => {}
                    }
                    if self.transport.loop_enabled {
                        self.engine.send(AudioCommand::SetLoopRange {
                            enabled: true,
                            loop_in: self.transport.loop_in,
                            loop_out: self.transport.loop_out,
                        });
                    }
                }
            }
            Message::Transport(TransportMessage::EndLoopDrag) => {
                self.transport.dragging_loop = None;
                if self.transport.loop_in > self.transport.loop_out {
                    std::mem::swap(&mut self.transport.loop_in, &mut self.transport.loop_out);
                }
                if self.transport.loop_enabled {
                    self.engine.send(AudioCommand::SetLoopRange {
                        enabled: true,
                        loop_in: self.transport.loop_in,
                        loop_out: self.transport.loop_out,
                    });
                }
            }
            Message::Clip(ClipMessage::SelectClip(id)) => {
                self.interaction.selected_clip = id;
            }
            Message::Clip(ClipMessage::StartClipDrag { clip_id, grab_offset_x, start_x, start_y }) => {
                clips::start_clip_drag(self, clip_id, grab_offset_x, start_x, start_y);
            }
            Message::Clip(ClipMessage::UpdateClipDrag(x, y)) => {
                clips::update_clip_drag(self, x, y);
            }
            Message::Clip(ClipMessage::EndClipDrag) => {
                clips::end_clip_drag(self);
            }
            Message::Clip(ClipMessage::StartClipTrim { clip_id, edge, anchor_x }) => {
                clips::start_clip_trim(self, clip_id, edge, anchor_x);
            }
            Message::Clip(ClipMessage::UpdateClipTrim(x)) => {
                clips::update_clip_trim(self, x);
            }
            Message::Clip(ClipMessage::EndClipTrim) => {
                clips::end_clip_trim(self);
            }
            Message::Tick => {
                return viewport::handle_tick(self);
            }
            Message::Viewport(ViewportMessage::ViewportWidth(w)) => viewport::viewport_width(self, w),
            Message::Viewport(ViewportMessage::TimelineContentSize(w, h)) => {
                viewport::timeline_content_size(self, w, h);
            }
            Message::Viewport(ViewportMessage::ScrollToX(x)) => viewport::scroll_to_x(self, x),
            Message::Viewport(ViewportMessage::ScrollToY(y)) => viewport::scroll_to_y(self, y),
            Message::ProjectIo(ProjectIoMessage::BounceToWav) => {
                return project_io::bounce_dialog();
            }
            Message::ProjectIo(ProjectIoMessage::BouncePathSelected(Some(path))) => {
                self.io.bouncing = true;
                self.engine.send(AudioCommand::BounceToWav { path });
            }
            Message::ProjectIo(ProjectIoMessage::BouncePathSelected(None)) => {}
            Message::ProjectIo(ProjectIoMessage::SaveProject) => {
                if self.io.project_path.is_some() {
                    return project_io::start_save(self);
                } else {
                    return self.update(Message::ProjectIo(ProjectIoMessage::SaveProjectAs));
                }
            }
            Message::ProjectIo(ProjectIoMessage::SaveProjectAs) => {
                return project_io::save_project_as_dialog();
            }
            Message::ProjectIo(ProjectIoMessage::SavePathSelected(Some(path))) => {
                // Ensure path ends with .rproj
                let path = if path.ends_with(".rproj") {
                    std::path::PathBuf::from(path)
                } else {
                    std::path::PathBuf::from(format!("{path}.rproj"))
                };
                self.io.project_path = Some(path);
                return project_io::start_save(self);
            }
            Message::ProjectIo(ProjectIoMessage::SavePathSelected(None)) => {}
            Message::ProjectIo(ProjectIoMessage::OpenProject) => {
                return project_io::open_project_dialog();
            }
            Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(path))) => {
                let path = std::path::PathBuf::from(path);
                self.io.project_path = Some(path.clone());
                return project_io::load_project_task(path);
            }
            Message::ProjectIo(ProjectIoMessage::OpenPathSelected(None)) => {}
            Message::ProjectIo(ProjectIoMessage::OpenRecent(path)) => {
                self.io.project_path = Some(path.clone());
                return project_io::load_project_task(path);
            }
            Message::ProjectIo(ProjectIoMessage::ProjectSaved(Ok(()))) => {
                self.io.save_state = None;
                // First successful save of a New Project lifts the
                // gate; idempotent for normal saves.
                self.io.has_active_project = true;
                if let Some(ref path) = self.io.project_path {
                    crate::recent::add(&mut self.io.recent_projects, path);
                }
            }
            Message::ProjectIo(ProjectIoMessage::ProjectSaved(Err(e))) => {
                self.io.save_state = None;
                self.error_message = Some(format!("Save failed: {e}"));
            }
            Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(loaded))) => {
                // Stop playback, clear state, then replay
                self.engine.send(AudioCommand::Stop);
                self.transport.playing = false;
                self.transport.recording = false;
                self.io.loading = true;
                self.io.pending_load = Some(loaded);
                self.engine.send(AudioCommand::ClearAll);
                // Lift the gate immediately — replay runs when the
                // engine emits `AllCleared`, but the project is
                // logically active the moment load succeeded.
                self.io.has_active_project = true;
                if let Some(ref path) = self.io.project_path {
                    crate::recent::add(&mut self.io.recent_projects, path);
                }
            }
            Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Err(e))) => {
                self.error_message = Some(format!("Load failed: {e}"));
            }
            Message::Track(TrackMessage::AddInstrumentTrack) => {
                self.engine.send(AudioCommand::AddInstrumentTrack {
                    id_hint: None,
                    name: None,
                });
                self.mixer.add_track_menu_open = false;
            }
            Message::Bus(BusMessage::AddBus) => {
                self.engine.send(AudioCommand::AddBus {
                    id_hint: None,
                    name: None,
                });
            }
            Message::Bus(BusMessage::RemoveBus(bus_id)) => {
                self.engine.send(AudioCommand::RemoveBus { bus_id });
                // Locally clear any track routings pointing here; the engine
                // does the same. Mirrors how TrackRemoved clears refs.
                for track in &mut self.registry.tracks {
                    if track.output == TrackOutput::Bus(bus_id) {
                        track.output = TrackOutput::Master;
                    }
                }
            }
            Message::Bus(BusMessage::SetBusVolume(bus_id, vol_db)) => {
                self.engine.send(AudioCommand::SetBusVolume {
                    bus_id,
                    volume: db_to_gain(vol_db),
                });
                self.with_bus_mut(bus_id, |b| b.volume = vol_db);
            }
            Message::Bus(BusMessage::SetBusPan(bus_id, pan)) => {
                self.engine.send(AudioCommand::SetBusPan { bus_id, pan });
                self.with_bus_mut(bus_id, |b| b.pan = pan);
            }
            Message::Bus(BusMessage::ToggleBusMute(bus_id)) => {
                let new_muted = self.with_bus_mut(bus_id, |b| {
                    b.muted = !b.muted;
                    b.muted
                });
                if let Some(muted) = new_muted {
                    self.engine
                        .send(AudioCommand::SetBusMute { bus_id, muted });
                }
            }
            Message::Track(TrackMessage::SetTrackOutput(track_id, output)) => {
                self.engine
                    .send(AudioCommand::SetTrackOutput { track_id, output });
                self.with_track_mut(track_id, |t| t.output = output);
            }
            Message::Bus(BusMessage::AddPluginToBus(bus_id, plugin)) => {
                self.engine.send(AudioCommand::AddPluginToBus {
                    bus_id,
                    clap_file_path: plugin.clap_file_path,
                    clap_plugin_id: plugin.clap_plugin_id,
                    id_hint: None,
                });
            }
            Message::Bus(BusMessage::RemovePluginFromBus(bus_id, instance_id)) => {
                self.engine.send(AudioCommand::RemovePluginFromBus {
                    bus_id,
                    instance_id,
                });
            }
            Message::MidiClip(MidiClipMessage::DeleteMidiClip(id)) => {
                self.engine.send(AudioCommand::DeleteMidiClip { clip_id: id });
                if self.interaction.selected_midi_clip == Some(id) {
                    self.interaction.selected_midi_clip = None;
                }
            }
            Message::MidiClip(MidiClipMessage::StartMidiClipDrag { clip_id, grab_offset_x, start_x, start_y }) => {
                clips::start_midi_clip_drag(self, clip_id, grab_offset_x, start_x, start_y);
            }
            Message::MidiClip(MidiClipMessage::UpdateMidiClipDrag(x, y)) => {
                clips::update_midi_clip_drag(self, x, y);
            }
            Message::MidiClip(MidiClipMessage::EndMidiClipDrag) => {
                clips::end_midi_clip_drag(self);
            }
            Message::MidiClip(MidiClipMessage::StartMidiClipTrim { clip_id, edge, anchor_x }) => {
                clips::start_midi_clip_trim(self, clip_id, edge, anchor_x);
            }
            Message::MidiClip(MidiClipMessage::UpdateMidiClipTrim(x)) => {
                clips::update_midi_clip_trim(self, x);
            }
            Message::MidiClip(MidiClipMessage::EndMidiClipTrim) => {
                clips::end_midi_clip_trim(self);
            }
            Message::MidiEditor(MidiEditorMessage::OpenMidiEditor(clip_id)) => {
                clips::open_midi_editor(self, clip_id);
            }
            Message::MidiEditor(MidiEditorMessage::OpenSelectedMidiClip) => {
                if let Some(clip_id) = self.interaction.selected_midi_clip {
                    clips::open_midi_editor(self, clip_id);
                }
            }
            Message::MidiEditor(MidiEditorMessage::CloseMidiEditor) => {
                self.interaction.editing_midi_clip = None;
            }
            Message::MidiEditor(MidiEditorMessage::AddNote { clip_id, note, start_tick, duration_ticks, velocity }) => {
                self.engine.send(AudioCommand::AddMidiNote {
                    clip_id,
                    note: MidiNote { note, velocity, start_tick, duration_ticks },
                });
            }
            Message::MidiEditor(MidiEditorMessage::RemoveNote { clip_id, note_index }) => {
                self.engine.send(AudioCommand::RemoveMidiNote {
                    clip_id,
                    note_index,
                });
            }
            Message::MidiEditor(MidiEditorMessage::MoveNote { clip_id, note_index, new_start_tick, new_note }) => {
                self.engine.send(AudioCommand::MoveMidiNote {
                    clip_id,
                    note_index,
                    new_start_tick,
                    new_note,
                });
            }
            Message::MidiEditor(MidiEditorMessage::ResizeNote { clip_id, note_index, new_duration_ticks }) => {
                self.engine.send(AudioCommand::ResizeMidiNote {
                    clip_id,
                    note_index,
                    new_duration_ticks,
                });
            }
            Message::MidiEditor(MidiEditorMessage::SelectNote { note_index }) => {
                if let Some(ref mut editor) = self.interaction.editing_midi_clip {
                    editor.selected_note = note_index;
                }
            }
            Message::MidiEditor(MidiEditorMessage::PreviewNote(track_id, note)) => {
                self.engine.send(AudioCommand::SendNoteOn {
                    track_id,
                    note,
                    velocity: 0.8,
                });
            }
            Message::MidiEditor(MidiEditorMessage::StopPreview(track_id, note)) => {
                self.engine.send(AudioCommand::SendNoteOff {
                    track_id,
                    note,
                });
            }
            Message::MidiEditor(MidiEditorMessage::ScrollX(delta)) => {
                if let Some(ref mut editor) = self.interaction.editing_midi_clip {
                    editor.scroll_x = (editor.scroll_x + delta).max(0.0);
                }
            }
            Message::MidiEditor(MidiEditorMessage::ScrollY(delta)) => {
                if let Some(ref mut editor) = self.interaction.editing_midi_clip {
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
                            Some(Message::ProjectIo(ProjectIoMessage::SaveProjectAs))
                        } else {
                            Some(Message::ProjectIo(ProjectIoMessage::SaveProject))
                        }
                    }
                    keyboard::Key::Character(ref c) if c.as_str() == "o" => {
                        Some(Message::ProjectIo(ProjectIoMessage::OpenProject))
                    }
                    _ => None,
                }
            } else {
                match key {
                    keyboard::Key::Named(keyboard::key::Named::Enter) => {
                        Some(Message::MidiEditor(MidiEditorMessage::OpenSelectedMidiClip))
                    }
                    _ => None,
                }
            }
        });
        Subscription::batch([tick, keys])
    }
}
