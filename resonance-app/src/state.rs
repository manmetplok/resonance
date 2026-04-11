/// GUI-side state types for the Resonance application.
use resonance_audio::types::*;
use serde::{Deserialize, Serialize};

/// Sub-type of an instrument track, surfaced in the Compose tab. Only used
/// for display and icon defaulting — the audio engine itself treats all
/// instrument tracks identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InstrumentType {
    Synth,
    Drum,
}

impl Default for InstrumentType {
    fn default() -> Self {
        InstrumentType::Synth
    }
}

impl InstrumentType {
    pub const ALL: [InstrumentType; 2] = [InstrumentType::Synth, InstrumentType::Drum];

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

/// Icon shown next to the instrument name in the Compose tab. Backed by a
/// Font Awesome glyph; kept in an enum so the persisted value survives
/// font-file renames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InstrumentIcon {
    Music,
    Drum,
    Guitar,
    Microphone,
    WaveSquare,
    CompactDisc,
    Sliders,
}

impl Default for InstrumentIcon {
    fn default() -> Self {
        InstrumentIcon::Music
    }
}

impl InstrumentIcon {
    pub const ALL: [InstrumentIcon; 7] = [
        InstrumentIcon::Music,
        InstrumentIcon::Drum,
        InstrumentIcon::Guitar,
        InstrumentIcon::Microphone,
        InstrumentIcon::WaveSquare,
        InstrumentIcon::CompactDisc,
        InstrumentIcon::Sliders,
    ];

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

    /// Default icon for the given instrument type.
    pub fn default_for(ty: InstrumentType) -> Self {
        match ty {
            InstrumentType::Synth => InstrumentIcon::Music,
            InstrumentType::Drum => InstrumentIcon::Drum,
        }
    }
}

impl std::fmt::Display for InstrumentIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ClipDragState {
    pub clip_id: ClipId,
    pub grab_offset_x: f32,
    pub original_start_sample: SamplePos,
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
    /// When set, this track is a sub-track that reads one non-main output
    /// port from its parent track's instrument plugin. Sub-tracks have
    /// their own fader / pan / mute / bus routing, but no clips, no plugin
    /// chain of their own, and no record arm — they're fed entirely from
    /// the parent plugin's fan-out. `None` on all normal tracks.
    pub sub_track: Option<SubTrackLink>,
}

/// Identifies a sub-track's parent and which plugin output port it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubTrackLink {
    pub parent_track_id: TrackId,
    /// Index into the plugin's declared output-port layout. `0` is the
    /// parent's own main output — never used by a sub-track. Sub-tracks
    /// always have `output_port_index >= 1`.
    pub output_port_index: u32,
}

impl TrackState {
    /// New audio track with default settings. `order` comes from the
    /// caller (usually `next_track_order`).
    pub fn new_audio(id: TrackId, order: usize) -> Self {
        Self {
            id,
            name: format!("Track {}", id),
            volume: 0.0,
            pan: 0.0,
            muted: false,
            soloed: false,
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
            sub_track: None,
        }
    }

    /// New instrument track with default settings.
    pub fn new_instrument(id: TrackId, order: usize) -> Self {
        Self {
            id,
            name: format!("Instrument {}", id),
            volume: 0.0,
            pan: 0.0,
            muted: false,
            soloed: false,
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
            sub_track: None,
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
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub snap_ticks: u64,
    pub selected_note: Option<usize>,
}
