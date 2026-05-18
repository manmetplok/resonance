/// GUI-side state types for the Resonance application.
use resonance_audio::types::*;
use serde::{Deserialize, Serialize};

/// Whether an in-flight bounce is rendering offline (CLAP synth) or
/// recording in real time from an audio input. Drives the progress
/// modal's wording and gates which features the cancel button enables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BounceMode {
    Offline,
    Realtime,
}

/// Active state for the bounce-in-place progress modal.
#[derive(Debug, Clone)]
pub struct BounceProgressState {
    pub mode: BounceMode,
    /// Display name of the source track (used in the modal title).
    pub source_name: String,
    /// `[0.0, 1.0]` from the engine's `BounceProgress` events.
    pub fraction: f32,
}

// ---------------------------------------------------------------------------
// Global track events (tempo & time signature changes)
// ---------------------------------------------------------------------------

/// A tempo change on the tempo track. Type alias for the engine's
/// `TempoPoint` — both sides share the same type and the same `TempoMap`
/// implementation, eliminating any risk of divergence.
pub type TempoEvent = TempoPoint;

/// A time signature change on the signature track. Type alias for the
/// engine's `SignaturePoint`.
pub type SignatureEvent = SignaturePoint;

/// Sub-type of an instrument track, surfaced in the Compose tab. Only used
/// for display and icon defaulting — the audio engine itself treats all
/// instrument tracks identically.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InstrumentType {
    #[default]
    Synth,
    Drum,
}

impl InstrumentType {
    pub fn as_str(self) -> &'static str {
        match self {
            InstrumentType::Synth => "Synth",
            InstrumentType::Drum => "Drum",
        }
    }
}

impl std::fmt::Display for InstrumentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Role a track plays inside a compose section's derived arrangement.
/// When the user clicks "Derive pad / bass / lead", the compose handler
/// targets the first track carrying the matching role. Untagged tracks
/// (`None`) are never picked automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackRole {
    Pad,
    Bass,
    Lead,
}

impl TrackRole {
    pub fn as_str(self) -> &'static str {
        match self {
            TrackRole::Pad => "Pad",
            TrackRole::Bass => "Bass",
            TrackRole::Lead => "Lead",
        }
    }
}

impl std::fmt::Display for TrackRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Icon shown next to the instrument name in the Compose tab. Backed by a
/// Font Awesome glyph; kept in an enum so the persisted value survives
/// font-file renames.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InstrumentIcon {
    #[default]
    Music,
    Drum,
    Guitar,
    Microphone,
    WaveSquare,
    CompactDisc,
    Sliders,
}

impl InstrumentIcon {
    pub fn glyph(self) -> char {
        use crate::theme::fa;
        match self {
            InstrumentIcon::Music => fa::MUSIC,
            InstrumentIcon::Drum => fa::DRUM,
            InstrumentIcon::Guitar => fa::GUITAR,
            InstrumentIcon::Microphone => fa::MICROPHONE,
            InstrumentIcon::WaveSquare => fa::WAVE_SQUARE,
            InstrumentIcon::CompactDisc => fa::COMPACT_DISC,
            InstrumentIcon::Sliders => fa::SLIDERS,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            InstrumentIcon::Music => "Music",
            InstrumentIcon::Drum => "Drum",
            InstrumentIcon::Guitar => "Guitar",
            InstrumentIcon::Microphone => "Microphone",
            InstrumentIcon::WaveSquare => "Wave",
            InstrumentIcon::CompactDisc => "Disc",
            InstrumentIcon::Sliders => "Sliders",
        }
    }
}

impl std::fmt::Display for InstrumentIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct ClipDragState {
    pub clip_id: ClipId,
    pub grab_offset_x: f32,
    pub original_track_id: TrackId,
    pub current_x: f32,
    pub current_y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipEdge {
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct ClipTrimState {
    pub clip_id: ClipId,
    pub edge: ClipEdge,
    pub original_start_sample: SamplePos,
    pub original_trim_start: u64,
    pub original_trim_end: u64,
    pub original_total_frames: u64,
    pub anchor_x: f32,
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
    /// When true, every effect plugin on this track is bypassed.
    /// Instrument plugins (slot 0 on instrument tracks) still play.
    pub fx_bypassed: bool,
    pub order: usize,
    pub record_armed: bool,
    pub monitor_enabled: bool,
    pub mono: bool,
    pub input_device_name: Option<String>,
    /// 0-indexed starting input channel on the track's input device.
    pub input_port_index: u16,
    pub plugins: Vec<PluginSlotState>,
    /// Current VU meter level for left channel (linear amplitude).
    pub level_l: f32,
    /// Current VU meter level for right channel (linear amplitude).
    pub level_r: f32,
    pub track_type: TrackType,
    /// Where this track's post-fader audio is routed.
    pub output: TrackOutput,
    /// Instrument sub-type (synth/drum). Only meaningful when
    /// `track_type == TrackType::Instrument`; audio tracks carry the default.
    pub instrument_type: InstrumentType,
    /// Icon shown next to the name in Compose. Default is
    /// `InstrumentIcon::default_for(instrument_type)` for new tracks.
    pub instrument_icon: InstrumentIcon,
    /// Role this track plays inside derived-arrangement workflows. `None`
    /// means the track is excluded from auto-derive targeting.
    pub role: Option<TrackRole>,
    /// When set, this track is a sub-track that reads one non-main output
    /// port from its parent track's instrument plugin. Sub-tracks have
    /// their own fader / pan / mute / bus routing, but no clips, no plugin
    /// chain of their own, and no record arm — they're fed entirely from
    /// the parent plugin's fan-out. `None` on all normal tracks.
    pub sub_track: Option<SubTrackLink>,
    /// Hardware MIDI input device the user picked for this track. Notes
    /// arriving on this device feed the track's instrument plugin (live
    /// monitoring) and, when the track is record-armed during playback,
    /// are captured into a MIDI clip on the timeline.
    pub midi_input_device: Option<String>,
    /// Channel filter for hardware MIDI input. `None` = omni (accept any).
    pub midi_input_channel: Option<u8>,
    /// Hardware MIDI output device. Notes played on this track (currently
    /// live input passthrough; timeline playback to external is a future
    /// extension) are also sent here in addition to the instrument plugin.
    pub midi_output_device: Option<String>,
    /// Channel that hardware MIDI output uses (`None` = channel 1).
    pub midi_output_channel: Option<u8>,
}

/// Identifies a sub-track's parent and which plugin output port it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubTrackLink {
    pub parent_track_id: TrackId,
    /// Index into the plugin's declared output-port layout. `0` is the
    /// parent's own main output — never used by a sub-track. Sub-tracks
    /// always have `output_port_index >= 1`.
    pub output_port_index: u32,
}

impl TrackState {
    /// New audio track with default settings. `order` comes from the
    /// caller (usually `next_track_order`). The default name uses the
    /// 1-based order so the user sees "Track 1", "Track 2", ... rather
    /// than the engine's internal id (which lives in the billions for
    /// auto-allocated tracks).
    pub fn new_audio(id: TrackId, order: usize) -> Self {
        Self {
            id,
            name: format!("Track {}", order + 1),
            volume: 0.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            fx_bypassed: false,
            order,
            record_armed: false,
            monitor_enabled: false,
            mono: true,
            input_device_name: None,
            input_port_index: 0,
            plugins: Vec::new(),
            level_l: 0.0,
            level_r: 0.0,
            track_type: TrackType::Audio,
            output: TrackOutput::Master,
            instrument_type: InstrumentType::Synth,
            instrument_icon: InstrumentIcon::Music,
            role: None,
            sub_track: None,
            midi_input_device: None,
            midi_input_channel: None,
            midi_output_device: None,
            midi_output_channel: None,
        }
    }

    /// New instrument track with default settings.
    pub fn new_instrument(id: TrackId, order: usize) -> Self {
        Self {
            id,
            name: format!("Instrument {}", order + 1),
            volume: 0.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            fx_bypassed: false,
            order,
            record_armed: false,
            monitor_enabled: false,
            mono: false,
            input_device_name: None,
            input_port_index: 0,
            plugins: Vec::new(),
            level_l: 0.0,
            level_r: 0.0,
            track_type: TrackType::Instrument,
            output: TrackOutput::Master,
            instrument_type: InstrumentType::Synth,
            instrument_icon: InstrumentIcon::Music,
            role: None,
            sub_track: None,
            midi_input_device: None,
            midi_input_channel: None,
            midi_output_device: None,
            midi_output_channel: None,
        }
    }

    /// New vocal track. Behaves engine-side like an instrument track for
    /// live MIDI input + recording (so users can capture a melody from a
    /// keyboard), but plays back pre-rendered audio clips from the SVS
    /// pipeline. Defaults to a microphone icon and the `Vocal` role hint.
    pub fn new_vocal(id: TrackId, order: usize) -> Self {
        Self {
            id,
            name: format!("Vocal {}", order + 1),
            volume: 0.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            fx_bypassed: false,
            order,
            record_armed: false,
            monitor_enabled: false,
            mono: true,
            input_device_name: None,
            input_port_index: 0,
            plugins: Vec::new(),
            level_l: 0.0,
            level_r: 0.0,
            track_type: TrackType::Vocal,
            output: TrackOutput::Master,
            instrument_type: InstrumentType::Synth,
            instrument_icon: InstrumentIcon::Microphone,
            role: None,
            sub_track: None,
            midi_input_device: None,
            midi_input_channel: None,
            midi_output_device: None,
            midi_output_channel: None,
        }
    }

    /// New sub-track driven by a parent instrument plugin's output port.
    pub fn new_sub_track(
        id: TrackId,
        order: usize,
        name: String,
        parent_track_id: TrackId,
        output_port_index: u32,
    ) -> Self {
        let mut t = Self::new_instrument(id, order);
        t.name = name;
        t.sub_track = Some(SubTrackLink {
            parent_track_id,
            output_port_index,
        });
        t
    }
}

impl BusState {
    pub fn new(id: BusId, order: usize, name: String) -> Self {
        Self {
            id,
            name,
            order,
            volume: 0.0,
            pan: 0.0,
            muted: false,
            fx_bypassed: false,
            plugins: Vec::new(),
            level_l: 0.0,
            level_r: 0.0,
        }
    }
}

impl PluginSlotState {
    pub fn new(
        instance_id: PluginInstanceId,
        plugin_name: String,
        clap_plugin_id: String,
        clap_file_path: String,
        params: Vec<ParamInfo>,
        has_gui: bool,
    ) -> Self {
        Self {
            instance_id,
            plugin_name,
            clap_plugin_id,
            clap_file_path,
            params,
            custom: PluginCustomState::Generic,
            has_gui,
            editor_open: false,
        }
    }
}

/// GUI-side bus state.
#[derive(Debug, Clone)]
pub struct BusState {
    pub id: BusId,
    pub name: String,
    pub order: usize,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    /// When true, every plugin in this bus's FX chain is bypassed.
    pub fx_bypassed: bool,
    pub plugins: Vec<PluginSlotState>,
    pub level_l: f32,
    pub level_r: f32,
}

/// GUI-side MIDI clip state.
#[derive(Debug, Clone)]
pub struct MidiClipState {
    pub id: ClipId,
    pub track_id: TrackId,
    pub start_sample: SamplePos,
    pub duration_ticks: u64,
    pub name: String,
    pub notes: Vec<MidiNote>,
    pub trim_start_ticks: u64,
    pub trim_end_ticks: u64,
}

/// GUI-side plugin instance state.
#[derive(Debug, Clone)]
pub struct PluginSlotState {
    pub instance_id: PluginInstanceId,
    pub plugin_name: String,
    pub clap_plugin_id: String,
    pub clap_file_path: String,
    pub params: Vec<ParamInfo>,
    pub custom: PluginCustomState,
    /// Whether the plugin exposes a CLAP_EXT_GUI editor.
    pub has_gui: bool,
    /// Whether the host has currently opened the plugin's editor window.
    pub editor_open: bool,
}

/// Plugin-specific GUI state for bundled plugins. Currently a single
/// variant; kept as an enum so future plugins can add their own inline
/// state without reshaping `PluginSlotState`.
#[derive(Debug, Clone)]
pub enum PluginCustomState {
    Generic,
}

/// GUI-side clip state.
#[derive(Debug, Clone)]
pub struct ClipState {
    pub id: ClipId,
    pub track_id: TrackId,
    pub start_sample: SamplePos,
    pub duration_samples: u64,
    pub name: String,
    /// Total raw audio frames (before trim). Used for trim bounds.
    pub total_frames: u64,
    pub trim_start_frames: u64,
    pub trim_end_frames: u64,
    /// Downsampled waveform peaks: (min, max) per chunk of frames.
    pub waveform_peaks: Vec<(f32, f32)>,
}

#[derive(Debug, Clone)]
pub struct MidiClipDragState {
    pub clip_id: ClipId,
    pub grab_offset_x: f32,
    pub original_track_id: TrackId,
    pub current_x: f32,
    pub current_y: f32,
}

#[derive(Debug, Clone)]
pub struct MidiClipTrimState {
    pub clip_id: ClipId,
    pub edge: ClipEdge,
    pub original_start_sample: SamplePos,
    pub original_duration_ticks: u64,
    pub original_trim_start_ticks: u64,
    pub original_trim_end_ticks: u64,
    pub anchor_x: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Arrange,
    Mixer,
    Compose,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoopDragTarget {
    In,
    Out,
}

/// State for the MIDI piano roll editor.
#[derive(Debug, Clone)]
pub struct MidiEditorState {
    pub clip_id: ClipId,
    pub track_id: TrackId,
    pub scroll_y: f32,
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub snap_ticks: u64,
    pub selected_note: Option<usize>,
}

// ---------------------------------------------------------------------------
// Resonance sub-state groupings
// ---------------------------------------------------------------------------

/// Transport, tempo, metronome, and loop range — everything the play head
/// and the tempo engine depend on. Held as a sub-struct on `Resonance` so
/// handlers that only care about transport can take `&mut TransportState`.
#[derive(Debug, Clone)]
pub struct TransportState {
    pub playing: bool,
    pub recording: bool,
    pub recording_start_sample: u64,
    pub playhead: u64,
    pub bpm: f32,
    pub bpm_input: String,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
    pub metronome_enabled: bool,
    /// Number of bars the metronome counts in before playback/recording
    /// starts. 0 disables the pre-count.
    pub precount_bars: u8,
    pub loop_enabled: bool,
    pub loop_in: u64,
    pub loop_out: u64,
    pub loop_range_set: bool,
    pub dragging_loop: Option<LoopDragTarget>,
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            playing: false,
            recording: false,
            recording_start_sample: 0,
            playhead: 0,
            bpm: 120.0,
            bpm_input: "120".to_string(),
            time_sig_num: 4,
            time_sig_den: 4,
            metronome_enabled: false,
            precount_bars: 2,
            loop_enabled: false,
            loop_in: 0,
            loop_out: 0,
            loop_range_set: false,
            dragging_loop: None,
        }
    }
}

/// Horizontal and vertical scroll position of the arrange-view timeline.
/// `viewport_width` / `timeline_content_width` / `_height` are reported back
/// from the canvas after layout.
#[derive(Debug, Clone)]
pub struct ArrangeViewport {
    /// Horizontal zoom in pixels per second.
    pub zoom: f32,
    pub scroll_offset: f32,
    pub scroll_offset_y: f32,
    pub viewport_width: f32,
    pub timeline_content_width: f32,
    pub timeline_content_height: f32,
    /// Whether the global tracks area (tempo, time signature) is expanded.
    pub global_tracks_expanded: bool,
}

impl Default for ArrangeViewport {
    fn default() -> Self {
        Self {
            zoom: 100.0,
            scroll_offset: 0.0,
            scroll_offset_y: 0.0,
            viewport_width: 1000.0,
            timeline_content_width: 1000.0,
            timeline_content_height: 0.0,
            global_tracks_expanded: false,
        }
    }
}

/// Which global track lane an event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalTrackKind {
    Tempo,
    Signature,
}

/// A selected event on a global track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedGlobalEvent {
    pub kind: GlobalTrackKind,
    /// Index into the corresponding events vec.
    pub index: usize,
}

/// Transient clip interaction state: current selection, active drag/trim,
/// and the open MIDI editor if any.
#[derive(Debug, Default)]
pub struct ClipInteractionState {
    pub selected_clip: Option<ClipId>,
    pub selected_midi_clip: Option<ClipId>,
    /// Currently selected (highlighted) track in the arrange view.
    pub selected_track: Option<TrackId>,
    pub clip_drag: Option<ClipDragState>,
    pub clip_trim: Option<ClipTrimState>,
    pub midi_clip_drag: Option<MidiClipDragState>,
    pub midi_clip_trim: Option<MidiClipTrimState>,
    pub editing_midi_clip: Option<MidiEditorState>,
    /// Currently selected event on a global track (tempo or signature).
    pub selected_global_event: Option<SelectedGlobalEvent>,
}

/// Project save/load and offline-bounce progress state.
#[derive(Default)]
pub struct ProjectIoState {
    pub project_path: Option<std::path::PathBuf>,
    pub save_state: Option<crate::project::SaveCollector>,
    pub loading: bool,
    pub pending_load: Option<Box<crate::project::LoadedProject>>,
    /// Runtime-only state to re-apply after an undo/redo restore, once
    /// `replay_loaded_project` has rebuilt the declarative project.
    /// `None` for a normal project load, `Some` for undo/redo.
    pub pending_undo_extras: Option<crate::undo::UndoExtras>,
    pub bouncing: bool,
    /// When false, the startup modal is shown and interactive
    /// messages are dropped. Flipped true on successful load or
    /// on the first successful save of a new project.
    pub has_active_project: bool,
    /// Recent-projects list, loaded from disk on startup and
    /// refreshed whenever an entry is added.
    pub recent_projects: Vec<crate::recent::RecentEntry>,
}

/// Pure UI state for the mixer view and its menus.
#[derive(Debug, Default)]
pub struct MixerUiState {
    pub selected_plugin: Option<PluginInstanceId>,
    pub expanded_sub_track_parents: std::collections::HashSet<TrackId>,
    pub add_track_menu_open: bool,
    pub settings_open: bool,
}

/// Tracks + busses + id counters. Central registry that handlers borrow to
/// mutate track/bus/plugin state without touching the rest of `Resonance`.
#[derive(Debug, Default)]
pub struct TrackRegistry {
    pub tracks: Vec<TrackState>,
    pub busses: Vec<BusState>,
    pub next_track_order: usize,
    pub next_bus_order: usize,
    /// Id counter for auto-created sub-tracks. Lives in a high numeric
    /// range so it never collides with engine-allocated track ids
    /// (engine tracks count up from 1).
    pub next_sub_track_id: u64,
}

impl TrackRegistry {
    /// `tracks` and `busses` are kept sorted by `.order` as an invariant
    /// so the view layer iterates them directly without an O(n log n)
    /// resort per frame. Every mutation that pushes, removes, or changes
    /// `.order` MUST call `resort_tracks` / `resort_busses` afterwards,
    /// or the on-screen ordering will drift from the data model.
    pub fn sorted_tracks(&self) -> Vec<&TrackState> {
        debug_assert!(
            self.tracks.windows(2).all(|w| w[0].order <= w[1].order),
            "TrackRegistry.tracks must be sorted by .order — call resort_tracks() after the mutation that ordered last"
        );
        self.tracks.iter().collect()
    }

    pub fn sorted_busses(&self) -> Vec<&BusState> {
        debug_assert!(
            self.busses.windows(2).all(|w| w[0].order <= w[1].order),
            "TrackRegistry.busses must be sorted by .order — call resort_busses() after the mutation that ordered last"
        );
        self.busses.iter().collect()
    }

    /// Re-establishes the sorted-by-order invariant on `tracks`. Cheap
    /// for already-sorted slices (Rust's sort is adaptive), so call this
    /// liberally after any mutation that might break order.
    pub fn resort_tracks(&mut self) {
        self.tracks.sort_by_key(|t| t.order);
    }

    /// Re-establishes the sorted-by-order invariant on `busses`.
    pub fn resort_busses(&mut self) {
        self.busses.sort_by_key(|b| b.order);
    }

    /// Run `f` on the track with the given id, returning whatever `f`
    /// returns. `None` if the track doesn't exist.
    pub fn with_track_mut<R>(
        &mut self,
        id: TrackId,
        f: impl FnOnce(&mut TrackState) -> R,
    ) -> Option<R> {
        self.tracks.iter_mut().find(|t| t.id == id).map(f)
    }

    /// Run `f` on the bus with the given id, returning whatever `f` returns.
    /// `None` if the bus doesn't exist.
    pub fn with_bus_mut<R>(&mut self, id: BusId, f: impl FnOnce(&mut BusState) -> R) -> Option<R> {
        self.busses.iter_mut().find(|b| b.id == id).map(f)
    }

    /// Allocate a fresh id from `next_sub_track_id`, skipping past any
    /// id already taken by a track in the registry. Used for sub-tracks
    /// and for bounce-target tracks that share this counter. Without the
    /// skip, a collision with an engine-allocated id silently overwrites
    /// the other track in the engine's hashmap (or no-ops the new one,
    /// depending on which command ran first).
    pub fn allocate_sub_track_id(&mut self) -> TrackId {
        loop {
            let candidate = self.next_sub_track_id;
            self.next_sub_track_id += 1;
            if !self.tracks.iter().any(|t| t.id == candidate) {
                return candidate;
            }
        }
    }
}
