use iced::Size;
use resonance_audio::midi_hardware::MidiDeviceInfo;
use resonance_audio::types::*;
use resonance_audio::AudioEngine;
use resonance_music_theory::TableRegistry;

mod chord_sheet_pdf;
pub(crate) mod compose;
mod engine_events;
mod message;
mod midi_editor;
pub(crate) mod presets;
pub(crate) mod project;
pub(crate) mod recent;
pub(crate) mod state;
mod theme;
mod timeline;
mod timeline_draw;
mod timeline_input;
mod timeline_snap;
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
    /// Hardware MIDI input ports advertised by the OS. Refreshed
    /// periodically from `Tick` so hot-plugged devices appear.
    pub(crate) midi_input_devices: Vec<MidiDeviceInfo>,
    /// Hardware MIDI output ports.
    pub(crate) midi_output_devices: Vec<MidiDeviceInfo>,
    /// Wall-clock instant of the last MIDI device list refresh.
    pub(crate) midi_devices_last_refresh: std::time::Instant,
    /// Whether MIDI clock master output is enabled.
    pub(crate) midi_clock_send_enabled: bool,
    /// Hardware MIDI output port carrying the master clock.
    pub(crate) midi_clock_send_device: Option<String>,
    /// Whether MIDI clock slave (input) is enabled.
    pub(crate) midi_clock_recv_enabled: bool,
    /// Hardware MIDI input port carrying the master clock.
    pub(crate) midi_clock_recv_device: Option<String>,
    pub(crate) available_plugins: Vec<ScannedPlugin>,
    /// Cached pick-list option lists for the view layer. Rebuilt only
    /// when source data changes (devices, busses, plugin scan) so a
    /// continuous resize doesn't reallocate option vecs every frame.
    /// See `view::ui_caches` for the cache and rebuild API.
    pub(crate) view_caches: view::ui_caches::UiViewCaches,
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
    /// Markov table registry for chord generators. Constructed once at
    /// startup with all built-in tables.
    pub(crate) table_registry: TableRegistry,

    /// Tempo change events on the tempo track (sorted by bar number).
    pub(crate) tempo_events: Vec<state::TempoEvent>,
    /// Time signature change events on the signature track (sorted by bar).
    pub(crate) signature_events: Vec<state::SignatureEvent>,
    /// GUI-side tempo map — shared implementation with the audio engine.
    /// Rebuilt from `tempo_events` / `signature_events` whenever they change.
    pub(crate) tempo_map: TempoMap,

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
    /// When set, the "Bounce in place" dialog is shown for an external
    /// MIDI track. Holds the source track id plus the user's current
    /// device/port selection.
    pub(crate) bounce_dialog: Option<crate::view::bounce_dialog::BounceDialogState>,
    /// When set, a bounce-in-place run is in flight. Drives the modal
    /// progress overlay and gates transport / mutating UI so the user
    /// can't disturb the render mid-flight. Cleared by
    /// `TrackBounceCompleted`, `TrackBounceError`, or
    /// `TrackBounceCancelled`.
    pub(crate) bounce_in_progress: Option<crate::state::BounceProgressState>,
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

    // ---- Track presets ----
    /// Built-in default track presets (baked into the binary).
    pub(crate) default_presets: Vec<presets::TrackPreset>,
    /// User-saved track presets (loaded from disk on startup).
    pub(crate) user_presets: Vec<presets::TrackPreset>,
    /// When set, the next `TrackAdded` / `InstrumentTrackAdded` engine
    /// event will apply this preset to the newly created track.
    pub(crate) pending_track_preset: Option<presets::TrackPreset>,
    /// When set, the next `AllPluginStatesSaved` event will capture
    /// plugin states for this track and save it as a user preset.
    pub(crate) pending_preset_save: Option<resonance_audio::types::TrackId>,
    /// Plugin state blobs to apply as PluginAdded events arrive for a
    /// preset-created track. Tuple of (target track id, ordered list of
    /// state blobs matching the preset's plugin chain).
    pub(crate) pending_preset_plugin_states:
        Option<(resonance_audio::types::TrackId, Vec<Option<Vec<u8>>>)>,
}

/// Startup tab requested via `--tab arrange|mixer|compose`. Read once at
/// `main` and threaded into `Resonance::new()` via this module-local
/// statics — keeps the iced application builder closure capture-free.
static STARTUP_TAB: std::sync::OnceLock<ViewMode> = std::sync::OnceLock::new();
static DEMO_MODE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

fn parse_startup_tab() -> Option<ViewMode> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = if let Some(v) = arg.strip_prefix("--tab=") {
            v.to_string()
        } else if arg == "--tab" {
            args.next()?
        } else {
            continue;
        };
        return match value.to_ascii_lowercase().as_str() {
            "arrange" => Some(ViewMode::Arrange),
            "mixer" => Some(ViewMode::Mixer),
            "compose" => Some(ViewMode::Compose),
            other => {
                eprintln!("Unknown --tab value '{other}'. Expected arrange|mixer|compose.");
                None
            }
        };
    }
    None
}

fn parse_demo_flag() -> bool {
    std::env::args().any(|a| a == "--demo")
}

fn main() -> iced::Result {
    if let Some(tab) = parse_startup_tab() {
        let _ = STARTUP_TAB.set(tab);
    }
    let _ = DEMO_MODE.set(parse_demo_flag());

    let mut app = iced::application("Resonance", Resonance::update, Resonance::view)
        .font(theme::ICON_FONT_BYTES);
    for face in theme::UI_FONT_FACES {
        app = app.font(*face);
    }
    app.default_font(theme::UI_FONT)
        .subscription(Resonance::subscription)
        .theme(|_| theme::resonance_theme())
        .window_size(Size::new(1280.0, 720.0))
        .exit_on_close_request(false)
        // MSAA is expensive on Linux/Wayland with wgpu — every redraw
        // pays for a 4× sample buffer. Our canvases use rounded paths
        // sparingly and the lavender accent is forgiving without AA, so
        // disabling it speeds up the steady-state and makes window
        // resize visibly smoother. Tested on radv (Vulkan) where the AA
        // pass was the dominant per-frame cost.
        .antialiasing(false)
        .run_with(Resonance::new)
}

impl Resonance {
    /// Rebuild the GUI-side tempo map from the current events and send the
    /// events to the audio engine. Call whenever `tempo_events` or
    /// `signature_events` are modified.
    pub(crate) fn rebuild_and_send_tempo(&mut self) {
        self.rebuild_tempo_map();
        self.engine.send(AudioCommand::SetTempoEvents {
            tempo: self.tempo_events.clone(),
            signature: self.signature_events.clone(),
        });
    }

    /// Rebuild only the GUI-side tempo map (no engine send). Used when
    /// only UI display needs updating, e.g. during tempo drags.
    pub(crate) fn rebuild_tempo_map(&mut self) {
        self.tempo_map.tempo_points = self.tempo_events.clone();
        self.tempo_map.signature_points = self.signature_events.clone();
        self.tempo_map.bpm = self.transport.bpm;
        self.tempo_map.numerator = self.transport.time_sig_num;
        self.tempo_map.denominator = self.transport.time_sig_den;
        self.tempo_map.rebuild_bar_table(self.sample_rate);
    }


    /// Update the transport BPM display from the current tempo map.
    pub(crate) fn sync_tempo_display(&mut self) {
        let (bpm, _, _) = self
            .tempo_map
            .tempo_at_sample(self.transport.playhead, self.sample_rate);
        self.transport.bpm = bpm;
        self.transport.bpm_input = format!("{:.1}", bpm);
    }

    /// Remove a tempo event by index (must be > 0 to protect the initial
    /// event), rebuild the tempo map, and sync the BPM display.
    pub(crate) fn remove_tempo_event(&mut self, index: usize) {
        if index > 0 && index < self.tempo_events.len() {
            self.tempo_events.remove(index);
            self.rebuild_and_send_tempo();
            self.sync_tempo_display();
        }
    }

    /// Remove a signature event by index (must be > 0 to protect the initial
    /// event), rebuild the tempo map, and sync the time-signature display.
    pub(crate) fn remove_signature_event(&mut self, index: usize) {
        if index > 0 && index < self.signature_events.len() {
            self.signature_events.remove(index);
            self.rebuild_and_send_tempo();
            let (_, num, den) = self
                .tempo_map
                .tempo_at_sample(self.transport.playhead, self.sample_rate);
            self.transport.time_sig_num = num;
            self.transport.time_sig_den = den;
            self.engine.send(AudioCommand::SetTimeSignature {
                numerator: num,
                denominator: den,
            });
        }
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
        let result = self.registry.with_track_mut(id, f);
        debug_assert!(result.is_some(), "with_track_mut: no track with id {id:?}");
        result
    }

    /// Run `f` on the bus with the given id, returning whatever `f`
    /// returns. `None` if the bus doesn't exist.
    pub(crate) fn with_bus_mut<R>(
        &mut self,
        id: BusId,
        f: impl FnOnce(&mut BusState) -> R,
    ) -> Option<R> {
        let result = self.registry.with_bus_mut(id, f);
        debug_assert!(result.is_some(), "with_bus_mut: no bus with id {id:?}");
        result
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
        let result = self
            .master_plugins
            .iter_mut()
            .find(|p| p.instance_id == instance_id)
            .map(f);
        debug_assert!(
            result.is_some(),
            "with_plugin_mut: no plugin with id {instance_id:?}"
        );
        result
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
                panic!("Audio engine init failed: {e}");
            }
        };

        // Request input device list and plugin scan on startup
        engine.send(AudioCommand::ListInputDevices);
        engine.send(AudioCommand::ListMidiInputDevices);
        engine.send(AudioCommand::ListMidiOutputDevices);
        engine.send(AudioCommand::ScanPlugins);

        let recent_projects = recent::load();

        let app = Self {
            engine,
            sample_rate: 44100, // overwritten by SampleRateDetected event
            input_devices: Vec::new(),
            default_input_device_name: None,
            midi_input_devices: Vec::new(),
            midi_output_devices: Vec::new(),
            midi_devices_last_refresh: std::time::Instant::now(),
            midi_clock_send_enabled: false,
            midi_clock_send_device: None,
            midi_clock_recv_enabled: false,
            midi_clock_recv_device: None,
            available_plugins: Vec::new(),
            view_caches: view::ui_caches::UiViewCaches::default(),
            error_message: None,
            master_volume: 0.0, // 0 dB = unity gain
            master_level_l: 0.0,
            master_level_r: 0.0,
            master_plugins: Vec::new(),
            master_fx_bypassed: false,
            view_mode: STARTUP_TAB.get().copied().unwrap_or(ViewMode::Arrange),
            clips: Vec::new(),
            midi_clips: Vec::new(),
            compose: compose::ComposeState::default(),
            table_registry: TableRegistry::with_builtins(),

            tempo_events: vec![state::TempoEvent { bar: 0, bpm: 120.0 }],
            signature_events: vec![state::SignatureEvent {
                bar: 0,
                numerator: 4,
                denominator: 4,
            }],
            tempo_map: TempoMap::default(),

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
            bounce_dialog: None,
            bounce_in_progress: None,
            dirty: false,
            confirm_quit: None,
            quit_after_save: None,
            default_presets: presets::default_presets(),
            user_presets: presets::load_user_presets(),
            pending_track_preset: None,
            pending_preset_save: None,
            pending_preset_plugin_states: None,
        };

        let mut app = app;
        if DEMO_MODE.get().copied().unwrap_or(false) {
            seed_demo_content(&mut app);
        }

        (app, iced::Task::none())
    }
}

/// Populate the GUI-side state with a small set of tracks, busses, clips,
/// and a Compose section so the views render with content for screenshots.
/// Bypasses the audio engine entirely — these objects exist only in the
/// app's `registry` / `compose` / `clips` collections and won't make sound.
fn seed_demo_content(app: &mut Resonance) {
    use resonance_audio::types::{MidiNote, SamplePos};
    use resonance_music_theory::{Chord, ChordQuality, Mode, MotifSource, PitchClass, Scale};

    use crate::compose::{
        ChordState, GenerateParams, SectionDefinitionState, SectionPlacementState,
    };

    app.io.has_active_project = true;
    app.transport.bpm = 90.0;
    app.transport.bpm_input = "90.0".to_string();
    app.transport.time_sig_num = 6;
    app.transport.time_sig_den = 8;
    app.tempo_events = vec![state::TempoEvent { bar: 0, bpm: 90.0 }];
    app.signature_events = vec![state::SignatureEvent {
        bar: 0,
        numerator: 6,
        denominator: 8,
    }];
    app.rebuild_tempo_map();

    let sr = app.sample_rate as u64;
    let secs_per_beat = 60.0 / app.transport.bpm as f64;
    let bar_samples = (secs_per_beat * 6.0 * sr as f64) as u64;
    app.master_level_l = 0.62;
    app.master_level_r = 0.48;

    // ---- Tracks ----
    let mk_instr = |id: u64,
                    order: usize,
                    name: &str,
                    plugin_name: &str,
                    icon: state::InstrumentIcon|
     -> TrackState {
        let mut t = TrackState::new_instrument(id, order);
        t.name = name.to_string();
        t.instrument_icon = icon;
        t.level_l = 0.5;
        t.level_r = 0.4;
        if !plugin_name.is_empty() {
            t.plugins.push(PluginSlotState::new(
                id * 100,
                plugin_name.to_string(),
                String::new(),
                String::new(),
                Vec::new(),
                false,
            ));
        }
        t
    };

    let mut drums = mk_instr(
        1,
        0,
        "Drums",
        "Resonance Drums",
        state::InstrumentIcon::Drum,
    );
    drums.instrument_type = state::InstrumentType::Drum;

    let bass = mk_instr(
        2,
        1,
        "Synth Bass",
        "Resonance Wave",
        state::InstrumentIcon::Music,
    );
    let pad = mk_instr(
        3,
        2,
        "Synth Pad",
        "Resonance Wave",
        state::InstrumentIcon::WaveSquare,
    );
    let lead = mk_instr(
        4,
        3,
        "Lead Synth",
        "Resonance Wave",
        state::InstrumentIcon::Music,
    );

    let mut audio = TrackState::new_audio(5, 4);
    audio.name = "Drums Bounce".to_string();
    audio.muted = true;
    audio.instrument_icon = state::InstrumentIcon::Microphone;

    app.registry.tracks = vec![drums, bass, pad, lead, audio];
    app.registry.next_track_order = 5;
    app.interaction.selected_track = Some(2);

    // ---- Busses ----
    app.registry.busses = vec![
        BusState::new(100, 0, "Bus 1 · Drums".to_string()),
        BusState::new(101, 1, "Bus 2 · FX".to_string()),
    ];
    app.registry.busses[0].plugins.push(PluginSlotState::new(
        10001,
        "Comp".to_string(),
        String::new(),
        String::new(),
        Vec::new(),
        false,
    ));
    app.registry.busses[0].level_l = 0.55;
    app.registry.busses[0].level_r = 0.50;
    app.registry.busses[1].plugins.push(PluginSlotState::new(
        10002,
        "Verb".to_string(),
        String::new(),
        String::new(),
        Vec::new(),
        false,
    ));
    app.registry.busses[1].level_l = 0.32;
    app.registry.busses[1].level_r = 0.30;
    app.registry.next_bus_order = 2;
    // Demo seed bypasses the engine event handlers that normally
    // refresh these caches, so refresh them by hand.
    app.view_caches.rebuild_output(&app.registry.busses);

    // ---- Clips on the timeline ----
    let bar_ticks = 480 * 6 / 2; // 6/8 → 6 eighth-note beats per bar
    let make_midi_clip = |id: u64,
                          track: u64,
                          name: &str,
                          start_bar: u64,
                          length_bars: u64,
                          density: u32|
     -> MidiClipState {
        let mut notes = Vec::new();
        let total_ticks = length_bars * bar_ticks;
        let step = (total_ticks / density as u64).max(60);
        let mut tick = 0u64;
        let mut pitch = 60u8;
        let mut i = 0u32;
        while tick < total_ticks {
            notes.push(MidiNote {
                note: pitch,
                velocity: 0.8,
                start_tick: tick,
                duration_ticks: (step * 9) / 10,
            });
            tick += step;
            i += 1;
            pitch = 48 + ((i * 5) % 24) as u8;
        }
        MidiClipState {
            id,
            track_id: track,
            start_sample: (start_bar * bar_samples) as SamplePos,
            duration_ticks: total_ticks,
            name: name.to_string(),
            notes,
            trim_start_ticks: 0,
            trim_end_ticks: 0,
        }
    };

    app.midi_clips = vec![
        make_midi_clip(11, 1, "Pattern A", 0, 6, 32),
        make_midi_clip(12, 2, "Bm progression", 0, 6, 12),
        make_midi_clip(13, 3, "Pad", 0, 6, 8),
        make_midi_clip(14, 4, "Motif", 0, 6, 20),
    ];

    // Audio bounce on track 5 — uses peaks rather than a real waveform.
    let peak_count = 256usize;
    let waveform_peaks = (0..peak_count)
        .map(|i| {
            let t = i as f32 / peak_count as f32;
            let amp = 0.4 + 0.4 * (t * 12.0).sin().abs();
            (-amp, amp)
        })
        .collect();
    app.clips = vec![ClipState {
        id: 15,
        track_id: 5,
        start_sample: 0,
        duration_samples: bar_samples * 5 + bar_samples / 2,
        name: "Drums bounce".to_string(),
        total_frames: bar_samples * 5 + bar_samples / 2,
        trim_start_frames: 0,
        trim_end_frames: 0,
        waveform_peaks,
    }];

    // Place the playhead a bit into the song so it's visible.
    app.transport.playhead = bar_samples * 4;

    // ---- Compose section ----
    let def_id = app.compose.fresh_id();
    let chords = [
        Chord::new(PitchClass::B, ChordQuality::Min),
        Chord::new(PitchClass::B, ChordQuality::Min),
        Chord::new(PitchClass::Fs, ChordQuality::Maj),
        Chord::new(PitchClass::G, ChordQuality::Maj),
        Chord::new(PitchClass::E, ChordQuality::Min),
    ];
    let chord_states: Vec<ChordState> = chords
        .iter()
        .enumerate()
        .map(|(i, c)| ChordState {
            id: app.compose.fresh_id(),
            start_beat: i as u32 * 4,
            duration_beats: 4,
            chord: *c,
        })
        .collect();

    app.compose.definitions.push(SectionDefinitionState {
        id: def_id,
        name: "Intro".to_string(),
        color: [139, 109, 255],
        length_bars: 8,
        chords: chord_states,
        scale: Some(Scale::new(PitchClass::B, Mode::Minor)),
        progression_seed: 12345,
        generate_params: GenerateParams::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators: std::collections::HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: MotifSource::default(),
    });

    let placement_id = app.compose.fresh_id();
    app.compose.placements.push(SectionPlacementState {
        id: placement_id,
        definition_id: def_id,
        start_bar: 0,
    });
    app.compose.selected_placement_id = Some(placement_id);
    let _ = TrackOutput::Master; // silence unused-import warning when feature flags shift
}
