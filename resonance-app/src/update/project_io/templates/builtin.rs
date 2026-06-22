//! Built-in starter templates: stable data definitions and project builders.
//!
//! The four built-in starters (Empty, Band Recording, Beatmaking, Vocal Songwriting)
//! are defined in code as `ProjectFile` builders — no shipped files — so they always
//! track the current `PROJECT_FORMAT_VERSION` and never go stale. Each builder yields
//! a [`BuiltinProject`]: a ready-to-replay `ProjectFile` plus the per-clip MIDI notes
//! that, in a saved project, would live in sibling `.mid` files.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use resonance_audio::types::{ClipId, MidiNote, PluginInstanceId, TICKS_PER_QUARTER_NOTE};

use crate::compose::{LaneGeneratorConfig, LaneGeneratorKind};
use crate::project::{
    ProjectBus, ProjectFile, ProjectMidiClip, ProjectPlugin, ProjectSectionChord,
    ProjectSectionDefinition, ProjectSectionPlacement, ProjectTrack, PROJECT_FORMAT_VERSION,
};
use crate::state::{InstrumentIcon, InstrumentType};

use super::compute_summary;
use super::TemplateSummary;

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
    ///
    /// This requires importing `Template` from the parent module.
    pub fn template(self) -> super::Template {
        let project = self.build();
        super::Template::new_builtin(
            self.display_name().to_string(),
            self.description().to_string(),
            project.summary(),
        )
    }
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
        external_instrument: None,
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
