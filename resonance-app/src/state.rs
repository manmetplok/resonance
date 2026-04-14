/// GUI-side state types for the Resonance application.
use resonance_audio::types::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Global track events (tempo & time signature changes)
// ---------------------------------------------------------------------------

/// A tempo change on the tempo track. Bar 0 is always present as the
/// initial project tempo; additional events mark tempo changes at later bars.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoEvent {
    /// 0-based bar number where this tempo takes effect.
    pub bar: u32,
    pub bpm: f32,
}

/// A time signature change on the signature track. Bar 0 is always present.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureEvent {
    /// 0-based bar number where this signature takes effect.
    pub bar: u32,
    pub numerator: u8,
    pub denominator: u8,
}

/// Return the interpolated BPM at a fractional bar position.
/// Between events at different bars the BPM ramps linearly.
/// When multiple events share the same bar (step change) the last
/// value at that bar wins.
pub fn bpm_at_bar(bar: f64, tempo_events: &[TempoEvent]) -> f64 {
    if tempo_events.is_empty() {
        return 120.0;
    }
    let mut prev_bpm = tempo_events[0].bpm as f64;
    let mut prev_bar = tempo_events[0].bar as f64;
    let mut next: Option<&TempoEvent> = None;

    for e in tempo_events {
        if (e.bar as f64) <= bar {
            prev_bpm = e.bpm as f64;
            prev_bar = e.bar as f64;
        } else {
            next = Some(e);
            break;
        }
    }

    // Interpolate if we're between two events at different bars.
    if let Some(ne) = next {
        if prev_bar < bar {
            let t = (bar - prev_bar) / (ne.bar as f64 - prev_bar);
            return prev_bpm + (ne.bpm as f64 - prev_bpm) * t;
        }
    }

    prev_bpm
}

/// Return the arrival BPM at a bar — the ramp target approaching from
/// the left. For step changes (multiple events at one bar), returns
/// the first event's value.
pub fn arrival_bpm_at_bar_gui(bar: u32, tempo_events: &[TempoEvent]) -> f64 {
    if tempo_events.is_empty() {
        return 120.0;
    }
    for e in tempo_events {
        if e.bar == bar {
            return e.bpm as f64;
        }
        if e.bar > bar {
            break;
        }
    }
    bpm_at_bar(bar as f64, tempo_events)
}

/// Compute the average BPM across a bar for smooth sample accumulation.
/// Uses departure at bar start, arrival at bar end.
pub fn avg_bpm_for_bar(bar: u32, tempo_events: &[TempoEvent]) -> f64 {
    let bpm_start = bpm_at_bar(bar as f64, tempo_events);
    let bpm_end = arrival_bpm_at_bar_gui(bar + 1, tempo_events);
    (bpm_start + bpm_end) / 2.0
}

/// Compute the sample position of the start of a given bar number,
/// considering all tempo and signature changes with smooth linear BPM
/// interpolation between events.
pub fn bar_to_sample(
    bar: u32,
    tempo_events: &[TempoEvent],
    signature_events: &[SignatureEvent],
    sample_rate: u32,
) -> u64 {
    let sr = sample_rate as f64;
    let mut sample_pos: f64 = 0.0;
    let mut cur_num = signature_events.first().map(|e| e.numerator).unwrap_or(4);
    let mut si: usize = if signature_events.first().map(|e| e.bar) == Some(0) { 1 } else { 0 };

    for b in 0..bar {
        while let Some(e) = signature_events.get(si) {
            if e.bar == b { cur_num = e.numerator; si += 1; } else { break; }
        }
        let bpm = avg_bpm_for_bar(b, tempo_events);
        let samples_per_beat = sr * 60.0 / bpm;
        sample_pos += samples_per_beat * cur_num as f64;
    }

    sample_pos.round() as u64
}

/// Find the tempo and time signature active at a given sample position,
/// with smooth linear BPM interpolation between tempo events.
pub fn tempo_at_sample(
    sample_pos: u64,
    tempo_events: &[TempoEvent],
    signature_events: &[SignatureEvent],
    sample_rate: u32,
) -> (f32, u8, u8) {
    let sr = sample_rate as f64;
    let mut acc: f64 = 0.0;
    let mut cur_num = signature_events.first().map(|e| e.numerator).unwrap_or(4);
    let mut cur_den = signature_events.first().map(|e| e.denominator).unwrap_or(4);
    let mut si: usize = if signature_events.first().map(|e| e.bar) == Some(0) { 1 } else { 0 };

    for b in 0u32.. {
        while let Some(e) = signature_events.get(si) {
            if e.bar == b { cur_num = e.numerator; cur_den = e.denominator; si += 1; }
            else { break; }
        }
        let bpm = avg_bpm_for_bar(b, tempo_events);
        let samples_per_beat = sr * 60.0 / bpm;
        let samples_per_bar = samples_per_beat * cur_num as f64;

        if acc + samples_per_bar > sample_pos as f64 {
            // Compute the exact fractional bar position and return
            // the continuously interpolated BPM at that point.
            let frac = (sample_pos as f64 - acc) / samples_per_bar;
            let exact_bpm = bpm_at_bar(b as f64 + frac, tempo_events);
            return (exact_bpm as f32, cur_num, cur_den);
        }
        acc += samples_per_bar;
        if b > 20_000 { break; }
    }

    let bpm = tempo_events.last().map(|e| e.bpm).unwrap_or(120.0);
    (bpm, cur_num, cur_den)
}

/// Convert a sample position to a (bar, fraction) pair where fraction
/// is 0.0..1.0 within the bar. Used for pixel→bar mapping during drag.
pub fn sample_to_bar(
    sample_pos: u64,
    tempo_events: &[TempoEvent],
    signature_events: &[SignatureEvent],
    sample_rate: u32,
) -> (u32, f64) {
    let sr = sample_rate as f64;
    let mut acc: f64 = 0.0;
    let mut cur_num = signature_events.first().map(|e| e.numerator).unwrap_or(4);
    let mut si: usize = if signature_events.first().map(|e| e.bar) == Some(0) { 1 } else { 0 };

    for b in 0u32.. {
        while let Some(e) = signature_events.get(si) {
            if e.bar == b { cur_num = e.numerator; si += 1; } else { break; }
        }
        let bpm = avg_bpm_for_bar(b, tempo_events);
        let samples_per_beat = sr * 60.0 / bpm;
        let samples_per_bar = samples_per_beat * cur_num as f64;

        if acc + samples_per_bar > sample_pos as f64 {
            let frac = (sample_pos as f64 - acc) / samples_per_bar;
            return (b, frac);
        }
        acc += samples_per_bar;
        if b > 20_000 { break; }
    }

    (0, 0.0)
}

/// Convert a tick offset from a clip's start sample to an absolute sample,
/// integrating tempo changes. GUI-side equivalent of `TempoMap::tick_to_abs_sample`.
pub fn tick_to_abs_sample(
    clip_start: u64,
    tick_offset: u64,
    tempo_events: &[TempoEvent],
    signature_events: &[SignatureEvent],
    sample_rate: u32,
) -> u64 {
    if tick_offset == 0 {
        return clip_start;
    }
    let sr = sample_rate as f64;
    let ppq = TICKS_PER_QUARTER_NOTE as f64;

    if tempo_events.len() <= 1 {
        let bpm = tempo_events.first().map(|e| e.bpm as f64).unwrap_or(120.0);
        let spt = (sr * 60.0 / bpm) / ppq;
        return clip_start + (tick_offset as f64 * spt) as u64;
    }

    let mut sample_pos: f64 = 0.0;
    let mut tick_pos: f64 = 0.0;
    let mut cur_num = signature_events.first().map(|e| e.numerator).unwrap_or(4);
    let mut si: usize = if signature_events.first().map(|e| e.bar) == Some(0) { 1 } else { 0 };
    let mut clip_tick: f64 = 0.0;
    let mut found_start = false;

    for b in 0u32..20_001 {
        while let Some(e) = signature_events.get(si) {
            if e.bar == b { cur_num = e.numerator; si += 1; } else { break; }
        }
        let ticks_in_bar = cur_num as f64 * ppq;
        let bpm_s = bpm_at_bar(b as f64, tempo_events);
        let bpm_e = arrival_bpm_at_bar_gui(b + 1, tempo_events);
        let avg = (bpm_s + bpm_e) / 2.0;
        let bar_samples = sr * 60.0 / avg * cur_num as f64;

        if !found_start && (clip_start as f64) < sample_pos + bar_samples {
            let sf = if bar_samples > 0.0 {
                (clip_start as f64 - sample_pos) / bar_samples
            } else { 0.0 };
            clip_tick = tick_pos + sample_frac_to_tick_frac(sf, bpm_s, bpm_e) * ticks_in_bar;
            found_start = true;
        }

        if found_start {
            let target = clip_tick + tick_offset as f64;
            if target < tick_pos + ticks_in_bar {
                let tf = (target - tick_pos) / ticks_in_bar;
                let sf = tick_frac_to_sample_frac(tf, bpm_s, bpm_e);
                return (sample_pos + sf * bar_samples) as u64;
            }
        }

        sample_pos += bar_samples;
        tick_pos += ticks_in_bar;
    }

    // Fallback
    let bpm = tempo_events.last().map(|e| e.bpm as f64).unwrap_or(120.0);
    let spt = (sr * 60.0 / bpm) / ppq;
    if !found_start { clip_tick = tick_pos; }
    (sample_pos + (clip_tick + tick_offset as f64 - tick_pos) * spt) as u64
}

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
    pub const ALL: [TrackRole; 3] = [TrackRole::Pad, TrackRole::Bass, TrackRole::Lead];

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
    pub scroll_x: f32,
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
            global_tracks_expanded: true,
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
    pub collapsed_sub_track_parents: std::collections::HashSet<TrackId>,
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
    pub fn sorted_tracks(&self) -> Vec<&TrackState> {
        let mut v: Vec<&TrackState> = self.tracks.iter().collect();
        v.sort_by_key(|t| t.order);
        v
    }

    pub fn sorted_busses(&self) -> Vec<&BusState> {
        let mut v: Vec<&BusState> = self.busses.iter().collect();
        v.sort_by_key(|b| b.order);
        v
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
    pub fn with_bus_mut<R>(
        &mut self,
        id: BusId,
        f: impl FnOnce(&mut BusState) -> R,
    ) -> Option<R> {
        self.busses.iter_mut().find(|b| b.id == id).map(f)
    }

}
