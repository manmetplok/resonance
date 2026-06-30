/// Project save/load for the Resonance application.
///
/// v2 on-disk layout:
///
/// ```text
/// MyProject.rproj/
///   project.json               — metadata, no inline clip samples
///   audio/clip_{id}.wav        — 32-bit float stereo WAVs
///   midi/clip_{id}.mid         — Format 0 Standard MIDI files
///   plugins/plugin_{id}.bin    — opaque CLAP state blobs
/// ```
///
/// Audio WAVs are streamed there during recording and memory-mapped
/// at load, so even very long takes never materialise as a
/// contiguous in-RAM buffer. MIDI clips persist as real `.mid` files
/// so projects interchange cleanly with other tools.
///
/// This version hard-breaks v1 projects — there is no `.raw` →
/// `.wav` migration path. Users on v1 need to open the project with
/// a prior build and re-export.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use resonance_audio::midi_io;
use resonance_audio::types::{ClipId, FadeCurve, MidiNote, PluginInstanceId};

pub mod sections;
pub use sections::{ProjectSectionChord, ProjectSectionDefinition, ProjectSectionPlacement};

pub const PROJECT_FORMAT_VERSION: u32 = 2;

/// File name of the canonical project-metadata document inside a `.rproj`
/// directory. A manual save (over)writes this file.
pub const PROJECT_JSON: &str = "project.json";

/// File name of the autosave snapshot, written alongside
/// [`PROJECT_JSON`]. Kept as a *separate* side file (never overwriting
/// the canonical `project.json`) so a crash mid-autosave can only ever
/// truncate the snapshot, leaving the last manual save fully intact.
/// See epic #32 / doc #171 "Autosave triggering".
pub const AUTOSAVE_JSON: &str = "project.autosave.json";

/// On-disk project format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFile {
    pub version: u32,
    pub sample_rate: u32,
    pub bpm: f32,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
    pub metronome_enabled: bool,
    pub master_volume: f32,
    /// FX plugins inserted on the master bus. Empty on legacy projects.
    #[serde(default)]
    pub master_plugins: Vec<ProjectPlugin>,
    /// Whether the master FX chain is bypassed. `false` on legacy projects.
    #[serde(default)]
    pub master_fx_bypassed: bool,
    #[serde(alias = "punch_enabled")]
    pub loop_enabled: bool,
    #[serde(alias = "punch_in")]
    pub loop_in: u64,
    #[serde(alias = "punch_out")]
    pub loop_out: u64,
    pub tracks: Vec<ProjectTrack>,
    pub clips: Vec<ProjectClip>,
    #[serde(default)]
    pub midi_clips: Vec<ProjectMidiClip>,
    #[serde(default)]
    pub busses: Vec<ProjectBus>,
    #[serde(default)]
    pub section_definitions: Vec<ProjectSectionDefinition>,
    #[serde(default)]
    pub section_placements: Vec<ProjectSectionPlacement>,
    /// Tempo change events on the tempo track. Empty on legacy projects
    /// (a single event at bar 0 with the project BPM is inferred).
    #[serde(default)]
    pub tempo_events: Vec<crate::state::TempoEvent>,
    /// Time signature change events. Empty on legacy projects.
    #[serde(default)]
    pub signature_events: Vec<crate::state::SignatureEvent>,
    /// Whether the engine should emit MIDI clock to a hardware port.
    #[serde(default)]
    pub midi_clock_send_enabled: bool,
    /// Hardware MIDI output port carrying the master clock.
    #[serde(default)]
    pub midi_clock_send_device: Option<String>,
    /// Whether the engine should slave to incoming MIDI clock.
    #[serde(default)]
    pub midi_clock_recv_enabled: bool,
    /// Hardware MIDI input port carrying the master clock.
    #[serde(default)]
    pub midi_clock_recv_device: Option<String>,
    /// Project-scoped drum groups. Each group owns its grid/cycle/phase
    /// and per-pad patterns; the audio rendering reads these to materialise
    /// the drum track's MIDI clip on every section placement. Empty on
    /// legacy projects (which then get the built-in default kit/snare/hat
    /// layout the first time a new session opens).
    ///
    /// **Legacy field.** New projects persist the
    /// [`drum_patterns`](Self::drum_patterns) bank instead and leave this
    /// empty. On load we promote a non-empty legacy list into a single
    /// "Main" entry in the bank so projects round-trip cleanly.
    #[serde(default)]
    pub drum_groups: Vec<crate::compose::DrumGroup>,
    /// Project-scoped drum pattern bank. Each pattern owns its own set
    /// of [`DrumGroup`]s; section definitions reference patterns by id
    /// via `drum_pattern_id`. Empty on legacy projects (the loader then
    /// builds a single-pattern bank from `drum_groups`).
    #[serde(default)]
    pub drum_patterns: Vec<crate::compose::DrumPattern>,
    /// Loaded A/B reference tracks (external mastered tracks the user
    /// auditions against the mix). Empty on legacy projects. These are
    /// **monitor-only** — see [`ProjectReferenceSettings::monitor_only`] —
    /// and never participate in any render or export.
    #[serde(default)]
    pub references: Vec<ProjectReference>,
    /// Panel-level A/B settings (active selection, monitored source,
    /// loudness-match / trim, loop-to-mix). Defaults to a neutral,
    /// mix-monitoring state on legacy projects.
    #[serde(default)]
    pub reference_settings: ProjectReferenceSettings,
    /// Arrangement markers on the timeline. Empty on legacy projects.
    #[serde(default)]
    pub arrangement_markers: Vec<crate::state::ArrangementMarker>,
    /// Imported media-pool assets (doc #175). Each entry is an audio file
    /// transcoded into the project's `audio/` directory that clips
    /// reference by id via [`ProjectClip::asset_ref`]. Empty on legacy
    /// projects (which had no media pool); such projects load with an
    /// empty pool and every clip's `asset_ref` left `None`.
    #[serde(default)]
    pub pool_assets: Vec<ProjectPoolAsset>,
    /// Project groove library: user-extracted
    /// [`GrooveTemplate`](resonance_audio::quantize::GrooveTemplate)s
    /// (ba todo #395) the user saved for reuse, each with a stable
    /// per-project id and display name. Stock grooves are *not* duplicated
    /// here — they live in code and are referenced by index from
    /// [`quantize_settings`](Self::quantize_settings). Empty on legacy
    /// projects.
    #[serde(default)]
    pub groove_library: Vec<crate::state::UserGroove>,
    /// Last-used MIDI quantize / humanize settings (ba todo #395),
    /// restored as the quantize panel's defaults on load. Neutral
    /// defaults on legacy projects.
    #[serde(default)]
    pub quantize_settings: crate::state::QuantizeSettings,
    /// Performance-mode footer selection (epic #11, todo #312): which
    /// instrument tuning the live fingering diagrams are drawn for and the
    /// capo offset. Defaults to Guitar 6 / no capo on legacy projects that
    /// predate Performance mode.
    #[serde(default)]
    pub performance: ProjectPerformance,
}

/// An empty project at the current format version with neutral
/// defaults (44.1 kHz, 120 BPM, 4/4). Purely a convenience for tests
/// and `..Default::default()` literals — deserialization is governed
/// by the per-field `#[serde(default)]` attributes above, not by this
/// impl.
impl Default for ProjectFile {
    fn default() -> Self {
        Self {
            version: PROJECT_FORMAT_VERSION,
            sample_rate: 44100,
            bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
            metronome_enabled: false,
            master_volume: 0.0,
            master_plugins: Vec::new(),
            master_fx_bypassed: false,
            loop_enabled: false,
            loop_in: 0,
            loop_out: 0,
            tracks: Vec::new(),
            clips: Vec::new(),
            midi_clips: Vec::new(),
            busses: Vec::new(),
            section_definitions: Vec::new(),
            section_placements: Vec::new(),
            tempo_events: Vec::new(),
            signature_events: Vec::new(),
            midi_clock_send_enabled: false,
            midi_clock_send_device: None,
            midi_clock_recv_enabled: false,
            midi_clock_recv_device: None,
            drum_groups: Vec::new(),
            drum_patterns: Vec::new(),
            references: Vec::new(),
            reference_settings: ProjectReferenceSettings::default(),
            arrangement_markers: Vec::new(),
            pool_assets: Vec::new(),
            groove_library: Vec::new(),
            quantize_settings: crate::state::QuantizeSettings::default(),
            performance: ProjectPerformance::default(),
        }
    }
}

/// Persisted Performance-mode footer selection (epic #11, todo #312):
/// the instrument tuning the live fingering diagrams are drawn for and the
/// capo offset. Mirrors the durable subset of
/// [`crate::state::PerformanceState`].
///
/// The tuning is stored by its stable display name
/// ([`Tuning::name`](resonance_music_theory::Tuning::name)) rather than its
/// `ALL_TUNINGS` index, so a project still resolves to the right instrument
/// if that list is ever reordered or extended. An unrecognised name (a
/// future build's tuning, or a hand-edited file) falls back to the default
/// Guitar 6 on load, mirroring the defensive clamping `PerformanceState`
/// already applies to a stale index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPerformance {
    /// Display name of the selected tuning, e.g. `"Guitar (6-string)"`.
    /// Resolved back to an `ALL_TUNINGS` index on load; an unknown name
    /// falls back to the default (Guitar 6).
    #[serde(default = "default_performance_tuning")]
    pub tuning: String,
    /// Capo position in frets (`0` = no capo).
    #[serde(default)]
    pub capo: u8,
}

/// Default [`ProjectPerformance::tuning`] for projects saved before
/// Performance mode existed: Guitar 6, the first entry in `ALL_TUNINGS` and
/// the footer's default selection.
fn default_performance_tuning() -> String {
    resonance_music_theory::GUITAR_6.name.to_string()
}

impl Default for ProjectPerformance {
    fn default() -> Self {
        Self {
            tuning: default_performance_tuning(),
            capo: 0,
        }
    }
}

/// One persisted A/B reference track. Holds only the durable, on-disk
/// facts: the source path, its display name, the cached integrated
/// loudness (so the loudness readout doesn't blank until the re-decode
/// finishes), and the user's comparison markers. The decoded PCM and
/// waveform overview are intentionally **not** persisted — they are
/// rebuilt by re-issuing `LoadReferenceTrack` on load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectReference {
    /// Absolute path to the source audio file.
    pub path: String,
    /// Display name (file stem unless the engine supplied one).
    pub name: String,
    /// Cached integrated loudness (LUFS) measured during analysis, so the
    /// readout shows a value before the re-decode completes. May be
    /// `-inf` if the original analysis never finished.
    pub integrated_lufs: f32,
    /// User-placed comparison markers, in the order they were saved.
    #[serde(default)]
    pub markers: Vec<ProjectReferenceMarker>,
}

/// A persisted comparison marker on a reference track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectReferenceMarker {
    /// Per-reference marker id.
    pub id: u32,
    /// Position within the reference track, in sample frames.
    pub position_samples: u64,
    /// User-facing label.
    pub label: String,
}

/// Persisted panel-level A/B settings. Mirrors the durable subset of
/// `reference::ReferenceState`, addressing the active reference by its
/// index into [`ProjectFile::references`] rather than by engine id (ids
/// are reallocated on load).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectReferenceSettings {
    /// Always `true`: a sentinel asserting (per design doc #198) that the
    /// persisted reference block is monitor-only and must never reach a
    /// render or export. Kept on disk as self-documenting provenance.
    #[serde(default = "default_true")]
    pub monitor_only: bool,
    /// Index into [`ProjectFile::references`] of the active reference, or
    /// `None` when nothing is selected.
    #[serde(default)]
    pub active: Option<usize>,
    /// Whether the monitored source was the reference (else the mix).
    /// Stored as a bool because the engine `ABSource` type deliberately
    /// carries no serde derive.
    #[serde(default)]
    pub ab_source_is_reference: bool,
    /// Whether the active reference is loudness-matched to the mix.
    #[serde(default)]
    pub loudness_match: bool,
    /// Manual level trim (dB) on top of any loudness match.
    #[serde(default)]
    pub trim_db: f32,
    /// Whether the reference cursor follows the mix transport.
    #[serde(default)]
    pub loop_to_mix: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ProjectReferenceSettings {
    fn default() -> Self {
        Self {
            monitor_only: true,
            active: None,
            ab_source_is_reference: false,
            loudness_match: false,
            trim_db: 0.0,
            loop_to_mix: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectTrack {
    pub id: u64,
    pub name: String,
    pub order: usize,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub soloed: bool,
    /// Whether the track's FX chain is bypassed. Default `false` keeps
    /// legacy projects loadable.
    #[serde(default)]
    pub fx_bypassed: bool,
    pub record_armed: bool,
    pub monitor_enabled: bool,
    pub mono: bool,
    pub input_device_name: Option<String>,
    /// 0-indexed starting input channel on the track's input device.
    /// None on legacy projects (loads as 0, i.e. first channel pair).
    #[serde(default)]
    pub input_port_index: Option<u16>,
    pub plugins: Vec<ProjectPlugin>,
    #[serde(default = "default_track_type")]
    pub track_type: String,
    /// If Some, the track routes to this bus id. None (default on old
    /// projects) means the track routes directly to master.
    #[serde(default)]
    pub output_bus: Option<u64>,
    /// Instrument sub-type (synth/drum) for display in Compose. Default for
    /// legacy projects is `Synth`.
    #[serde(default)]
    pub instrument_type: crate::state::InstrumentType,
    /// Display icon for the instrument. Default for legacy projects is the
    /// icon matching `instrument_type`.
    #[serde(default)]
    pub instrument_icon: crate::state::InstrumentIcon,
    /// Arrangement role for derive-from-chords flows. Legacy projects load
    /// with `None`, meaning the track is not auto-picked by any derive.
    #[serde(default)]
    pub role: Option<crate::state::TrackRole>,
    /// When set, this track is a sub-track driven by a non-main output
    /// port of `parent_track_id`'s instrument plugin. Legacy projects
    /// load with `None` (no sub-tracks existed before this feature).
    #[serde(default)]
    pub sub_track: Option<crate::state::SubTrackLink>,
    /// Hardware MIDI input device name. `None` on legacy projects and
    /// on tracks the user hasn't assigned.
    #[serde(default)]
    pub midi_input_device: Option<String>,
    /// Hardware MIDI input channel filter (0..=15) or omni.
    #[serde(default)]
    pub midi_input_channel: Option<u8>,
    /// Hardware MIDI output device name.
    #[serde(default)]
    pub midi_output_device: Option<String>,
    /// Hardware MIDI output channel (0..=15), or `None` = channel 1.
    #[serde(default)]
    pub midi_output_channel: Option<u8>,
}

/// On-disk bus state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectBus {
    pub id: u64,
    pub name: String,
    pub order: usize,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    #[serde(default)]
    pub fx_bypassed: bool,
    pub plugins: Vec<ProjectPlugin>,
}

fn default_track_type() -> String {
    "audio".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectPlugin {
    pub instance_id: u64,
    pub plugin_name: String,
    pub clap_plugin_id: String,
    pub clap_file_path: String,
    pub state_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectClip {
    pub id: u64,
    pub track_id: u64,
    pub start_sample: u64,
    pub name: String,
    pub total_frames: u64,
    pub trim_start_frames: u64,
    pub trim_end_frames: u64,
    /// Project-relative path to the clip's WAV file, e.g.
    /// `"audio/clip_42.wav"`. Absolute paths are resolved against
    /// the project directory at load time.
    pub audio_file: String,
    /// Id of the media-pool asset this clip was placed from (doc #175),
    /// or `None` for clips that aren't pool imports (recorded takes,
    /// bounces) and for legacy projects that predate the media pool.
    /// Rebuilt onto [`crate::state::ClipState::asset_ref`] on load.
    #[serde(default)]
    pub asset_ref: Option<u64>,
    /// Fade-in length in frames; `0` = no fade (epic #18, doc #156).
    /// Defaults to `0` so projects saved before fades existed load with
    /// no fade-in.
    #[serde(default)]
    pub fade_in_frames: u64,
    /// Curve shaping the fade-in ramp, stored as a short lowercase tag
    /// (see [`fade_curve_tag`]). [`FadeCurve`] carries no serde derive,
    /// so the project layer owns this round-trip. Defaults to the
    /// `EqualPower` tag for older projects.
    #[serde(default = "default_fade_curve_tag")]
    pub fade_in_curve: String,
    /// Fade-out length in frames; `0` = no fade (epic #18, doc #156).
    #[serde(default)]
    pub fade_out_frames: u64,
    /// Curve shaping the fade-out ramp, stored as a tag (see
    /// [`fade_curve_tag`]). Defaults to the `EqualPower` tag.
    #[serde(default = "default_fade_curve_tag")]
    pub fade_out_curve: String,
    /// Per-clip gain in decibels; `0.0` dB = unity (epic #18, doc #156).
    /// Defaults to `0.0` so older projects load at unity gain.
    #[serde(default)]
    pub gain_db: f32,
}

/// Default [`ProjectClip::fade_in_curve`] / [`ProjectClip::fade_out_curve`]
/// tag for projects saved before fade curves existed: the engine default,
/// `EqualPower`. Kept in sync with [`fade_curve_tag`].
fn default_fade_curve_tag() -> String {
    fade_curve_tag(FadeCurve::default()).to_string()
}

/// Serialize a [`FadeCurve`] to the short lowercase tag stored in
/// [`ProjectClip::fade_in_curve`] / [`ProjectClip::fade_out_curve`]. The
/// engine type has no serde derive, so the project layer owns this
/// round-trip. Kept in sync with [`fade_curve_from_tag`].
pub fn fade_curve_tag(curve: FadeCurve) -> &'static str {
    match curve {
        FadeCurve::Linear => "linear",
        FadeCurve::EqualPower => "equal_power",
        FadeCurve::Exp => "exp",
    }
}

/// Parse a fade-curve tag back into a [`FadeCurve`]. Unknown / unexpected
/// tags (a future build's new variant, or a hand-edited file) fall back to
/// [`FadeCurve::default`] so loading never fails on the curve label.
pub fn fade_curve_from_tag(tag: &str) -> FadeCurve {
    match tag {
        "linear" => FadeCurve::Linear,
        "equal_power" => FadeCurve::EqualPower,
        "exp" => FadeCurve::Exp,
        _ => FadeCurve::default(),
    }
}

/// One persisted media-pool asset (doc #175): an imported audio file
/// that clips reference by id. The engine transcodes every import to a
/// project-rate stereo f32 WAV under the project's `audio/` directory;
/// `project_relative_path` points at it (relocatable with the project),
/// while the remaining fields describe the original source for display.
/// The waveform thumbnail and live usage counts are runtime-derived and
/// deliberately *not* persisted — they're rebuilt on load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectPoolAsset {
    /// Stable per-project asset id, matched by [`ProjectClip::asset_ref`].
    pub id: u64,
    /// Project-relative path of the engine-format WAV, e.g.
    /// `"audio/asset_7.wav"`. Resolved against the project directory to
    /// detect whether the backing file is still present.
    pub project_relative_path: String,
    /// Absolute path of the source file the user originally imported
    /// (provenance / relink hint).
    pub original_path: String,
    /// Container/codec family of the original source, stored as a short
    /// lowercase tag (`"wav"`, `"flac"`, `"mp3"`, `"ogg"`, `"aac"`,
    /// `"mp4"`, or `"other"`). `resonance_common::AudioFormat` carries no
    /// serde derive, so it round-trips through this string.
    pub format: String,
    /// Channel count of the original source file.
    pub channels: u16,
    /// Sample rate of the original source file, in Hz.
    pub source_sample_rate: u32,
    /// Per-channel frame count of the imported (project-rate) WAV.
    pub duration_frames: u64,
}

/// Serialize an [`AudioFormat`](resonance_common::AudioFormat) to the
/// short lowercase tag stored in [`ProjectPoolAsset::format`]. The
/// engine type has no serde derive, so the project layer owns this
/// round-trip. Kept in sync with [`audio_format_from_tag`].
pub fn audio_format_tag(format: resonance_common::AudioFormat) -> &'static str {
    use resonance_common::AudioFormat;
    match format {
        AudioFormat::Wav => "wav",
        AudioFormat::Flac => "flac",
        AudioFormat::Mp3 => "mp3",
        AudioFormat::Ogg => "ogg",
        AudioFormat::Aac => "aac",
        AudioFormat::Mp4 => "mp4",
        AudioFormat::Other => "other",
    }
}

/// Parse a [`ProjectPoolAsset::format`] tag back into an
/// [`AudioFormat`](resonance_common::AudioFormat). Unknown / unexpected
/// tags (including a future build's new variant, or a hand-edited file)
/// fall back to `Other` so loading never fails on the format label.
pub fn audio_format_from_tag(tag: &str) -> resonance_common::AudioFormat {
    use resonance_common::AudioFormat;
    match tag {
        "wav" => AudioFormat::Wav,
        "flac" => AudioFormat::Flac,
        "mp3" => AudioFormat::Mp3,
        "ogg" => AudioFormat::Ogg,
        "aac" => AudioFormat::Aac,
        "mp4" => AudioFormat::Mp4,
        _ => AudioFormat::Other,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMidiClip {
    pub id: u64,
    pub track_id: u64,
    pub start_sample: u64,
    pub duration_ticks: u64,
    pub name: String,
    pub trim_start_ticks: u64,
    pub trim_end_ticks: u64,
    /// Project-relative path to the clip's Standard MIDI File.
    pub midi_file: String,
    /// Per-note vocal lyric annotations, parallel to the clip's note
    /// list. Carries OpenUtau-style slur markers (`"+"`) and explicit
    /// per-note label overrides. Empty when the clip isn't on a vocal
    /// track or hasn't had any lyric edits applied. Trailing empty
    /// strings are stripped by the serializer to keep the JSON lean —
    /// the replay path pads back to `notes.len()` on load.
    #[serde(default)]
    pub vocal_lyrics: Vec<String>,
}

/// Everything needed to reconstruct a project after loading from disk.
#[derive(Debug, Clone)]
pub struct LoadedProject {
    pub file: ProjectFile,
    /// Absolute path to the project directory (the `.rproj` folder).
    /// The replay step needs this to resolve `ProjectClip.audio_file`
    /// into the absolute path it hands the engine.
    pub project_dir: PathBuf,
    /// MIDI notes per clip id, read from the sibling `.mid` files.
    pub midi_notes: HashMap<ClipId, Vec<MidiNote>>,
    pub plugin_states: HashMap<PluginInstanceId, Vec<u8>>,
}

/// State accumulated during an async save operation. The engine
/// streams recorded audio straight to `audio/clip_{id}.wav`, so by
/// the time the save kicks off the clip files already exist on
/// disk; we only need to collect the confirmed path list and the
/// plugin state blobs, then write `project.json`.
pub struct SaveCollector {
    pub path: PathBuf,
    /// Map from clip id to project-relative WAV path, returned by
    /// `AudioEvent::ClipsSavedToProjectDir`.
    pub clip_files: HashMap<ClipId, String>,
    pub plugin_states: Vec<(PluginInstanceId, Vec<u8>)>,
    pub clips_done: bool,
    pub plugins_done: bool,
    /// True when this collector is servicing a periodic autosave rather
    /// than a manual save. Autosaves write their metadata to
    /// [`AUTOSAVE_JSON`], leave `dirty` set, skip the recents list and
    /// versioned backups, and update `last_autosave_at` on completion.
    pub autosave: bool,
}

/// Write a project to disk. Assumes the engine has already written
/// every audio clip's WAV file into `{path}/audio/`; this function
/// only writes `project.json`, the MIDI clip files, and the plugin
/// state blobs.
pub fn save_project(
    path: &Path,
    project: &ProjectFile,
    plugin_states: &[(PluginInstanceId, Vec<u8>)],
    midi_clips: &[(ClipId, Vec<MidiNote>)],
) -> Result<(), String> {
    write_project_metadata(path, PROJECT_JSON, project, plugin_states, midi_clips)
}

/// Write an autosave snapshot. Identical to [`save_project`] except the
/// project metadata goes to [`AUTOSAVE_JSON`] instead of [`PROJECT_JSON`],
/// so the canonical `project.json` is never overwritten. The shared
/// audio / MIDI / plugin blobs are written to the same id-keyed paths as
/// a normal save, so the side file plus those blobs form a complete,
/// loadable snapshot for crash recovery.
pub fn save_autosave(
    path: &Path,
    project: &ProjectFile,
    plugin_states: &[(PluginInstanceId, Vec<u8>)],
    midi_clips: &[(ClipId, Vec<MidiNote>)],
) -> Result<(), String> {
    write_project_metadata(path, AUTOSAVE_JSON, project, plugin_states, midi_clips)
}

/// Shared body of [`save_project`] / [`save_autosave`]: write the plugin
/// state blobs and MIDI clip files, then the project-metadata JSON under
/// `json_file_name`. Every write goes through [`atomic_write`].
fn write_project_metadata(
    path: &Path,
    json_file_name: &str,
    project: &ProjectFile,
    plugin_states: &[(PluginInstanceId, Vec<u8>)],
    midi_clips: &[(ClipId, Vec<MidiNote>)],
) -> Result<(), String> {
    let plugins_dir = path.join("plugins");
    let midi_dir = path.join("midi");
    std::fs::create_dir_all(&plugins_dir).map_err(|e| format!("Create plugins dir: {e}"))?;
    std::fs::create_dir_all(&midi_dir).map_err(|e| format!("Create midi dir: {e}"))?;

    // Write plugin state blobs.
    for (instance_id, data) in plugin_states {
        let file_name = format!("plugin_{instance_id}.bin");
        let file_path = plugins_dir.join(&file_name);
        atomic_write(&file_path, data).map_err(|e| format!("Write {file_name}: {e}"))?;
    }

    // Write MIDI clips as Standard MIDI Files.
    for (clip_id, notes) in midi_clips {
        let file_path = midi_dir.join(format!("clip_{clip_id}.mid"));
        let bytes = midi_io::encode_midi(notes).map_err(|e| format!("Encode midi {clip_id}: {e}"))?;
        atomic_write(&file_path, &bytes).map_err(|e| format!("Write midi {clip_id}: {e}"))?;
    }

    // Write the project-metadata JSON (project.json or, for an autosave,
    // project.autosave.json).
    let json =
        serde_json::to_string_pretty(project).map_err(|e| format!("Serialize project: {e}"))?;
    atomic_write(&path.join(json_file_name), json.as_bytes())
        .map_err(|e| format!("Write {json_file_name}: {e}"))?;

    Ok(())
}

/// Crash-safe file write: write `bytes` to a sibling `*.tmp` in the
/// same directory, fsync it, atomically rename it over `path`, then
/// fsync the parent directory so the rename itself reaches disk. A
/// crash at any point leaves either the previous file or the new file
/// fully intact — never a truncated target.
///
/// The temp file lives in the same directory as the target so the
/// rename stays within one filesystem (cross-device renames are not
/// atomic). Its name embeds the target's file name to avoid colliding
/// with the temp files of sibling writes in the same directory.
///
/// A leftover `*.tmp` from an interrupted write is inert: it shares no
/// name with any file the loader looks for, so it can never clobber a
/// good `project.json`, `.mid`, or `.bin`.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| format!("Atomic write target {} has no parent dir", path.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("Atomic write target {} has no file name", path.display()))?;

    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = parent.join(&tmp_name);

    // Write the full contents and fsync before the rename so the new
    // data is durable on disk before it becomes visible at `path`.
    {
        let mut f = std::fs::File::create(&tmp_path)
            .map_err(|e| format!("create {}: {e}", tmp_path.display()))?;
        f.write_all(bytes)
            .map_err(|e| format!("write {}: {e}", tmp_path.display()))?;
        f.sync_all()
            .map_err(|e| format!("fsync {}: {e}", tmp_path.display()))?;
    }

    // Atomic on POSIX: an observer sees either the old or the new file.
    std::fs::rename(&tmp_path, path).map_err(|e| {
        // Best-effort cleanup so a failed rename doesn't strand the tmp.
        let _ = std::fs::remove_file(&tmp_path);
        format!("rename {} -> {}: {e}", tmp_path.display(), path.display())
    })?;

    // fsync the directory so the rename entry itself survives a crash.
    // Directory fsync is unsupported on some platforms/filesystems, so
    // failures here are tolerated — the file data is already durable.
    if let Ok(dir) = std::fs::File::open(parent) {
        let _ = dir.sync_all();
    }

    Ok(())
}

/// One timestamped snapshot under a project's `backups/` directory.
/// Returned by [`list_backups`] for the restore UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupEntry {
    /// Absolute path to the backup file (`backups/project-<rfc3339>.json`).
    pub path: PathBuf,
    /// The RFC3339 UTC timestamp embedded in the file name, verbatim.
    pub timestamp: String,
}

/// Current wall clock as an RFC3339 UTC string suitable for a backup
/// file name. Callers pass the result to [`write_backup`]; keeping the
/// clock read out of `write_backup` lets tests drive rotation with
/// deterministic timestamps.
pub fn backup_timestamp_now() -> String {
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;

    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        // Formatting RFC3339 from a valid `OffsetDateTime` is infallible
        // in practice; fall back to an epoch-seconds stamp rather than
        // unwrap so a backup is still written under some sortable name.
        .unwrap_or_else(|_| {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("epoch-{secs}")
        })
}

fn backup_file_name(timestamp: &str) -> String {
    format!("project-{timestamp}.json")
}

/// Snapshot the project's already-written `project.json` into
/// `backups/project-<timestamp>.json` (atomic write) and prune the
/// oldest snapshots so at most `retention` remain.
///
/// Call this only after a successful save: it reads the canonical
/// `project.json`, so a failed save (which never wrote it, or left the
/// prior copy intact) is never captured as a fresh backup. The
/// audio/MIDI/plugin blobs are *shared*, not copied — a backup is a
/// metadata snapshot whose relative paths still resolve against the
/// project directory.
///
/// `timestamp` is the RFC3339 UTC stamp for the file name (see
/// [`backup_timestamp_now`]). A `retention` of 0 prunes every snapshot,
/// including the one just written; callers that want backups pass `>= 1`.
pub fn write_backup(project_dir: &Path, timestamp: &str, retention: u32) -> Result<PathBuf, String> {
    let source = project_dir.join("project.json");
    let bytes =
        std::fs::read(&source).map_err(|e| format!("Read project.json for backup: {e}"))?;

    let backups_dir = project_dir.join("backups");
    std::fs::create_dir_all(&backups_dir).map_err(|e| format!("Create backups dir: {e}"))?;

    let dest = backups_dir.join(backup_file_name(timestamp));
    atomic_write(&dest, &bytes).map_err(|e| format!("Write backup: {e}"))?;

    prune_backups(&backups_dir, retention)?;
    Ok(dest)
}

/// Delete the oldest snapshots in `backups_dir` until at most `retention`
/// remain. Newest-first ordering comes from [`scan_backups`].
fn prune_backups(backups_dir: &Path, retention: u32) -> Result<(), String> {
    for entry in scan_backups(backups_dir).into_iter().skip(retention as usize) {
        std::fs::remove_file(&entry.path)
            .map_err(|e| format!("Prune backup {}: {e}", entry.path.display()))?;
    }
    Ok(())
}

/// List the versioned backups under `{project_dir}/backups`, newest
/// first, for the restore UI. A missing or unreadable `backups/` dir
/// yields an empty list rather than an error — there's simply nothing to
/// restore.
pub fn list_backups(project_dir: &Path) -> Vec<BackupEntry> {
    scan_backups(&project_dir.join("backups"))
}

/// Scan a `backups/` directory for `project-<timestamp>.json` snapshots,
/// sorted newest-first by their embedded RFC3339 timestamp. Leftover
/// `*.tmp` files from an interrupted [`atomic_write`] are ignored (they
/// don't end in `.json`), as is any unrelated file.
fn scan_backups(backups_dir: &Path) -> Vec<BackupEntry> {
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;

    let read_dir = match std::fs::read_dir(backups_dir) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };

    let mut entries: Vec<(Option<OffsetDateTime>, BackupEntry)> = read_dir
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name()?.to_str()?;
            let timestamp = name.strip_prefix("project-")?.strip_suffix(".json")?;
            let parsed = OffsetDateTime::parse(timestamp, &Rfc3339).ok();
            Some((
                parsed,
                BackupEntry {
                    path: path.clone(),
                    timestamp: timestamp.to_string(),
                },
            ))
        })
        .collect();

    // Newest first. Parse the RFC3339 stamp rather than string-compare:
    // variable sub-second precision (`…00Z` vs `…00.5Z`) doesn't sort
    // chronologically as plain text. Unparseable names sort oldest.
    entries.sort_by(|(a, _), (b, _)| match (a, b) {
        (Some(a), Some(b)) => b.cmp(a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    entries.into_iter().map(|(_, e)| e).collect()
}

/// Read a project from disk. Audio clips stay on disk and are
/// memory-mapped by the engine's load-clip handler — this function
/// only parses `project.json`, loads MIDI clips from their `.mid`
/// files, and collects plugin state blobs.
pub fn load_project(path: &Path) -> Result<LoadedProject, String> {
    let json_path = if path.join("project.json").exists() {
        path.join("project.json")
    } else if path
        .file_name()
        .map(|f| f == "project.json")
        .unwrap_or(false)
    {
        path.to_path_buf()
    } else {
        return Err("No project.json found".to_string());
    };

    let project_dir = json_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| path.to_path_buf());

    let json =
        std::fs::read_to_string(&json_path).map_err(|e| format!("Read project.json: {e}"))?;
    let file: ProjectFile =
        serde_json::from_str(&json).map_err(|e| format!("Parse project.json: {e}"))?;

    if file.version > PROJECT_FORMAT_VERSION {
        return Err(format!(
            "Project version {} is newer than this build (v{}). \
             Please update Resonance.",
            file.version, PROJECT_FORMAT_VERSION
        ));
    }

    // Read MIDI clip files. Missing files are logged and replaced
    // with empty note lists so the rest of the project still loads.
    let mut midi_notes: HashMap<ClipId, Vec<MidiNote>> = HashMap::new();
    for mc in &file.midi_clips {
        let mid_path = project_dir.join(&mc.midi_file);
        match midi_io::read_midi_file(&mid_path) {
            Ok(notes) => {
                midi_notes.insert(mc.id, notes);
            }
            Err(e) => {
                eprintln!("Warning: could not load midi file {}: {e}", mc.midi_file);
                midi_notes.insert(mc.id, Vec::new());
            }
        }
    }

    // Read plugin state blobs for every track, bus, and master plugin.
    let mut plugin_states: HashMap<PluginInstanceId, Vec<u8>> = HashMap::new();
    let mut load_plugin_state = |plugin: &ProjectPlugin| {
        let state_path = project_dir.join(&plugin.state_file);
        match std::fs::read(&state_path) {
            Ok(data) => {
                plugin_states.insert(plugin.instance_id, data);
            }
            Err(e) => {
                eprintln!(
                    "Warning: could not load plugin state {}: {e}",
                    plugin.state_file
                );
            }
        }
    };
    for track in &file.tracks {
        for plugin in &track.plugins {
            load_plugin_state(plugin);
        }
    }
    for bus in &file.busses {
        for plugin in &bus.plugins {
            load_plugin_state(plugin);
        }
    }
    for plugin in &file.master_plugins {
        load_plugin_state(plugin);
    }

    Ok(LoadedProject {
        file,
        project_dir,
        midi_notes,
        plugin_states,
    })
}
