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
use std::path::{Path, PathBuf};

use resonance_audio::midi_io;
use resonance_audio::types::{ClipId, MidiNote, PluginInstanceId};

pub mod sections;
pub use sections::{ProjectSectionChord, ProjectSectionDefinition, ProjectSectionPlacement};

pub const PROJECT_FORMAT_VERSION: u32 = 2;

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
    let plugins_dir = path.join("plugins");
    let midi_dir = path.join("midi");
    std::fs::create_dir_all(&plugins_dir).map_err(|e| format!("Create plugins dir: {e}"))?;
    std::fs::create_dir_all(&midi_dir).map_err(|e| format!("Create midi dir: {e}"))?;

    // Write plugin state blobs.
    for (instance_id, data) in plugin_states {
        let file_name = format!("plugin_{instance_id}.bin");
        let file_path = plugins_dir.join(&file_name);
        std::fs::write(&file_path, data).map_err(|e| format!("Write {file_name}: {e}"))?;
    }

    // Write MIDI clips as Standard MIDI Files.
    for (clip_id, notes) in midi_clips {
        let file_path = midi_dir.join(format!("clip_{clip_id}.mid"));
        midi_io::write_midi_file(&file_path, notes)
            .map_err(|e| format!("Write midi {clip_id}: {e}"))?;
    }

    // Write project.json.
    let json =
        serde_json::to_string_pretty(project).map_err(|e| format!("Serialize project: {e}"))?;
    std::fs::write(path.join("project.json"), json)
        .map_err(|e| format!("Write project.json: {e}"))?;

    Ok(())
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
