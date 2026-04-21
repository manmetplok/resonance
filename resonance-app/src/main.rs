use iced::Size;
use resonance_audio::types::*;
use resonance_audio::AudioEngine;

pub(crate) mod compose;
mod engine_events;
mod message;
mod midi_editor;
pub(crate) mod project;
pub(crate) mod recent;
pub(crate) mod state;
mod theme;
mod timeline;
mod timeline_draw;
pub(crate) mod undo;
mod update;
pub(crate) mod util;
mod view;

use message::Message;
use state::*;
use undo::UndoHistory;

/// Application state.
pub(crate) struct Resonance {
    pub(crate) engine: AudioEngine,
    pub(crate) sample_rate: u32,
    pub(crate) input_devices: Vec<InputDeviceInfo>,
    pub(crate) default_input_device_name: Option<String>,
    pub(crate) available_plugins: Vec<ScannedPlugin>,
    pub(crate) error_message: Option<String>,
    pub(crate) master_volume: f32,
    pub(crate) master_level_l: f32,
    pub(crate) master_level_r: f32,
    /// FX plugins inserted on the master bus, rendered after every
    /// track and bus has been summed.
    pub(crate) master_plugins: Vec<PluginSlotState>,
    /// When true, the master FX chain is bypassed — the master fader
    /// and metering still run, but no master-bus plugins are processed.
    pub(crate) master_fx_bypassed: bool,
    pub(crate) view_mode: ViewMode,
    /// Audio clips on the timeline.
    pub(crate) clips: Vec<ClipState>,
    /// MIDI clips on the timeline.
    pub(crate) midi_clips: Vec<MidiClipState>,
    /// Compose tab state: section definitions, placements, chord progressions.
    pub(crate) compose: compose::ComposeState,

    /// Tempo change events on the tempo track (sorted by bar number).
    pub(crate) tempo_events: Vec<state::TempoEvent>,
    /// Time signature change events on the signature track (sorted by bar).
    pub(crate) signature_events: Vec<state::SignatureEvent>,

    // Sub-state groupings. See `state.rs` for definitions.
    pub(crate) transport: TransportState,
    pub(crate) viewport: ArrangeViewport,
    pub(crate) interaction: ClipInteractionState,
    pub(crate) io: ProjectIoState,
    pub(crate) mixer: MixerUiState,
    pub(crate) registry: TrackRegistry,
    /// Session-local undo/redo history. Cleared on project load.
    pub(crate) undo: UndoHistory,
    /// When set, the confirmation dialog for deleting a track with
    /// content is shown. Holds the track id that the user wants to remove.
    pub(crate) confirm_delete_track: Option<resonance_audio::types::TrackId>,
    /// True when the project has been modified since the last save.
    pub(crate) dirty: bool,
    /// When set, the "unsaved changes" quit-confirmation dialog is shown.
    /// Holds the window id so we can close it if the user confirms.
    pub(crate) confirm_quit: Option<iced::window::Id>,
    /// When set, the app should quit after the current save completes.
    /// Set by the "Save & Quit" flow in the unsaved-changes dialog.
    pub(crate) quit_after_save: Option<iced::window::Id>,
    /// Cache of the most recently observed CLAP state blob per plugin
    /// instance. Populated from `PluginStateSaved` / `AllPluginStatesSaved`
    /// engine events and read into undo snapshots so restores can replay
    /// plugin internal state via `LoadPluginState`. Stale between
    /// refreshes — parameter values in snapshots always come from live
    /// GUI state instead.
    pub(crate) plugin_state_cache:
        std::collections::HashMap<resonance_audio::types::PluginInstanceId, Vec<u8>>,
}

fn main() -> iced::Result {
    iced::application("Resonance", Resonance::update, Resonance::view)
        .font(theme::ICON_FONT_BYTES)
        .subscription(Resonance::subscription)
        .theme(|_| theme::resonance_theme())
        .window_size(Size::new(1280.0, 720.0))
        .exit_on_close_request(false)
        .run_with(Resonance::new)
}

impl Resonance {
    /// Update the transport BPM display from the current tempo events.
    pub(crate) fn sync_tempo_display(&mut self) {
        let (bpm, _, _) = state::tempo_at_sample(
            self.transport.playhead,
            &self.tempo_events,
            &self.signature_events,
            self.sample_rate,
        );
        self.transport.bpm = bpm;
        self.transport.bpm_input = format!("{:.1}", bpm);
    }

    /// Send the full tempo event list to the audio engine so it can
    /// compute BPM at any playhead position without per-tick commands.
    pub(crate) fn send_tempo_events_to_engine(&self) {
        use resonance_audio::types::{AudioCommand, TempoPoint, SignaturePoint};
        let tempo: Vec<TempoPoint> = self.tempo_events.iter().map(|e| {
            TempoPoint { bar: e.bar, bpm: e.bpm }
        }).collect();
        let signature: Vec<SignaturePoint> = self.signature_events.iter().map(|e| {
            SignaturePoint { bar: e.bar, numerator: e.numerator, denominator: e.denominator }
        }).collect();
        self.engine.send(AudioCommand::SetTempoEvents { tempo, signature });
    }

    pub(crate) fn sorted_tracks(&self) -> Vec<&TrackState> {
        self.registry.sorted_tracks()
    }

    pub(crate) fn sorted_busses(&self) -> Vec<&BusState> {
        self.registry.sorted_busses()
    }

    /// Run `f` on the track with the given id, returning whatever `f`
    /// returns. `None` if the track doesn't exist.
    pub(crate) fn with_track_mut<R>(
        &mut self,
        id: TrackId,
        f: impl FnOnce(&mut TrackState) -> R,
    ) -> Option<R> {
        self.registry.with_track_mut(id, f)
    }

    /// Run `f` on the bus with the given id, returning whatever `f`
    /// returns. `None` if the bus doesn't exist.
    pub(crate) fn with_bus_mut<R>(
        &mut self,
        id: BusId,
        f: impl FnOnce(&mut BusState) -> R,
    ) -> Option<R> {
        self.registry.with_bus_mut(id, f)
    }

    /// Locate a plugin slot on any track, bus, or master by instance id
    /// and run `f` on it. Iterates tracks first, then busses, then master.
    pub(crate) fn with_plugin_mut<R>(
        &mut self,
        instance_id: PluginInstanceId,
        f: impl FnOnce(&mut PluginSlotState) -> R,
    ) -> Option<R> {
        for track in &mut self.registry.tracks {
            if let Some(p) = track
                .plugins
                .iter_mut()
                .find(|p| p.instance_id == instance_id)
            {
                return Some(f(p));
            }
        }
        for bus in &mut self.registry.busses {
            if let Some(p) = bus
                .plugins
                .iter_mut()
                .find(|p| p.instance_id == instance_id)
            {
                return Some(f(p));
            }
        }
        self.master_plugins
            .iter_mut()
            .find(|p| p.instance_id == instance_id)
            .map(f)
    }

    /// Find the index in `self.registry.tracks` of the visible track at the
    /// given y coordinate in the arrange view. Used by clip drag handlers
    /// to pick the target lane under the cursor. Sub-tracks are excluded
    /// (the arrange view hides them).
    pub(crate) fn track_id_at_arrange_y(&self, y: f32) -> Option<TrackId> {
        let ruler_height = theme::RULER_HEIGHT;
        let track_idx = ((y - ruler_height + self.viewport.scroll_offset_y) / theme::TRACK_HEIGHT)
            .floor()
            .max(0.0) as usize;
        let mut sorted: Vec<&TrackState> = self
            .registry
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

        let recent_projects = recent::load();

        let app = Self {
            engine,
            sample_rate: 44100, // overwritten by SampleRateDetected event
            input_devices: Vec::new(),
            default_input_device_name: None,
            available_plugins: Vec::new(),
            error_message: None,
            master_volume: 0.0, // 0 dB = unity gain
            master_level_l: 0.0,
            master_level_r: 0.0,
            master_plugins: Vec::new(),
            master_fx_bypassed: false,
            view_mode: ViewMode::Arrange,
            clips: Vec::new(),
            midi_clips: Vec::new(),
            compose: compose::ComposeState::default(),

            tempo_events: vec![state::TempoEvent { bar: 0, bpm: 120.0 }],
            signature_events: vec![state::SignatureEvent { bar: 0, numerator: 4, denominator: 4 }],

            transport: TransportState::default(),
            viewport: ArrangeViewport::default(),
            interaction: ClipInteractionState::default(),
            io: ProjectIoState {
                recent_projects,
                ..ProjectIoState::default()
            },
            mixer: MixerUiState::default(),
            registry: TrackRegistry {
                next_sub_track_id: 1_000_000_000,
                ..TrackRegistry::default()
            },
            undo: UndoHistory::new(),
            plugin_state_cache: std::collections::HashMap::new(),
            confirm_delete_track: None,
            dirty: false,
            confirm_quit: None,
            quit_after_save: None,
        };

        (app, iced::Task::none())
    }
}
