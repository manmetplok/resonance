//! Project templates: storage layout, metadata model, and user-template scanning.
//!
//! Templates are stored as folders with the same on-disk shape as saved projects
//! (project.json + midi/ + plugins/ + audio/), plus a sibling `template.json` metadata
//! sidecar. This allows templates to round-trip through the existing project I/O
//! path.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use resonance_audio::types::{ClipId, MidiNote, TICKS_PER_QUARTER_NOTE};

use crate::compose::{LaneGeneratorConfig, LaneGeneratorKind};
use crate::project::{
    ProjectBus, ProjectFile, ProjectMidiClip, ProjectPlugin, ProjectSectionChord,
    ProjectSectionDefinition, ProjectSectionPlacement, ProjectTrack, PROJECT_FORMAT_VERSION,
};
use crate::state::{InstrumentIcon, InstrumentType};

/// Directory name for user templates under the config directory.
const TEMPLATES_DIR_NAME: &str = "templates";

/// Application directory name (same as used in recent.rs).
const APP_DIR: &str = "resonance";

/// Template storage directory, lazily created.
/// Mirrors the location pattern in `recent.rs`.
pub fn templates_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR).join(TEMPLATES_DIR_NAME))
}

/// Ensure the user templates directory exists, creating it if necessary.
/// Returns the path or None if the config directory cannot be resolved.
pub fn ensure_templates_dir() -> Option<PathBuf> {
    let dir = templates_dir()?;
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("Failed to create templates directory: {e}");
        None
    } else {
        Some(dir)
    }
}

/// Kind of template: built-in (defined in code) or user-created (from disk).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateKind {
    Builtin,
    User,
}

/// Summary statistics precomputed from a project for display in the template picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateSummary {
    pub track_count: usize,
    pub bus_count: usize,
    pub plugin_count: usize,
    pub tempo_bpm: f32,
    pub time_sig: String,
}

/// Metadata sidecar stored alongside each template folder as `template.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateMetadata {
    /// Human-readable name for display.
    pub name: String,
    /// Optional description for the template picker.
    pub description: String,
    /// Whether this template is built-in (always current schema) or user-created.
    pub built_in: bool,
    /// The project schema version this template was captured at.
    pub schema_version: u32,
    /// Precomputed summary for display.
    pub summary: TemplateSummary,
    /// Unix timestamp when the template was created.
    pub created_secs: u64,
}

/// Reason why a template entry is marked as stale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StaleReason {
    /// The template's schema_version is newer than the current build.
    SchemaVersionNewer { schema_version: u32 },
    /// The project.json file failed to parse.
    ProjectParseError { reason: String },
    /// The template.json file failed to parse.
    MetadataParseError { reason: String },
}

/// A user template entry that is stale (incompatible or corrupted).
/// The underlying files are left untouched for potential recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaleTemplate {
    /// Path to the template folder.
    pub path: PathBuf,
    /// Reason for the stale status.
    pub reason: StaleReason,
    /// The schema version from template.json if available.
    pub schema_version: Option<u32>,
}

/// A resolved template entry from the user templates directory.
#[derive(Debug, Clone)]
pub enum TemplateEntry {
    /// A valid, loadable template.
    Valid(Template),
    /// A stale template that cannot be loaded but is kept for recovery.
    Stale(StaleTemplate),
}

/// A complete template ready for instantiation.
#[derive(Debug, Clone)]
pub struct Template {
    /// Kind: built-in or user.
    pub kind: TemplateKind,
    /// Human-readable name.
    pub name: String,
    /// Optional description.
    pub description: String,
    /// Precomputed summary.
    pub summary: TemplateSummary,
    /// Absolute path to the template directory (for user templates) or a placeholder
    /// (for built-ins which don't exist on disk).
    pub path: PathBuf,
    /// Schema version of the captured project.
    pub schema_version: u32,
    /// Unix timestamp when created.
    pub created_secs: Option<u64>,
}

impl Template {
    /// Create a new user template from metadata and path.
    pub fn new_user(metadata: TemplateMetadata, path: PathBuf) -> Self {
        Self {
            kind: TemplateKind::User,
            name: metadata.name,
            description: metadata.description,
            summary: metadata.summary,
            path,
            schema_version: metadata.schema_version,
            created_secs: Some(metadata.created_secs),
        }
    }

    /// Create a new built-in template.
    pub fn new_builtin(
        name: String,
        description: String,
        summary: TemplateSummary,
    ) -> Self {
        Self {
            kind: TemplateKind::Builtin,
            name,
            description,
            summary,
            // Built-ins don't have a disk path; use a placeholder.
            path: PathBuf::new(),
            schema_version: PROJECT_FORMAT_VERSION,
            created_secs: None,
        }
    }
}

/// Scan the user templates directory and return all discovered templates.
///
/// This function:
/// - Resolves the templates directory (lazily creating it if missing)
/// - Enumerates all subdirectories
/// - For each directory, attempts to parse `template.json`
/// - If template.json is valid and parseable:
///   - Checks if schema_version > PROJECT_FORMAT_VERSION or if project.json fails to parse
///   - Returns a Stale entry if either check fails (without touching files)
///   - Returns a Valid Template entry if all checks pass
/// - Missing/unreadable directory -> returns empty list, never an error
/// - All I/O errors are swallowed and logged to stderr
pub fn scan_user_templates() -> Vec<TemplateEntry> {
    let Some(dir) = templates_dir() else {
        return Vec::new();
    };
    scan_templates_in(&dir)
}

/// Scan a specific directory for user templates. Split out from
/// [`scan_user_templates`] (which resolves the real config dir) so the
/// classification logic can be exercised against a controlled fixture
/// directory in tests. See [`scan_user_templates`] for the contract;
/// a missing/unreadable `dir` yields an empty list, never an error.
pub fn scan_templates_in(dir: &std::path::Path) -> Vec<TemplateEntry> {
    // If the directory doesn't exist, return empty (lazy creation happens on first save).
    if !dir.exists() {
        return Vec::new();
    }

    // If it exists but isn't a directory, return empty.
    if !dir.is_dir() {
        eprintln!("Templates path exists but is not a directory: {}", dir.display());
        return Vec::new();
    }

    let mut results = Vec::new();

    // Try to read the directory entries.
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to read templates directory: {e}");
            return Vec::new();
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Failed to read templates directory entry: {e}");
                continue;
            }
        };

        let path = entry.path();
        if !path.is_dir() {
            continue; // Skip non-directories
        }

        // Look for template.json in this directory.
        let template_json_path = path.join("template.json");
        let project_json_path = path.join("project.json");

        // Try to read and parse template.json.
        let template_metadata: TemplateMetadata = match fs::read_to_string(&template_json_path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(m) => m,
                Err(e) => {
                    results.push(TemplateEntry::Stale(StaleTemplate {
                        path: path.clone(),
                        reason: StaleReason::MetadataParseError {
                            reason: format!("Failed to parse template.json: {e}"),
                        },
                        schema_version: None,
                    }));
                    continue;
                }
            },
            Err(e) => {
                eprintln!(
                    "Failed to read template.json at {}: {e}",
                    template_json_path.display()
                );
                // If template.json is missing or unreadable, this isn't a valid template.
                // We could mark it as stale, but without metadata we don't have schema_version.
                // For now, skip it silently (it might be a partial/aborted save).
                continue;
            }
        };

        // Check if schema_version is newer than current build.
        if template_metadata.schema_version > PROJECT_FORMAT_VERSION {
            results.push(TemplateEntry::Stale(StaleTemplate {
                path: path.clone(),
                reason: StaleReason::SchemaVersionNewer {
                    schema_version: template_metadata.schema_version,
                },
                schema_version: Some(template_metadata.schema_version),
            }));
            continue;
        }

        // Try to parse project.json to verify it's valid.
        // We don't need the full ProjectFile, just need to check it parses.
        let _: ProjectFile = match fs::read_to_string(&project_json_path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(p) => p,
                Err(e) => {
                    results.push(TemplateEntry::Stale(StaleTemplate {
                        path: path.clone(),
                        reason: StaleReason::ProjectParseError {
                            reason: format!("Failed to parse project.json: {e}"),
                        },
                        schema_version: Some(template_metadata.schema_version),
                    }));
                    continue;
                }
            },
            Err(e) => {
                eprintln!(
                    "Failed to read project.json at {}: {e}",
                    project_json_path.display()
                );
                results.push(TemplateEntry::Stale(StaleTemplate {
                    path: path.clone(),
                    reason: StaleReason::ProjectParseError {
                        reason: format!("Failed to read project.json: {e}"),
                    },
                    schema_version: Some(template_metadata.schema_version),
                }));
                continue;
            }
        };

        // All checks passed - this is a valid template.
        results.push(TemplateEntry::Valid(Template::new_user(
            template_metadata,
            path,
        )));
    }

    results
}

/// Compute a TemplateSummary from a ProjectFile.
pub fn compute_summary(project: &ProjectFile) -> TemplateSummary {
    TemplateSummary {
        track_count: project.tracks.len(),
        bus_count: project.busses.len(),
        plugin_count: project
            .tracks
            .iter()
            .map(|t| t.plugins.len())
            .sum::<usize>()
            + project
                .busses
                .iter()
                .map(|b| b.plugins.len())
                .sum::<usize>()
            + project.master_plugins.len(),
        tempo_bpm: project.bpm,
        time_sig: format!("{}/{}", project.time_sig_num, project.time_sig_den),
    }
}

// ---------------------------------------------------------------------------
// Built-in starter projects (impl-plan doc #197, design #192).
//
// The four starters are defined in code as `ProjectFile` builders — no
// shipped files — so they always track the current `PROJECT_FORMAT_VERSION`
// and never go stale. Each builder yields a [`BuiltinProject`]: a ready-to-
// replay `ProjectFile` plus the per-clip MIDI notes that, in a saved
// project, would live in sibling `.mid` files. Instantiating a built-in
// (todo #665) feeds these straight into the existing replay path with
// `project_path = None`, so the source is never mutated.
// ---------------------------------------------------------------------------

/// Stable CLAP ids of the bundled Resonance plugins. The concrete `.clap`
/// file path is machine-specific (resolved by the runtime plugin scan), so
/// built-in templates reference plugins by id only and leave the path empty
/// for the instantiate step to fill in against `available_plugins`.
const CLAP_DRUMS: &str = "com.resonance.drums";
const CLAP_WAVETABLE: &str = "com.resonance.wavetable";
const CLAP_REVERB: &str = "com.resonance.reverb";
const CLAP_DELAY: &str = "com.resonance.delay";
const CLAP_EQ: &str = "com.resonance.eq";
const CLAP_COMPRESSOR: &str = "com.resonance.compressor";
const CLAP_MASTERING: &str = "com.resonance.mastering";

/// A built-in starter rendered to data: a replay-ready `ProjectFile` plus
/// the MIDI notes that a saved project would store in `midi/clip_{id}.mid`.
/// The `midi_notes` map is keyed by the ids of the file's `midi_clips`, so
/// it slots directly into a `LoadedProject` without touching disk.
#[derive(Debug, Clone)]
pub struct BuiltinProject {
    pub file: ProjectFile,
    pub midi_notes: HashMap<ClipId, Vec<MidiNote>>,
}

impl BuiltinProject {
    /// Display summary chips (track/bus/plugin counts, tempo, time-sig)
    /// computed from the built project so they always match its contents.
    pub fn summary(&self) -> TemplateSummary {
        compute_summary(&self.file)
    }
}

/// The four built-in starter templates. Stable identifiers so the picker
/// and the instantiate flow (todo #665) can map a selection back to its
/// builder without depending on the (localizable) display name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinTemplateId {
    VocalSongwriting,
    BandRecording,
    Beatmaking,
    Empty,
}

impl BuiltinTemplateId {
    /// All built-ins, in picker order.
    pub const ALL: [BuiltinTemplateId; 4] = [
        BuiltinTemplateId::VocalSongwriting,
        BuiltinTemplateId::BandRecording,
        BuiltinTemplateId::Beatmaking,
        BuiltinTemplateId::Empty,
    ];

    /// Stable machine slug (persisted / passed around, never localized).
    pub fn slug(self) -> &'static str {
        match self {
            BuiltinTemplateId::VocalSongwriting => "vocal-songwriting",
            BuiltinTemplateId::BandRecording => "band-recording",
            BuiltinTemplateId::Beatmaking => "beatmaking",
            BuiltinTemplateId::Empty => "empty",
        }
    }

    /// Look a built-in up by its [`slug`](Self::slug).
    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.slug() == slug)
    }

    /// Human-readable name for the template picker.
    pub fn display_name(self) -> &'static str {
        match self {
            BuiltinTemplateId::VocalSongwriting => "Vocal songwriting",
            BuiltinTemplateId::BandRecording => "Band recording",
            BuiltinTemplateId::Beatmaking => "Beatmaking",
            BuiltinTemplateId::Empty => "Empty",
        }
    }

    /// One-line description for the template picker.
    pub fn description(self) -> &'static str {
        match self {
            BuiltinTemplateId::VocalSongwriting => {
                "Chord progression, a Compose vocal line on the Lilia voicebank, and a pad/bass bed."
            }
            BuiltinTemplateId::BandRecording => {
                "Six mic/DI audio tracks routed to drum and instrument busses, with a master chain."
            }
            BuiltinTemplateId::Beatmaking => {
                "A drum sampler and a wavetable synth with reverb and delay FX return busses."
            }
            BuiltinTemplateId::Empty => "A blank project at the default tempo and time signature.",
        }
    }

    /// Build the starter as a replay-ready [`BuiltinProject`].
    pub fn build(self) -> BuiltinProject {
        match self {
            BuiltinTemplateId::VocalSongwriting => build_vocal_songwriting(),
            BuiltinTemplateId::BandRecording => build_band_recording(),
            BuiltinTemplateId::Beatmaking => build_beatmaking(),
            BuiltinTemplateId::Empty => build_empty(),
        }
    }

    /// Build the starter and wrap it in a built-in [`Template`] descriptor
    /// (name, description, summary chips) for the picker.
    pub fn template(self) -> Template {
        let project = self.build();
        Template::new_builtin(
            self.display_name().to_string(),
            self.description().to_string(),
            project.summary(),
        )
    }
}

/// The built-in starter [`Template`] descriptors, in picker order. Building
/// each project to derive its summary chips is cheap (pure in-memory data).
pub fn builtin_templates() -> Vec<Template> {
    BuiltinTemplateId::ALL
        .iter()
        .map(|id| id.template())
        .collect()
}

// ---- Builder helpers ------------------------------------------------------

/// A bundled-plugin slot referenced by stable CLAP id. The `.clap` file
/// path and the saved-state blob are intentionally empty: built-ins ship
/// no files, so the instantiate step resolves the path from the runtime
/// plugin scan and the plugin opens at its own defaults.
fn builtin_plugin(instance_id: u64, name: &str, clap_plugin_id: &str) -> ProjectPlugin {
    ProjectPlugin {
        instance_id,
        plugin_name: name.to_string(),
        clap_plugin_id: clap_plugin_id.to_string(),
        clap_file_path: String::new(),
        state_file: String::new(),
    }
}

/// A track with neutral defaults (0 dB, centred, unmuted). `track_type` is
/// one of `"audio"`, `"instrument"`, `"vocal"` — the same strings the
/// replay path matches on.
fn base_track(id: u64, order: usize, name: &str, track_type: &str) -> ProjectTrack {
    ProjectTrack {
        id,
        name: name.to_string(),
        order,
        volume: 0.0,
        pan: 0.0,
        muted: false,
        soloed: false,
        fx_bypassed: false,
        record_armed: false,
        monitor_enabled: false,
        mono: false,
        input_device_name: None,
        input_port_index: None,
        plugins: Vec::new(),
        track_type: track_type.to_string(),
        output_bus: None,
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

/// A bus with neutral defaults.
fn base_bus(id: u64, order: usize, name: &str) -> ProjectBus {
    ProjectBus {
        id,
        name: name.to_string(),
        order,
        volume: 0.0,
        pan: 0.0,
        muted: false,
        fx_bypassed: false,
        plugins: Vec::new(),
    }
}

// ---- The four starters ----------------------------------------------------

/// Empty — a blank project at the default tempo (120 BPM) and time
/// signature (4/4), nothing routed.
fn build_empty() -> BuiltinProject {
    BuiltinProject {
        file: ProjectFile::default(),
        midi_notes: HashMap::new(),
    }
}

/// Band recording — six mic/DI audio tracks split across a drum bus and an
/// instrument bus, with an EQ → Compressor → Mastering chain on the master.
fn build_band_recording() -> BuiltinProject {
    const DRUM_BUS: u64 = 100;
    const INST_BUS: u64 = 101;

    let audio = |id: u64, order: usize, name: &str, bus: u64, icon: InstrumentIcon| {
        let mut t = base_track(id, order, name, "audio");
        t.output_bus = Some(bus);
        t.instrument_icon = icon;
        t
    };

    let tracks = vec![
        audio(1, 0, "Kick", DRUM_BUS, InstrumentIcon::Microphone),
        audio(2, 1, "Snare", DRUM_BUS, InstrumentIcon::Microphone),
        audio(3, 2, "Overheads", DRUM_BUS, InstrumentIcon::Microphone),
        audio(4, 3, "Bass DI", INST_BUS, InstrumentIcon::Music),
        audio(5, 4, "Guitar", INST_BUS, InstrumentIcon::Guitar),
        audio(6, 5, "Lead Vocal", INST_BUS, InstrumentIcon::Microphone),
    ];

    let mut drum_bus = base_bus(DRUM_BUS, 0, "Drums");
    drum_bus.plugins = vec![builtin_plugin(1001, "Resonance Compressor", CLAP_COMPRESSOR)];
    let inst_bus = base_bus(INST_BUS, 1, "Instruments");

    let file = ProjectFile {
        tracks,
        busses: vec![drum_bus, inst_bus],
        master_plugins: vec![
            builtin_plugin(1002, "Resonance EQ", CLAP_EQ),
            builtin_plugin(1003, "Resonance Compressor", CLAP_COMPRESSOR),
            builtin_plugin(1004, "Resonance Mastering", CLAP_MASTERING),
        ],
        ..ProjectFile::default()
    };

    BuiltinProject {
        file,
        midi_notes: HashMap::new(),
    }
}

/// Beatmaking — a drum sampler and a wavetable synth, plus reverb and delay
/// FX return busses. True parallel aux sends arrive with the aux-send
/// feature (todos #475+); until then the FX live on return busses so the
/// routing scaffold is in place.
fn build_beatmaking() -> BuiltinProject {
    let mut drums = base_track(1, 0, "Drums", "instrument");
    drums.instrument_type = InstrumentType::Drum;
    drums.instrument_icon = InstrumentIcon::Drum;
    drums.plugins = vec![builtin_plugin(1001, "Resonance Drums", CLAP_DRUMS)];

    let mut synth = base_track(2, 1, "Bass Synth", "instrument");
    synth.instrument_icon = InstrumentIcon::Music;
    synth.plugins = vec![builtin_plugin(1002, "Resonance Wavetable", CLAP_WAVETABLE)];

    let mut reverb = base_bus(100, 0, "Reverb");
    reverb.plugins = vec![builtin_plugin(1003, "Resonance Reverb", CLAP_REVERB)];
    let mut delay = base_bus(101, 1, "Delay");
    delay.plugins = vec![builtin_plugin(1004, "Resonance Delay", CLAP_DELAY)];

    let file = ProjectFile {
        bpm: 90.0,
        tracks: vec![drums, synth],
        busses: vec![reverb, delay],
        ..ProjectFile::default()
    };

    BuiltinProject {
        file,
        midi_notes: HashMap::new(),
    }
}

/// Vocal songwriting — a chord progression (Compose section), a generated
/// vocal line on the Lilia voicebank, and a pad/bass instrument bed. The
/// vocal melody is pre-baked from the progression so the project opens with
/// a real Compose vocal line instead of an empty staff.
fn build_vocal_songwriting() -> BuiltinProject {
    use resonance_music_theory::{
        Chord, ChordQuality, Mode, PitchClass, Scale, TimedChord, VocalParams, VocalVoicebank,
    };

    const VOCAL_TRACK: u64 = 1;
    const VOCAL_CLIP: u64 = 40;
    const SECTION_DEF: u64 = 10;
    const BEATS_PER_CHORD: u32 = 4;
    const LENGTH_BARS: u32 = 4;
    const TIME_SIG_NUM: u8 = 4;
    const SEED: u64 = 0x00C0_FFEE_FACE_F00D;

    // ---- Tracks: vocal lead + pad/bass bed ----
    let mut vocal = base_track(VOCAL_TRACK, 0, "Lead Vocal", "vocal");
    vocal.instrument_icon = InstrumentIcon::Microphone;

    let mut pad = base_track(2, 1, "Pad", "instrument");
    pad.instrument_icon = InstrumentIcon::WaveSquare;
    pad.plugins = vec![builtin_plugin(1001, "Resonance Wavetable", CLAP_WAVETABLE)];

    let mut bass = base_track(3, 2, "Bass", "instrument");
    bass.instrument_icon = InstrumentIcon::Music;
    bass.plugins = vec![builtin_plugin(1002, "Resonance Wavetable", CLAP_WAVETABLE)];

    // ---- Chord progression: I–V–vi–IV in C major ----
    let progression = [
        Chord::new(PitchClass::C, ChordQuality::Maj),
        Chord::new(PitchClass::G, ChordQuality::Maj),
        Chord::new(PitchClass::A, ChordQuality::Min),
        Chord::new(PitchClass::F, ChordQuality::Maj),
    ];
    let chords: Vec<ProjectSectionChord> = progression
        .iter()
        .enumerate()
        .map(|(i, c)| ProjectSectionChord {
            id: 20 + i as u64,
            start_beat: i as u32 * BEATS_PER_CHORD,
            duration_beats: BEATS_PER_CHORD,
            chord: *c,
        })
        .collect();

    // ---- Vocal lane generator on the Lilia voicebank ----
    let vocal_params = VocalParams {
        voicebank: VocalVoicebank::Lilia,
        ..VocalParams::default()
    };
    let mut lane_generators = HashMap::new();
    lane_generators.insert(
        VOCAL_TRACK,
        LaneGeneratorConfig {
            kind: LaneGeneratorKind::Vocal(vocal_params.clone()),
            seed: SEED,
        },
    );

    let section = ProjectSectionDefinition {
        id: SECTION_DEF,
        name: "Verse".to_string(),
        color: [139, 109, 255],
        length_bars: LENGTH_BARS,
        chords,
        scale: Some(Scale::new(PitchClass::C, Mode::Major)),
        progression_seed: 0,
        generate_params: Default::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators,
        beats_per_chord: BEATS_PER_CHORD,
        seventh_chords: false,
        motif_source: Default::default(),
        drum_pattern_id: None,
    };
    let placement = ProjectSectionPlacement {
        id: 30,
        definition_id: SECTION_DEF,
        start_bar: 0,
    };

    // ---- Pre-bake the vocal melody from the progression ----
    let timed: Vec<TimedChord> = progression
        .iter()
        .enumerate()
        .map(|(i, c)| TimedChord {
            chord: *c,
            start_beat: i as u32 * BEATS_PER_CHORD,
            duration_beats: BEATS_PER_CHORD,
        })
        .collect();
    let generated = resonance_music_theory::derive_vocal(
        &timed,
        &vocal_params,
        TICKS_PER_QUARTER_NOTE as u32,
        SEED,
    );
    let notes: Vec<MidiNote> = generated
        .iter()
        .map(|n| MidiNote {
            note: n.note,
            velocity: n.velocity,
            start_tick: n.start_tick,
            duration_ticks: n.duration_ticks,
        })
        .collect();
    let duration_ticks = LENGTH_BARS as u64 * TIME_SIG_NUM as u64 * TICKS_PER_QUARTER_NOTE;

    let midi_clip = ProjectMidiClip {
        id: VOCAL_CLIP,
        track_id: VOCAL_TRACK,
        start_sample: 0,
        duration_ticks,
        name: "Verse · Lead Vocal".to_string(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
        midi_file: format!("midi/clip_{VOCAL_CLIP}.mid"),
        vocal_lyrics: Vec::new(),
    };

    let file = ProjectFile {
        bpm: 96.0,
        tracks: vec![vocal, pad, bass],
        midi_clips: vec![midi_clip],
        section_definitions: vec![section],
        section_placements: vec![placement],
        ..ProjectFile::default()
    };

    let mut midi_notes = HashMap::new();
    midi_notes.insert(VOCAL_CLIP, notes);

    BuiltinProject { file, midi_notes }
}
