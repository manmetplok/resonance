use iced::widget::text::Shaping;
use iced::widget::{
    button, canvas, column, container, mouse_area, opaque, pick_list, row, slider, stack, text,
    Space,
};
use iced::{alignment, Element, Font, Length, Size, Subscription};
use resonance_audio::types::*;
use resonance_audio::AudioEngine;

mod settings;
mod theme;
mod timeline;

use settings::Settings;

use timeline::TimelineCanvas;

/// Convert dB to linear gain. -60 dB or below maps to 0.0 (silence).
fn db_to_gain(db: f32) -> f32 {
    if db <= -60.0 {
        0.0
    } else {
        10.0f32.powf(db / 20.0)
    }
}

/// Application state.
struct Resonance {
    engine: AudioEngine,
    tracks: Vec<TrackState>,
    clips: Vec<ClipState>,
    playhead: u64,
    playing: bool,
    recording: bool,
    recording_start_sample: u64,
    sample_rate: u32,
    zoom: f32,           // pixels per second
    scroll_offset: f32,  // horizontal scroll in pixels
    scroll_offset_y: f32, // vertical scroll in pixels
    next_track_order: usize,
    input_devices: Vec<InputDeviceInfo>,
    default_input_device_name: Option<String>,
    bpm: f32,
    time_sig_num: u8,
    time_sig_den: u8,
    metronome_enabled: bool,
    available_plugins: Vec<ScannedPlugin>,
    settings_open: bool,
    settings: Settings,
    applied_buffer_size: u32,
    error_message: Option<String>,
    master_volume: f32,
    punch_enabled: bool,
    punch_in: u64,
    punch_out: u64,
    punch_range_set: bool,
    dragging_punch: Option<PunchDragTarget>,
    /// Pending file path to inject via CLAP state (instance_id, persist_key, path).
    pending_plugin_path: Option<(PluginInstanceId, String, String)>,
}

/// GUI-side track state.
#[derive(Debug, Clone)]
pub struct TrackState {
    pub id: TrackId,
    pub name: String,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub soloed: bool,
    pub order: usize,
    pub record_armed: bool,
    pub monitor_enabled: bool,
    pub mono: bool,
    pub input_device_name: Option<String>,
    pub plugins: Vec<PluginSlotState>,
}

/// GUI-side plugin instance state.
#[derive(Debug, Clone)]
pub struct PluginSlotState {
    pub instance_id: PluginInstanceId,
    pub plugin_name: String,
    pub clap_plugin_id: String,
    pub params: Vec<ParamInfo>,
    pub expanded: bool,
    pub custom: PluginCustomState,
}

/// Plugin-specific GUI state for bundled plugins.
#[derive(Debug, Clone)]
pub enum PluginCustomState {
    Generic,
    Drums { selected_pad: usize },
    Amp { model_name: String, file_list: Vec<String>, current_index: usize },
    Ir { ir_name: String, ir_info: String, file_list: Vec<String>, current_index: usize },
}

/// GUI-side clip state.
#[derive(Debug, Clone)]
pub struct ClipState {
    pub id: ClipId,
    pub track_id: TrackId,
    pub start_sample: SamplePos,
    pub duration_samples: u64,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PunchDragTarget {
    In,
    Out,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum Message {
    Play,
    Pause,
    Stop,
    SkipBack,
    SkipForward,
    AddTrack,
    RemoveTrack(TrackId),
    SetTrackVolume(TrackId, f32),
    SetTrackPan(TrackId, f32),
    SetMasterVolume(f32),
    ToggleMute(TrackId),
    ToggleSolo(TrackId),
    ImportFile(TrackId),
    FileSelected(TrackId, Option<String>),
    DeleteClip(ClipId),
    ZoomIn,
    ZoomOut,
    Tick,
    ToggleRecordArm(TrackId),
    ToggleMonitor(TrackId),
    ToggleTrackMono(TrackId),
    SetTrackInputDevice(TrackId, Option<String>),
    SetBpm(f32),
    ToggleMetronome,
    CycleTimeSignature,
    ScrollX(f32),
    ScrollY(f32),
    AddPluginToTrack(TrackId, ScannedPlugin),
    RemovePluginFromTrack(TrackId, PluginInstanceId),
    TogglePluginPanel(PluginInstanceId),
    SetPluginParam(PluginInstanceId, u32, f64),
    DrumPadSelect(PluginInstanceId, usize),
    PluginBrowseFile(PluginInstanceId),
    PluginFileSelected(PluginInstanceId, Option<String>),
    PluginPrevFile(PluginInstanceId),
    PluginNextFile(PluginInstanceId),
    OpenSettings,
    CloseSettings,
    SettingsSetBufferSize(u32),
    DismissError,
    TogglePunch,
    SetPunchIn(u64),
    SetPunchOut(u64),
    StartPunchDrag(PunchDragTarget),
    UpdatePunchDrag(f32),
    EndPunchDrag,
}

fn main() -> iced::Result {
    iced::application("Resonance", Resonance::update, Resonance::view)
        .subscription(Resonance::subscription)
        .theme(|_| theme::resonance_theme())
        .window_size(Size::new(1280.0, 720.0))
        .run_with(Resonance::new)
}

impl Resonance {
    fn new() -> (Self, iced::Task<Message>) {
        let settings = Settings::load();
        let engine =
            AudioEngine::new(settings.buffer_size).expect("Failed to initialize audio engine");

        // Request input device list and plugin scan on startup
        engine.send(AudioCommand::ListInputDevices);
        engine.send(AudioCommand::ScanPlugins);

        let applied_buffer_size = settings.buffer_size;

        let app = Self {
            engine,
            tracks: Vec::new(),
            clips: Vec::new(),
            playhead: 0,
            playing: false,
            recording: false,
            recording_start_sample: 0,
            sample_rate: 44100, // overwritten by SampleRateDetected event
            zoom: 100.0,
            scroll_offset: 0.0,
            scroll_offset_y: 0.0,
            next_track_order: 0,
            input_devices: Vec::new(),
            default_input_device_name: None,
            bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
            metronome_enabled: false,
            available_plugins: Vec::new(),
            settings_open: false,
            settings,
            applied_buffer_size,
            error_message: None,
            master_volume: 0.0, // 0 dB = unity gain
            punch_enabled: false,
            punch_in: 0,
            punch_out: 0,
            punch_range_set: false,
            dragging_punch: None,
            pending_plugin_path: None,
        };

        (app, iced::Task::none())
    }

    fn update(&mut self, message: Message) -> iced::Task<Message> {
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
                return iced::Task::perform(
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

                let task = iced::Task::perform(
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
                        // Build a minimal nih_plug state JSON with the path field
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
                for track in &mut self.tracks {
                    if let Some(p) = track.plugins.iter_mut().find(|p| p.instance_id == instance_id) {
                        let (file_list_len, new_idx) = match &p.custom {
                            PluginCustomState::Amp { file_list, current_index, .. } => {
                                if file_list.is_empty() { continue; }
                                let new = if *current_index == 0 { file_list.len() - 1 } else { current_index - 1 };
                                (file_list.len(), new)
                            }
                            PluginCustomState::Ir { file_list, current_index, .. } => {
                                if file_list.is_empty() { continue; }
                                let new = if *current_index == 0 { file_list.len() - 1 } else { current_index - 1 };
                                (file_list.len(), new)
                            }
                            _ => continue,
                        };
                        let _ = file_list_len;
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
            Message::PluginNextFile(instance_id) => {
                for track in &mut self.tracks {
                    if let Some(p) = track.plugins.iter_mut().find(|p| p.instance_id == instance_id) {
                        let new_idx = match &p.custom {
                            PluginCustomState::Amp { file_list, current_index, .. } => {
                                if file_list.is_empty() { continue; }
                                if *current_index >= file_list.len() - 1 { 0 } else { current_index + 1 }
                            }
                            PluginCustomState::Ir { file_list, current_index, .. } => {
                                if file_list.is_empty() { continue; }
                                if *current_index >= file_list.len() - 1 { 0 } else { current_index + 1 }
                            }
                            _ => continue,
                        };
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
            Message::ScrollX(delta) => {
                self.scroll_offset = (self.scroll_offset + delta).max(0.0);
            }
            Message::ScrollY(delta) => {
                self.scroll_offset_y = (self.scroll_offset_y + delta).max(0.0);
                // Clamp to max content height
                let max_y = (self.tracks.len() as f32 * theme::TRACK_HEIGHT).max(0.0);
                self.scroll_offset_y = self.scroll_offset_y.min(max_y);
            }
            Message::OpenSettings => {
                self.settings_open = true;
            }
            Message::CloseSettings => {
                self.settings_open = false;
            }
            Message::SettingsSetBufferSize(size) => {
                self.settings.buffer_size = size;
                self.settings.save();
                match self.engine.set_buffer_size(size) {
                    Ok(()) => {
                        self.applied_buffer_size = size;
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Failed to apply buffer size: {}", e));
                    }
                }
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
            Message::Tick => {
                while let Some(event) = self.engine.try_recv() {
                    self.handle_engine_event(event);
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
        iced::Task::none()
    }

    fn handle_engine_event(&mut self, event: AudioEvent) {
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
            } => {
                self.clips.push(ClipState {
                    id: clip_id,
                    track_id,
                    start_sample,
                    duration_samples,
                    name,
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
            } => {
                self.clips.push(ClipState {
                    id: clip_id,
                    track_id,
                    start_sample,
                    duration_samples,
                    name,
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
                    "com.resonance.drums" => PluginCustomState::Drums { selected_pad: 0 },
                    "com.resonance.amp" => PluginCustomState::Amp {
                        model_name: String::new(),
                        file_list: Vec::new(),
                        current_index: 0,
                    },
                    "com.resonance.ir" => PluginCustomState::Ir {
                        ir_name: String::new(),
                        ir_info: String::new(),
                        file_list: Vec::new(),
                        current_index: 0,
                    },
                    _ => PluginCustomState::Generic,
                };
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
                    track.plugins.push(PluginSlotState {
                        instance_id,
                        plugin_name,
                        clap_plugin_id,
                        params,
                        expanded: false,
                        custom,
                    });
                }
            }
            AudioEvent::PluginRemoved {
                track_id,
                instance_id,
            } => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
                    track.plugins.retain(|p| p.instance_id != instance_id);
                }
            }
            AudioEvent::PluginsScanned { plugins } => {
                self.available_plugins = plugins;
            }
            AudioEvent::PluginStateSaved { instance_id, data } => {
                // If we have a pending path to inject, modify the state and reload
                if let Some((pending_id, ref key, ref path)) = self.pending_plugin_path.clone() {
                    if pending_id == instance_id {
                        // NIH-plug's CLAP state format prepends an 8-byte length (u64 LE)
                        // before the JSON payload. We must strip it before parsing and
                        // re-add it after modification.
                        if data.len() > 8 {
                            let json_data = &data[8..];
                            if let Ok(mut state) = serde_json::from_slice::<serde_json::Value>(json_data) {
                                if let Some(fields) = state.get_mut("fields") {
                                    // Double-serialize: the value must be a JSON string of the path
                                    if let Ok(serialized) = serde_json::to_string(path.as_str()) {
                                        fields[&key] = serde_json::Value::String(serialized);
                                    }
                                }
                                if let Ok(new_json) = serde_json::to_vec(&state) {
                                    let mut new_data = (new_json.len() as u64).to_le_bytes().to_vec();
                                    new_data.extend_from_slice(&new_json);
                                    self.engine.send(AudioCommand::LoadPluginState {
                                        instance_id,
                                        data: new_data,
                                    });
                                }
                            }
                        }
                        self.pending_plugin_path = None;
                    }
                }
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::Tick)
    }

    fn view(&self) -> Element<'_, Message> {
        let transport = self.view_transport();
        let main_area = self.view_main_area();

        let content: Element<'_, Message> = if let Some(ref err) = self.error_message {
            let error_bar = container(
                row![
                    text(err).size(13).color(iced::Color::WHITE),
                    Space::with_width(Length::Fill),
                    button(text("\u{00d7}").size(14).color(iced::Color::WHITE))
                        .on_press(Message::DismissError)
                        .style(|_theme, _status| iced::widget::button::Style {
                            background: Some(iced::Background::Color(iced::Color::TRANSPARENT)),
                            text_color: iced::Color::WHITE,
                            ..Default::default()
                        })
                ]
                .spacing(8)
                .align_y(alignment::Vertical::Center)
                .padding(8),
            )
            .width(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::RECORD_RED)),
                ..Default::default()
            });
            column![transport, error_bar, main_area].spacing(0).into()
        } else {
            column![transport, main_area].spacing(0).into()
        };

        let base: Element<'_, Message> = container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::BG)),
                ..Default::default()
            })
            .into();

        if self.settings_open {
            stack![base, self.view_settings_overlay()].into()
        } else {
            base
        }
    }

    fn view_transport(&self) -> Element<'_, Message> {
        let tempo = TempoMap {
            bpm: self.bpm,
            numerator: self.time_sig_num,
            denominator: self.time_sig_den,
            metronome_enabled: self.metronome_enabled,
        };
        let bar_beat_str = tempo.format_position(self.playhead, self.sample_rate);

        let play_pause = if self.playing {
            button(text("⏸").size(18).color(theme::TEXT).shaping(Shaping::Advanced))
                .on_press(Message::Pause)
                .style(|_theme, status| theme::transport_button_style(status))
        } else {
            button(text("▶").size(18).color(theme::ACCENT).shaping(Shaping::Advanced))
                .on_press(Message::Play)
                .style(|_theme, status| theme::transport_button_style(status))
        };

        let stop_btn = button(text("⏹").size(18).color(theme::TEXT).shaping(Shaping::Advanced))
            .on_press(Message::Stop)
            .style(|_theme, status| theme::transport_button_style(status));

        let skip_back = button(text("⏪").size(16).color(theme::TEXT).shaping(Shaping::Advanced))
            .on_press(Message::SkipBack)
            .style(|_theme, status| theme::transport_button_style(status));

        let skip_fwd = button(text("⏩").size(16).color(theme::TEXT).shaping(Shaping::Advanced))
            .on_press(Message::SkipForward)
            .style(|_theme, status| theme::transport_button_style(status));

        let time_display = text(bar_beat_str)
            .size(20)
            .font(Font::MONOSPACE)
            .color(theme::ACCENT);

        // BPM slider and display
        let bpm_slider = slider(20.0..=300.0, self.bpm, Message::SetBpm)
            .width(80)
            .step(1.0);
        let bpm_text = text(format!("{:.0}", self.bpm))
            .size(14)
            .font(Font::MONOSPACE)
            .color(theme::TEXT);
        let bpm_label = text("BPM").size(10).color(theme::TEXT_DIM);

        // Time signature button
        let time_sig_str = format!("{}/{}", self.time_sig_num, self.time_sig_den);
        let time_sig_btn = button(text(time_sig_str).size(14).font(Font::MONOSPACE).color(theme::TEXT))
            .on_press(Message::CycleTimeSignature)
            .style(|_theme, status| theme::transport_button_style(status));

        // Metronome toggle
        let met_color = if self.metronome_enabled {
            theme::METRONOME_ON
        } else {
            theme::TEXT_DIM
        };
        let metronome_enabled = self.metronome_enabled;
        let met_btn = button(text("Met").size(12).color(met_color))
            .on_press(Message::ToggleMetronome)
            .style(move |_theme, status| {
                if metronome_enabled {
                    let bg = match status {
                        iced::widget::button::Status::Hovered => iced::Color::from_rgb(0.15, 0.25, 0.15),
                        iced::widget::button::Status::Pressed => iced::Color::from_rgb(0.10, 0.20, 0.10),
                        _ => iced::Color::from_rgb(0.12, 0.20, 0.12),
                    };
                    iced::widget::button::Style {
                        background: Some(iced::Background::Color(bg)),
                        text_color: theme::METRONOME_ON,
                        border: iced::Border {
                            color: theme::METRONOME_ON,
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        ..Default::default()
                    }
                } else {
                    theme::transport_button_style(status)
                }
            });

        // Recording indicator
        let rec_indicator = if self.recording {
            text("● REC").size(14).color(theme::RECORD_RED)
        } else {
            text("").size(14)
        };

        let zoom_out = button(text("−").size(16).color(theme::TEXT))
            .on_press(Message::ZoomOut)
            .style(|_theme, status| theme::transport_button_style(status));

        let zoom_in = button(text("+").size(16).color(theme::TEXT))
            .on_press(Message::ZoomIn)
            .style(|_theme, status| theme::transport_button_style(status));

        let add_track = button(text("+ Track").size(14).color(theme::TEXT))
            .on_press(Message::AddTrack)
            .style(|_theme, status| theme::transport_button_style(status));

        let settings_btn = button(text("\u{2699}").size(16).color(theme::TEXT))
            .on_press(Message::OpenSettings)
            .style(|_theme, status| theme::transport_button_style(status));

        let punch_color = if self.punch_enabled {
            theme::PUNCH_MARKER
        } else {
            theme::TEXT_DIM
        };
        let punch_enabled = self.punch_enabled;
        let punch_btn = button(text("P").size(12).color(punch_color))
            .on_press(Message::TogglePunch)
            .style(move |_theme, status| {
                if punch_enabled {
                    let bg = match status {
                        iced::widget::button::Status::Hovered => {
                            iced::Color::from_rgb(0.25, 0.20, 0.10)
                        }
                        iced::widget::button::Status::Pressed => {
                            iced::Color::from_rgb(0.20, 0.15, 0.08)
                        }
                        _ => iced::Color::from_rgb(0.22, 0.18, 0.08),
                    };
                    iced::widget::button::Style {
                        background: Some(iced::Background::Color(bg)),
                        text_color: theme::PUNCH_MARKER,
                        border: iced::Border {
                            color: theme::PUNCH_MARKER,
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        ..Default::default()
                    }
                } else {
                    theme::transport_button_style(status)
                }
            });

        let master_slider = slider(-60.0..=6.0f32, self.master_volume, Message::SetMasterVolume)
            .width(80)
            .step(0.1);
        let master_vol_label = if self.master_volume <= -60.0 {
            "-inf".to_string()
        } else {
            format!("{:.1}", self.master_volume)
        };

        let transport_row = row![
            Space::with_width(10),
            skip_back,
            stop_btn,
            play_pause,
            skip_fwd,
            Space::with_width(16),
            time_display,
            Space::with_width(6),
            rec_indicator,
            Space::with_width(16),
            bpm_slider,
            bpm_text,
            bpm_label,
            Space::with_width(8),
            time_sig_btn,
            Space::with_width(4),
            met_btn,
            Space::with_width(4),
            punch_btn,
            Space::with_width(Length::Fill),
            text("Master").size(10).color(theme::TEXT_DIM),
            master_slider,
            text(master_vol_label).size(11).font(Font::MONOSPACE).color(theme::TEXT_DIM),
            Space::with_width(12),
            zoom_out,
            text("Zoom").size(12).color(theme::TEXT_DIM),
            zoom_in,
            Space::with_width(20),
            add_track,
            Space::with_width(6),
            settings_btn,
            Space::with_width(10),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center)
        .height(48);

        container(transport_row)
            .width(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::PANEL)),
                border: iced::Border {
                    color: theme::SEPARATOR,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    fn view_settings_overlay(&self) -> Element<'_, Message> {
        let backdrop = mouse_area(
            container(Space::new(Length::Fill, Length::Fill))
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_theme| container::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgba(
                        0.0, 0.0, 0.0, 0.6,
                    ))),
                    ..Default::default()
                }),
        )
        .on_press(Message::CloseSettings);

        let title = text("Settings").size(20).color(theme::ACCENT);

        let buf_label = text("Buffer Size").size(14).color(theme::TEXT);
        let buf_options: Vec<u32> = settings::BUFFER_SIZE_OPTIONS.to_vec();
        let buf_picker = pick_list(buf_options, Some(self.settings.buffer_size), |size| {
            Message::SettingsSetBufferSize(size)
        })
        .text_size(14)
        .width(120);

        let buf_row = row![buf_label, Space::with_width(Length::Fill), buf_picker]
            .spacing(8)
            .align_y(alignment::Vertical::Center);

        let close_btn = button(text("Close").size(14).color(theme::TEXT))
            .on_press(Message::CloseSettings)
            .style(|_theme, status| theme::transport_button_style(status));

        let dialog_content = column![
            title,
            Space::with_height(16),
            buf_row,
            Space::with_height(20),
            close_btn,
        ]
        .spacing(8)
        .padding(24)
        .width(360);

        let dialog = container(dialog_content).style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::PANEL)),
            border: iced::Border {
                color: theme::SEPARATOR,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        });

        let centered = container(opaque(dialog))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill);

        stack![backdrop, centered].into()
    }

    fn view_main_area(&self) -> Element<'_, Message> {
        let track_headers = self.view_track_headers();
        let timeline = self.view_timeline();

        let main = row![track_headers, timeline];

        container(main)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_track_headers(&self) -> Element<'_, Message> {
        let mut headers = column![].spacing(0);

        // Ruler header spacer
        headers = headers.push(
            container(Space::new(Length::Fill, 30)).style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::PANEL_DARK)),
                ..Default::default()
            }),
        );

        let mut sorted_tracks: Vec<&TrackState> = self.tracks.iter().collect();
        sorted_tracks.sort_by_key(|t| t.order);

        // Calculate which tracks are visible given scroll_offset_y
        let visible_start = self.scroll_offset_y / theme::TRACK_HEIGHT;
        let first_visible = visible_start.floor() as usize;
        // Add top padding for the scrolled-away portion
        let top_pad = first_visible as f32 * theme::TRACK_HEIGHT - self.scroll_offset_y;
        if first_visible > 0 {
            headers = headers.push(Space::new(Length::Fill, (first_visible as f32 * theme::TRACK_HEIGHT - self.scroll_offset_y).max(0.0)));
        } else if self.scroll_offset_y > 0.0 {
            // Partial first track: use negative-ish padding — just skip offset
            headers = headers.push(Space::new(Length::Fill, top_pad.max(0.0)));
        }

        for (i, track) in sorted_tracks.iter().enumerate() {
            if i < first_visible {
                continue;
            }
            let header = self.view_track_header(track);
            headers = headers.push(header);
        }

        container(headers)
            .width(180)
            .height(Length::Fill)
            .clip(true)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::PANEL)),
                border: iced::Border {
                    color: theme::SEPARATOR,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    fn view_track_header(&self, track: &TrackState) -> Element<'_, Message> {
        let name = text(track.name.clone()).size(13).color(theme::TEXT);

        // Record arm button
        let rec_color = if track.record_armed {
            theme::RECORD_RED
        } else {
            theme::TEXT_DIM
        };
        let armed = track.record_armed;
        let rec_btn = button(text("R").size(11).color(rec_color))
            .on_press(Message::ToggleRecordArm(track.id))
            .style(move |_theme, status| {
                if armed {
                    theme::record_armed_button_style(status)
                } else {
                    theme::small_button_style(status)
                }
            })
            .padding(2);

        let mute_color = if track.muted {
            theme::ACCENT
        } else {
            theme::TEXT_DIM
        };
        let mute_btn = button(text("M").size(11).color(mute_color))
            .on_press(Message::ToggleMute(track.id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        let solo_color = if track.soloed {
            theme::SOLO_YELLOW
        } else {
            theme::TEXT_DIM
        };
        let solo_btn = button(text("S").size(11).color(solo_color))
            .on_press(Message::ToggleSolo(track.id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        let vol_slider = slider(-60.0..=6.0f32, track.volume, {
            let id = track.id;
            move |v| Message::SetTrackVolume(id, v)
        })
        .width(80)
        .step(0.1);

        let vol_label = if track.volume <= -60.0 {
            "-inf".to_string()
        } else {
            format!("{:.1}", track.volume)
        };
        let vol_text = text(vol_label)
            .size(11)
            .font(Font::MONOSPACE)
            .color(theme::TEXT_DIM);

        let pan_slider = slider(-1.0..=1.0f32, track.pan, {
            let id = track.id;
            move |v| Message::SetTrackPan(id, v)
        })
        .width(50)
        .step(0.01);

        let pan_label = if track.pan.abs() < 0.01 {
            "C".to_string()
        } else if track.pan < 0.0 {
            format!("L{:.0}", -track.pan * 100.0)
        } else {
            format!("R{:.0}", track.pan * 100.0)
        };
        let pan_text = text(pan_label)
            .size(11)
            .font(Font::MONOSPACE)
            .color(theme::TEXT_DIM);

        let import_btn = button(text("+").size(12).color(theme::TEXT))
            .on_press(Message::ImportFile(track.id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        let del_btn = button(text("×").size(12).color(theme::TEXT_DIM))
            .on_press(Message::RemoveTrack(track.id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        // Monitor button
        let mon_color = if track.monitor_enabled {
            theme::METRONOME_ON
        } else {
            theme::TEXT_DIM
        };
        let mon_enabled = track.monitor_enabled;
        let mon_btn = button(text("I").size(11).color(mon_color))
            .on_press(Message::ToggleMonitor(track.id))
            .style(move |_theme, status| {
                if mon_enabled {
                    let bg = match status {
                        iced::widget::button::Status::Hovered => iced::Color::from_rgb(0.15, 0.25, 0.15),
                        iced::widget::button::Status::Pressed => iced::Color::from_rgb(0.10, 0.20, 0.10),
                        _ => iced::Color::from_rgb(0.12, 0.20, 0.12),
                    };
                    iced::widget::button::Style {
                        background: Some(iced::Background::Color(bg)),
                        text_color: theme::METRONOME_ON,
                        border: iced::Border {
                            color: theme::METRONOME_ON,
                            width: 1.0,
                            radius: 2.0.into(),
                        },
                        ..Default::default()
                    }
                } else {
                    theme::small_button_style(status)
                }
            })
            .padding(2);

        // Mono/Stereo toggle
        let mono_label = if track.mono { "M" } else { "S" };
        let mono_color = theme::TEXT;
        let is_mono = track.mono;
        let mono_btn = button(text(mono_label).size(11).color(mono_color))
            .on_press(Message::ToggleTrackMono(track.id))
            .style(move |_theme, status| {
                let bg = match status {
                    iced::widget::button::Status::Hovered => iced::Color::from_rgb(0.20, 0.20, 0.25),
                    iced::widget::button::Status::Pressed => iced::Color::from_rgb(0.15, 0.15, 0.20),
                    _ => iced::Color::from_rgb(0.18, 0.18, 0.22),
                };
                iced::widget::button::Style {
                    background: Some(iced::Background::Color(bg)),
                    text_color: if is_mono { theme::TEXT } else { theme::ACCENT },
                    border: iced::Border {
                        color: if is_mono { theme::SEPARATOR } else { theme::ACCENT },
                        width: 1.0,
                        radius: 2.0.into(),
                    },
                    ..Default::default()
                }
            })
            .padding(2);

        let top_row = row![
            name,
            Space::with_width(Length::Fill),
            mono_btn,
            mon_btn,
            rec_btn,
            mute_btn,
            solo_btn,
            import_btn,
            del_btn
        ]
        .spacing(4)
        .align_y(alignment::Vertical::Center);

        let bottom_row = row![vol_slider, vol_text, pan_slider, pan_text]
            .spacing(4)
            .align_y(alignment::Vertical::Center);

        let mut content = column![top_row, bottom_row].spacing(2).padding(6);

        // Clip entries with delete buttons
        let track_clips: Vec<&ClipState> = self.clips.iter().filter(|c| c.track_id == track.id).collect();
        for clip in &track_clips {
            let clip_name: String = if clip.name.chars().count() > 12 {
                let mut s: String = clip.name.chars().take(10).collect();
                s.push_str("..");
                s
            } else {
                clip.name.clone()
            };
            let clip_del = button(text("×").size(9).color(theme::TEXT_DIM))
                .on_press(Message::DeleteClip(clip.id))
                .style(|_theme, status| theme::small_button_style(status))
                .padding(1);
            let clip_row = row![
                text(clip_name).size(9).color(theme::TEXT_DIM),
                Space::with_width(Length::Fill),
                clip_del,
            ]
            .spacing(2)
            .align_y(alignment::Vertical::Center);
            content = content.push(clip_row);
        }

        // Plugin chain with clickable names, remove buttons, and expandable params
        for plugin in &track.plugins {
            let pname: String = if plugin.plugin_name.chars().count() > 12 {
                let mut s: String = plugin.plugin_name.chars().take(10).collect();
                s.push_str("..");
                s
            } else {
                plugin.plugin_name.clone()
            };
            let track_id = track.id;
            let pid = plugin.instance_id;
            let expanded = plugin.expanded;

            // Clickable plugin name
            let name_color = if expanded { theme::TEXT } else { theme::ACCENT };
            let name_btn = button(text(pname).size(9).color(name_color))
                .on_press(Message::TogglePluginPanel(pid))
                .style(|_theme, status| theme::small_button_style(status))
                .padding(1);

            let plugin_del = button(text("×").size(9).color(theme::TEXT_DIM))
                .on_press(Message::RemovePluginFromTrack(track_id, pid))
                .style(|_theme, status| theme::small_button_style(status))
                .padding(1);
            let plugin_row = row![
                name_btn,
                Space::with_width(Length::Fill),
                plugin_del,
            ]
            .spacing(2)
            .align_y(alignment::Vertical::Center);
            content = content.push(plugin_row);

            // Expandable parameter panel — custom views for bundled plugins
            if expanded {
                match &plugin.custom {
                    PluginCustomState::Drums { selected_pad } => {
                        content = content.push(self.view_drums_panel(plugin, *selected_pad));
                    }
                    PluginCustomState::Amp { model_name, file_list, current_index } => {
                        content = content.push(self.view_amp_panel(plugin, model_name, file_list.len(), *current_index));
                    }
                    PluginCustomState::Ir { ir_name, ir_info, file_list, current_index } => {
                        content = content.push(self.view_ir_panel(plugin, ir_name, ir_info, file_list.len(), *current_index));
                    }
                    PluginCustomState::Generic => {
                        for param in &plugin.params {
                            let param_id = param.id;
                            let inst_id = pid;
                            let range = param.min_value..=param.max_value;
                            let param_slider = slider(
                                range,
                                param.current_value,
                                move |v| Message::SetPluginParam(inst_id, param_id, v),
                            )
                            .width(Length::Fill)
                            .step(0.001);

                            let param_label = text(param.name.clone()).size(8).color(theme::TEXT_DIM);
                            let param_value_text = text(format!("{:.2}", param.current_value))
                                .size(8)
                                .font(Font::MONOSPACE)
                                .color(theme::TEXT_DIM);

                            let param_row = column![
                                row![param_label, Space::with_width(Length::Fill), param_value_text]
                                    .spacing(2),
                                param_slider,
                            ]
                            .spacing(1);
                            content = content.push(param_row);
                        }
                    }
                }
            }
        }

        if !self.available_plugins.is_empty() {
            let track_id = track.id;
            let fx_picker = pick_list(
                self.available_plugins.clone(),
                None::<ScannedPlugin>,
                move |plugin: ScannedPlugin| Message::AddPluginToTrack(track_id, plugin),
            )
            .placeholder("+ FX")
            .text_size(10)
            .width(Length::Fill);
            content = content.push(fx_picker);
        }

        // Input device picker (shown when track is record-armed)
        if track.record_armed && !self.input_devices.is_empty() {
            let selected = track
                .input_device_name
                .as_ref()
                .and_then(|name| self.input_devices.iter().find(|d| &d.name == name))
                .cloned();

            let track_id = track.id;
            let device_picker = pick_list(
                self.input_devices.clone(),
                selected,
                move |device: InputDeviceInfo| {
                    Message::SetTrackInputDevice(track_id, Some(device.name))
                },
            )
            .placeholder("Select input...")
            .text_size(10)
            .width(Length::Fill);

            content = content.push(device_picker);
        }

        let bg = if track.record_armed {
            theme::PANEL_ARMED
        } else {
            theme::PANEL_DARK
        };
        let border_color = if track.record_armed {
            theme::RECORD_RED
        } else {
            theme::SEPARATOR
        };

        container(content)
            .width(Length::Fill)
            .height(Length::Shrink)
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(bg)),
                border: iced::Border {
                    color: border_color,
                    width: 0.5,
                    radius: 0.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    fn view_timeline(&self) -> Element<'_, Message> {
        let recording_tracks: Vec<TrackId> = if self.recording {
            self.tracks
                .iter()
                .filter(|t| t.record_armed)
                .map(|t| t.id)
                .collect()
        } else {
            Vec::new()
        };

        let timeline_data = TimelineCanvas {
            tracks: &self.tracks,
            clips: &self.clips,
            playhead: self.playhead,
            sample_rate: self.sample_rate,
            zoom: self.zoom,
            scroll_offset: self.scroll_offset,
            recording_tracks,
            recording_start_sample: self.recording_start_sample,
            bpm: self.bpm,
            time_sig_num: self.time_sig_num,
            scroll_offset_y: self.scroll_offset_y,
            punch_enabled: self.punch_enabled,
            punch_in: self.punch_in,
            punch_out: self.punch_out,
        };

        canvas(timeline_data)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    // -----------------------------------------------------------------------
    // Custom plugin views
    // -----------------------------------------------------------------------

    fn view_drums_panel<'a>(&self, plugin: &PluginSlotState, selected_pad: usize) -> Element<'a, Message> {
        let pid = plugin.instance_id;
        let pad_names = [
            "Kick", "Snare", "HH Close", "HH Open",
            "Tom Hi", "Tom Mid", "Tom Low", "Crash",
            "Ride", "Rimshot", "Clap", "Cowbell",
        ];

        // 4x3 pad grid
        let mut grid = column![].spacing(2);
        for row_idx in 0..3 {
            let mut grid_row = row![].spacing(2);
            for col_idx in 0..4 {
                let pad_idx = row_idx * 4 + col_idx;
                let is_selected = pad_idx == selected_pad;
                let name = pad_names[pad_idx];
                let bg = if is_selected {
                    iced::Color::from_rgb(0.25, 0.3, 0.45)
                } else {
                    iced::Color::from_rgb(0.2, 0.2, 0.24)
                };
                let border_color = if is_selected {
                    iced::Color::from_rgb(0.4, 0.5, 0.8)
                } else {
                    iced::Color::from_rgb(0.3, 0.3, 0.35)
                };
                let pad_btn = button(
                    container(text(name).size(7).color(theme::TEXT))
                        .center_x(Length::Fill)
                        .center_y(Length::Fill),
                )
                .on_press(Message::DrumPadSelect(pid, pad_idx))
                .width(Length::Fill)
                .height(28)
                .style(move |_theme, _status| iced::widget::button::Style {
                    background: Some(iced::Background::Color(bg)),
                    text_color: theme::TEXT,
                    border: iced::Border {
                        color: border_color,
                        width: if is_selected { 1.5 } else { 0.5 },
                        radius: 3.0.into(),
                    },
                    ..Default::default()
                });
                grid_row = grid_row.push(pad_btn);
            }
            grid = grid.push(grid_row);
        }

        // Per-pad controls: find volume/pan params for selected pad
        // Params are nested as "Pad X > Volume", "Pad X > Pan"
        let pad_prefix = format!("Pad {} > ", selected_pad + 1);
        let mut pad_controls = column![
            text(format!("Pad: {}", pad_names[selected_pad])).size(8).color(theme::TEXT)
        ].spacing(1);

        for param in &plugin.params {
            if param.name.starts_with(&pad_prefix) || param.name == "Master Volume" {
                let param_id = param.id;
                let inst_id = pid;
                let label = if param.name == "Master Volume" {
                    "Master".to_string()
                } else {
                    param.name.strip_prefix(&pad_prefix).unwrap_or(&param.name).to_string()
                };
                let range = param.min_value..=param.max_value;
                let param_slider = slider(range, param.current_value, move |v| {
                    Message::SetPluginParam(inst_id, param_id, v)
                })
                .width(Length::Fill)
                .step(0.001);
                pad_controls = pad_controls.push(
                    column![
                        text(label).size(7).color(theme::TEXT_DIM),
                        param_slider,
                    ].spacing(0)
                );
            }
        }

        column![grid, pad_controls].spacing(4).into()
    }

    fn view_amp_panel<'a>(&self, plugin: &PluginSlotState, model_name: &str, file_count: usize, current_index: usize) -> Element<'a, Message> {
        let pid = plugin.instance_id;

        let display = if model_name.is_empty() {
            "No model loaded".to_string()
        } else {
            model_name.to_string()
        };
        let name_text = text(display).size(8).color(theme::TEXT);

        let count_text = if file_count > 0 {
            text(format!("{}/{}", current_index + 1, file_count)).size(7).color(theme::TEXT_DIM)
        } else {
            text("").size(7)
        };

        let prev_btn = button(text("<").size(8).color(theme::TEXT))
            .on_press(Message::PluginPrevFile(pid))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(1)
            .width(20);

        let browse_btn = button(text("Browse").size(7).color(theme::TEXT))
            .on_press(Message::PluginBrowseFile(pid))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(1);

        let next_btn = button(text(">").size(8).color(theme::TEXT))
            .on_press(Message::PluginNextFile(pid))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(1)
            .width(20);

        let nav_row = row![prev_btn, browse_btn, next_btn]
            .spacing(2)
            .align_y(alignment::Vertical::Center);

        // Gain sliders
        let mut controls = column![name_text, count_text, nav_row].spacing(2);
        for param in &plugin.params {
            if param.name == "Input Gain" || param.name == "Output Gain" {
                let param_id = param.id;
                let inst_id = pid;
                let range = param.min_value..=param.max_value;
                let param_slider = slider(range, param.current_value, move |v| {
                    Message::SetPluginParam(inst_id, param_id, v)
                })
                .width(Length::Fill)
                .step(0.001);
                controls = controls.push(
                    column![
                        text(param.name.clone()).size(7).color(theme::TEXT_DIM),
                        param_slider,
                    ].spacing(0)
                );
            }
        }

        controls.into()
    }

    fn view_ir_panel<'a>(&self, plugin: &PluginSlotState, ir_name: &str, ir_info: &str, file_count: usize, current_index: usize) -> Element<'a, Message> {
        let pid = plugin.instance_id;

        let display = if ir_name.is_empty() {
            "No IR loaded".to_string()
        } else {
            ir_name.to_string()
        };
        let name_text = text(display).size(8).color(theme::TEXT);
        let info_text = text(ir_info.to_string()).size(7).color(theme::TEXT_DIM);

        let count_text = if file_count > 0 {
            text(format!("{}/{}", current_index + 1, file_count)).size(7).color(theme::TEXT_DIM)
        } else {
            text("").size(7)
        };

        let prev_btn = button(text("<").size(8).color(theme::TEXT))
            .on_press(Message::PluginPrevFile(pid))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(1)
            .width(20);

        let browse_btn = button(text("Browse").size(7).color(theme::TEXT))
            .on_press(Message::PluginBrowseFile(pid))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(1);

        let next_btn = button(text(">").size(8).color(theme::TEXT))
            .on_press(Message::PluginNextFile(pid))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(1)
            .width(20);

        let nav_row = row![prev_btn, browse_btn, next_btn]
            .spacing(2)
            .align_y(alignment::Vertical::Center);

        // Dry/Wet and Output Gain sliders
        let mut controls = column![name_text, info_text, count_text, nav_row].spacing(2);
        for param in &plugin.params {
            if param.name == "Dry/Wet" || param.name == "Output Gain" {
                let param_id = param.id;
                let inst_id = pid;
                let range = param.min_value..=param.max_value;
                let param_slider = slider(range, param.current_value, move |v| {
                    Message::SetPluginParam(inst_id, param_id, v)
                })
                .width(Length::Fill)
                .step(0.001);
                controls = controls.push(
                    column![
                        text(param.name.clone()).size(7).color(theme::TEXT_DIM),
                        param_slider,
                    ].spacing(0)
                );
            }
        }

        controls.into()
    }

}
