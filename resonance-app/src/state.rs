/// GUI-side state types for the Resonance application.
use resonance_audio::types::*;

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
    pub plugins: Vec<PluginSlotState>,
    /// Current VU meter level for left channel (linear amplitude).
    pub level_l: f32,
    /// Current VU meter level for right channel (linear amplitude).
    pub level_r: f32,
    pub track_type: TrackType,
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

/// Plugin-specific GUI state for bundled plugins.
#[derive(Debug, Clone)]
pub enum PluginCustomState {
    Generic,
    Drums(resonance_drums::ui::DrumsUiState),
    Amp(resonance_amp::ui::AmpUiState),
    Ir(resonance_ir::ui::IrUiState),
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
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PunchDragTarget {
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
