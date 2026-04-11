use iced::Size;
use resonance_audio::types::*;
use resonance_audio::AudioEngine;

pub(crate) mod compose;
mod engine_events;
mod message;
mod midi_editor;
pub(crate) mod project;
pub(crate) mod state;
mod theme;
mod timeline;
mod timeline_draw;
mod update;
pub(crate) mod util;
mod view;

use message::Message;
use state::*;

/// Application state.
pub(crate) struct Resonance {
    pub(crate) engine: AudioEngine,
    pub(crate) tracks: Vec<TrackState>,
    pub(crate) busses: Vec<BusState>,
    pub(crate) next_bus_order: usize,
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
    /// Id counter for auto-created sub-tracks. Lives in a high numeric
    /// range so it never collides with engine-allocated track ids
    /// (engine tracks count up from 1). Sub-tracks are purely app-side
    /// for now — the audio engine doesn't know about them until Phase 5.
    pub(crate) next_sub_track_id: u64,
    /// Parent track ids whose sub-tracks are currently collapsed in the
    /// mixer view. Purely UI state — the engine doesn't care.
    pub(crate) collapsed_sub_track_parents: std::collections::HashSet<TrackId>,
    pub(crate) input_devices: Vec<InputDeviceInfo>,
    pub(crate) default_input_device_name: Option<String>,
    pub(crate) bpm: f32,
    /// Editable text buffer backing the BPM text_input widget.
    pub(crate) bpm_input: String,
    pub(crate) time_sig_num: u8,
    pub(crate) time_sig_den: u8,
    pub(crate) metronome_enabled: bool,
    /// Number of bars the metronome counts in before playback/recording
    /// starts. 0 disables the pre-count.
    pub(crate) precount_bars: u8,
    pub(crate) available_plugins: Vec<ScannedPlugin>,
    pub(crate) settings_open: bool,
    pub(crate) add_track_menu_open: bool,
    pub(crate) error_message: Option<String>,
    pub(crate) master_volume: f32,
    pub(crate) master_level_l: f32,
    pub(crate) master_level_r: f32,
    pub(crate) loop_enabled: bool,
    pub(crate) loop_in: u64,
    pub(crate) loop_out: u64,
    pub(crate) loop_range_set: bool,
    pub(crate) dragging_loop: Option<LoopDragTarget>,
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
    /// Total content width of the timeline in pixels, reported from the
    /// canvas. Used to clamp horizontal scroll and size the scrollbar thumb.
    pub(crate) timeline_content_width: f32,
    /// Total content height of the timeline in pixels, reported from the
    /// canvas. Used to clamp vertical scroll and size the scrollbar thumb.
    pub(crate) timeline_content_height: f32,
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
    /// Compose tab state: section definitions, placements, chord progressions.
    pub(crate) compose: compose::ComposeState,
}

fn main() -> iced::Result {
    iced::application("Resonance", Resonance::update, Resonance::view)
        .font(theme::ICON_FONT_BYTES)
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

    pub(crate) fn sorted_busses(&self) -> Vec<&BusState> {
        let mut busses: Vec<&BusState> = self.busses.iter().collect();
        busses.sort_by_key(|b| b.order);
        busses
    }

    /// Run `f` on the track with the given id, returning whatever `f`
    /// returns. `None` if the track doesn't exist.
    pub(crate) fn with_track_mut<R>(
        &mut self,
        id: TrackId,
        f: impl FnOnce(&mut TrackState) -> R,
    ) -> Option<R> {
        self.tracks.iter_mut().find(|t| t.id == id).map(f)
    }

    /// Run `f` on the bus with the given id, returning whatever `f`
    /// returns. `None` if the bus doesn't exist.
    pub(crate) fn with_bus_mut<R>(
        &mut self,
        id: BusId,
        f: impl FnOnce(&mut BusState) -> R,
    ) -> Option<R> {
        self.busses.iter_mut().find(|b| b.id == id).map(f)
    }

    /// Locate a plugin slot on any track or bus by instance id and run
    /// `f` on it. Iterates tracks first, then busses.
    pub(crate) fn with_plugin_mut<R>(
        &mut self,
        instance_id: PluginInstanceId,
        f: impl FnOnce(&mut PluginSlotState) -> R,
    ) -> Option<R> {
        for track in &mut self.tracks {
            if let Some(p) = track
                .plugins
                .iter_mut()
                .find(|p| p.instance_id == instance_id)
            {
                return Some(f(p));
            }
        }
        for bus in &mut self.busses {
            if let Some(p) = bus
                .plugins
                .iter_mut()
                .find(|p| p.instance_id == instance_id)
            {
                return Some(f(p));
            }
        }
        None
    }

    /// Find the index in `self.tracks` of the visible track at the given
    /// y coordinate in the arrange view. Used by clip drag handlers to
    /// pick the target lane under the cursor. Sub-tracks are excluded
    /// (the arrange view hides them).
    pub(crate) fn track_id_at_arrange_y(&self, y: f32) -> Option<TrackId> {
        let ruler_height = theme::RULER_HEIGHT;
        let track_idx = ((y - ruler_height + self.scroll_offset_y) / theme::TRACK_HEIGHT)
            .floor()
            .max(0.0) as usize;
        let mut sorted: Vec<&TrackState> = self
            .tracks
            .iter()
            .filter(|t| t.sub_track.is_none())
            .collect();
        sorted.sort_by_key(|t| t.order);
        sorted.get(track_idx).map(|t| t.id)
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
            busses: Vec::new(),
            next_bus_order: 0,
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
            next_sub_track_id: 1_000_000_000,
            collapsed_sub_track_parents: std::collections::HashSet::new(),
            input_devices: Vec::new(),
            default_input_device_name: None,
            bpm: 120.0,
            bpm_input: "120".to_string(),
            time_sig_num: 4,
            time_sig_den: 4,
            metronome_enabled: false,
            precount_bars: 2,
            available_plugins: Vec::new(),
            settings_open: false,
            add_track_menu_open: false,
            error_message: None,
            master_volume: 0.0, // 0 dB = unity gain
            master_level_l: 0.0,
            master_level_r: 0.0,
            loop_enabled: false,
            loop_in: 0,
            loop_out: 0,
            loop_range_set: false,
            dragging_loop: None,
            view_mode: ViewMode::Arrange,
            selected_clip: None,
            clip_drag: None,
            clip_trim: None,
            selected_plugin: None,
            viewport_width: 1000.0,
            timeline_content_width: 1000.0,
            timeline_content_height: 0.0,
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
            compose: compose::ComposeState::default(),
        };

        (app, iced::Task::none())
    }
}
