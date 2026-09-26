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
///
/// # Module layout
///
/// | Sub-module | Contents |
/// |------------|----------|
/// | [`model`]  | Serde structs, tag converters, format constants |
/// | [`io`]     | Save / load / autosave / atomic-write / backup |
/// | [`sections`] | Section-definition and placement structs |
/// | [`take_audio`] | Reading a cycle-record take's WAV back out of the bundle |
/// | [`session`] | Session marker + crash-recovery probe (FU-M12a) |

pub mod sections;
pub mod model;
pub mod io;
pub mod take_audio;
pub mod session;

// Re-export section types (existing public surface).
pub use sections::{
    ProjectEntryLength, ProjectPatternEntry, ProjectSectionChord, ProjectSectionDefinition,
    ProjectSectionPlacement,
};

// Re-export model: format constants, all serde structs, and tag helpers.
pub use model::{
    AUTOSAVE_JSON, PROJECT_FORMAT_VERSION, PROJECT_JSON,
    LoadedProject, ProjectBus, ProjectChordRegion, ProjectChordTrack, ProjectClip,
    ProjectExternalInstrument, ProjectFile, ProjectKeyChange,
    ProjectMidiClip, ProjectMidiNote, ProjectPerformance, ProjectPlugin, ProjectPluginParam, ProjectPoolAsset,
    ProjectReference,
    ProjectReferenceMarker, ProjectReferenceSettings, ProjectSend, ProjectSidechainRoute,
    ProjectTrack, SaveCollector,
    audio_format_from_tag, audio_format_tag, clip_audio_file, fade_curve_from_tag, fade_curve_tag,
    send_source_from_tag, send_source_tag,
};

// Re-export the take-recording reader (epic #15, todo #1400): the take
// lane's waveform and its missing-media state both come from this one
// read of the WAV `clip_audio_file` names.
pub use take_audio::load_take_peaks;

// Re-export I/O: save, load, autosave, atomic-write, and backup helpers.
pub use io::{
    BackupEntry, atomic_write, backup_timestamp_now, list_backups, load_project, save_autosave,
    save_project, write_backup,
};
