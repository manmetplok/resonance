use iced::Size;
use resonance_audio::types::*;
use resonance_audio::AudioEngine;

mod engine_events;
mod message;
mod midi_editor;
pub(crate) mod project;
pub(crate) mod state;
mod theme;
mod timeline;
mod update;
pub(crate) mod util;
mod view;

use message::Message;
use state::*;

/// Application state.
pub(crate) struct Resonance {
    pub(crate) engine: AudioEngine,
    pub(crate) tracks: Vec<TrackState>,
    pub(crate) clips: Vec<ClipState>,
    pub(crate) playhead: u64,
    pub(crate) playing: bool,
    pub(crate) recording: bool,
    pub(crate) recording_start_sample: u64,
    pub(crate) sample_rate: u32,
    pub(crate) zoom: f32,           // pixels per second
    pub(crate) scroll_offset: f32,  // horizontal scroll in pixels
    pub(crate) scroll_offset_y: f32, // vertical scroll in pixels
    pub(crate) next_track_order: usize,
    pub(crate) input_devices: Vec<InputDeviceInfo>,
    pub(crate) default_input_device_name: Option<String>,
    pub(crate) bpm: f32,
    pub(crate) time_sig_num: u8,
    pub(crate) time_sig_den: u8,
    pub(crate) metronome_enabled: bool,
    pub(crate) available_plugins: Vec<ScannedPlugin>,
    pub(crate) settings_open: bool,
    pub(crate) error_message: Option<String>,
    pub(crate) master_volume: f32,
    pub(crate) master_level_l: f32,
    pub(crate) master_level_r: f32,
    pub(crate) punch_enabled: bool,
    pub(crate) punch_in: u64,
    pub(crate) punch_out: u64,
    pub(crate) punch_range_set: bool,
    pub(crate) dragging_punch: Option<PunchDragTarget>,
    /// Pending file path to inject via CLAP state (instance_id, persist_key, path).
    pub(crate) pending_plugin_path: Option<(PluginInstanceId, String, String)>,
    pub(crate) view_mode: ViewMode,
    pub(crate) selected_clip: Option<ClipId>,
    /// Clip drag state: (clip_id, grab_offset_x, original_start_sample, original_track_id, current_x)
    pub(crate) clip_drag: Option<ClipDragState>,
    /// Clip trim state
    pub(crate) clip_trim: Option<ClipTrimState>,
    /// Currently selected plugin for the bottom panel in mixer view.
    pub(crate) selected_plugin: Option<PluginInstanceId>,
    /// Current viewport width of the timeline canvas (in pixels).
    pub(crate) viewport_width: f32,
    /// Whether an offline bounce is in progress.
    pub(crate) bouncing: bool,
    /// Path to the current project directory (None if unsaved).
    pub(crate) project_path: Option<std::path::PathBuf>,
    /// In-progress save state: collecting data from engine.
    pub(crate) save_state: Option<project::SaveCollector>,
    /// True while loading a project (suppresses engine events).
    pub(crate) loading: bool,
    /// Pending project data to replay after ClearAll completes.
    pub(crate) pending_load: Option<Box<project::LoadedProject>>,
    /// MIDI clips on the timeline.
    pub(crate) midi_clips: Vec<MidiClipState>,
    /// Selected MIDI clip for interactions.
    pub(crate) selected_midi_clip: Option<ClipId>,
    /// MIDI clip drag state.
    pub(crate) midi_clip_drag: Option<MidiClipDragState>,
    /// MIDI clip trim state.
    pub(crate) midi_clip_trim: Option<MidiClipTrimState>,
    /// Currently open MIDI clip in the piano roll editor.
    pub(crate) editing_midi_clip: Option<MidiEditorState>,
}

fn main() -> iced::Result {
    iced::application("Resonance", Resonance::update, Resonance::view)
        .subscription(Resonance::subscription)
        .theme(|_| theme::resonance_theme())
        .window_size(Size::new(1280.0, 720.0))
        .run_with(Resonance::new)
}

impl Resonance {
    pub(crate) fn sorted_tracks(&self) -> Vec<&TrackState> {
        let mut tracks: Vec<&TrackState> = self.tracks.iter().collect();
        tracks.sort_by_key(|t| t.order);
        tracks
    }

    fn new() -> (Self, iced::Task<Message>) {
        let engine = match AudioEngine::new() {
            Ok(engine) => engine,
            Err(e) => {
                eprintln!("Audio engine init failed: {e}");
                std::process::exit(1);
            }
        };

        // Request input device list and plugin scan on startup
        engine.send(AudioCommand::ListInputDevices);
        engine.send(AudioCommand::ScanPlugins);

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
            error_message: None,
            master_volume: 0.0, // 0 dB = unity gain
            master_level_l: 0.0,
            master_level_r: 0.0,
            punch_enabled: false,
            punch_in: 0,
            punch_out: 0,
            punch_range_set: false,
            dragging_punch: None,
            pending_plugin_path: None,
            view_mode: ViewMode::Arrange,
            selected_clip: None,
            clip_drag: None,
            clip_trim: None,
            selected_plugin: None,
            viewport_width: 1000.0,
            bouncing: false,
            project_path: None,
            save_state: None,
            loading: false,
            pending_load: None,
            midi_clips: Vec::new(),
            selected_midi_clip: None,
            midi_clip_drag: None,
            midi_clip_trim: None,
            editing_midi_clip: None,
        };

        (app, iced::Task::none())
    }
}
