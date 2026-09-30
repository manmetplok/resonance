//! Core types for the Resonance audio engine. Split into sub-modules by
//! concern — everything is re-exported so `use resonance_audio::types::*`
//! keeps working unchanged.
/// Unlike [`ClipId`] below, this space has no engine-vs-app partition any
/// more (ARCH-04 D-4, same shape as [`PluginInstanceId`]): the app
/// allocates every track id (`Resonance::allocate_track_id`) — the plain
/// GUI "Add Track", instrument/vocal adds, sub-tracks, bounce targets,
/// the control API and project-load replay all draw from it — and the
/// engine only ever honours the id it is given. `AudioCommand::AddTrack`
/// / `AddInstrumentTrack` / `AddVocalTrack` carry a concrete `id`, not an
/// optional hint, and the add is rejected with `EngineError::internal`
/// if that id is already live. There is no base to name here because
/// there is only one owner — except that track ids stay below
/// `resonance_app::state::ids::BUS_ID_BASE`, for the same reason
/// [`BusId`] gives for keeping its own range: `song.summary` /
/// `song.tracks` list tracks and busses in one id-addressed sequence.
pub type TrackId = u64;
pub type ClipId = u64;
pub type SamplePos = u64;
/// Unlike [`TrackId`] above, this space has no engine-vs-app partition:
/// the app allocates every plugin instance id (`Resonance::allocate_plugin_id`)
/// and the engine only ever honours the one it is given — an
/// `AudioCommand::AddPlugin`/`AddPluginToBus`/`AddPluginToMaster` carries a
/// concrete `id`, not an optional hint, and the add is rejected with
/// `EngineError::internal` if that id is already live (ARCH-04 D-1,
/// `refactor-intent.md` Epic D). There is no base to name here because
/// there is only one owner.
pub type PluginInstanceId = u64;
/// Same engine-side shape as [`PluginInstanceId`] since ARCH-04 D-3: the
/// app allocates every bus id and `AudioCommand::AddBus` carries a
/// concrete `id`, which the engine either honours or refuses with
/// `EngineError::internal` on a collision — the engine itself has no
/// counter and no range to defend, because `ctx.tracks` and `ctx.busses`
/// are separate maps it never confuses. Unlike `PluginInstanceId`,
/// though, the APP still keeps bus ids in their own range
/// (`resonance_app::state::ids::BUS_ID_BASE`): `song.summary` /
/// `song.tracks` list tracks and busses together as one `TrackKind`-tagged
/// sequence addressed by this same raw id, so a bus id landing on a
/// live track's id would make that bus invisible to the control API (the
/// track's entry, listed first, wins the id). Purely an app/control-layer
/// convention — this crate has no reason to know about it.
pub type BusId = u64;
/// Same shape as [`PluginInstanceId`] since ARCH-04 D-2: the app allocates
/// every send id. `AudioCommand::AddAuxSend` carries a concrete `id` and is
/// refused with `EngineError::internal` on a collision; `SetAuxSend` edits
/// an id already live (a no-op if it is not). `CONTROL_SEND_ID_BASE` (2e9)
/// is gone outright: unlike [`BusId`], a send id is never displayed
/// alongside a track/bus id in one merged, id-keyed list, so there is no
/// app-side reason to keep sends in their own range either.
pub type SendId = u64;
/// Identifier for an imported media-pool asset. Allocated by the engine
/// on `AudioCommand::ImportAudioToPool` and carried by the
/// import-lifecycle events. Independent of [`ClipId`]: an asset lives in
/// the project pool and may back zero, one, or many clips.
pub type AssetId = u64;

/// First clip id the app allocates itself — every clip it names (drawn,
/// derived, imported, split, bounce targets, vocal renders), from its one
/// clip allocator (`EntityIds::clips` in `resonance-app`, whose name for
/// this value is `CLIP_ID_BASE` since D-7b). Engine clips (recordings,
/// take passes, live-MIDI captures) counted up from 1 until D-7d; they now
/// draw from blocks of that same allocator the app grants
/// (`AudioCommand::GrantIds`). Until D-7f every path that hands the engine a concrete clip id
/// (`LoadMidiClipDirect`, `LoadClipFromWav`, the take restore, the
/// STATE-08 `audio/clip_<id>.wav` scan) bumps `next_clip_id` only for ids
/// *below* this base, as for tracks. Before that rule (FU-A6a) the first
/// derived clip dragged the engine's counter to `base + 1`, which is
/// exactly where the app's derived counter allocated next.
pub const DERIVED_CLIP_ID_BASE: ClipId = 1 << 40;

/// How many clip ids the app grants the engine at a time
/// (`AudioCommand::GrantIds`, ARCH-04 D-7d; design doc D-6 §4.2 / §7a.4:
/// fixed, not scaled by the armed-track count).
pub const CLIP_GRANT_SIZE: u64 = 1024;

/// The engine reports `AudioEvent::IdGrantLow` once its clip-id grant falls
/// below this many ids. At the worst consumer the design found (a 1-beat
/// loop at 200 bpm cycle-recording 16 tracks, ~50 ids a second) this is
/// about ten seconds of slack against a refill that takes one event drain.
pub const CLIP_GRANT_LOW_WATER: u64 = 512;

/// Where a track's post-fader audio lands. Tracks either sum directly
/// into the master output (the default, matching pre-bus behaviour) or
/// route into a named bus for group processing before reaching master.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackOutput {
    Master,
    Bus(BusId),
}

/// Track flavour. `Audio` is a plain audio-clip track; `Instrument` carries
/// MIDI clips that feed an instrument plugin; `Vocal` is a singing-voice
/// track that pairs a MIDI clip (the staff / lyric carrier) with a
/// rendered audio clip from the SVS pipeline.
///
/// Engine code that needs to know "does this track receive MIDI?" should
/// use [`TrackType::accepts_midi`] rather than matching on `Instrument`
/// directly, so vocal tracks pick up the same plumbing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackType {
    Audio,
    Instrument,
    Vocal,
}

impl TrackType {
    /// Track accepts timed MIDI events — schedule MIDI clips, accept live
    /// MIDI input, drive an instrument plugin. Currently true for
    /// `Instrument` and `Vocal` (the vocal lane carries MIDI for the staff
    /// visualisation and for driving the SVS pipeline).
    pub fn accepts_midi(self) -> bool {
        matches!(self, TrackType::Instrument | TrackType::Vocal)
    }
}

mod aux_send;
pub mod sidechain;
mod clip;
mod commands;
mod error;
mod events;
mod reference;
mod measure;
mod probe;
mod stem;
mod freeze;
mod export;
mod tempo;
mod track;
mod vocal_tuning;

pub use aux_send::{aux_send_would_cycle, AuxSend, SendSource};
pub use sidechain::{SidechainRoute, SidechainTaps, MAX_SIDECHAIN_SOURCES};
pub use clip::{
    audio_clip_covers, compute_waveform_peaks, move_note_resorted, AudioClip, ClipSource,
    FadeCurve, MidiClip, MidiNote, PendingNoteEvent, WarpAlgorithm, WarpMarker, WAVEFORM_PEAK_FRAMES,
};
pub use freeze::FrozenSource;
pub use vocal_tuning::{F0Frame, GlobalTuning, NoteBlob, NoteEdit, TuningScale, VocalTuning};
pub use commands::{AudioCommand, IdGrantBlocks, PluginPresetLocation, PoolImportFile};
pub use error::{EngineError, EngineErrorKind};
pub use events::{
    AudioEvent, BouncedClipData, ExportErrorKind, ExportPhase, ImportStage, PluginEditorFailure,
};
pub use reference::{ABSource, ReferenceAnalysisStage, ReferenceId, ReferenceMarker};
pub use measure::{
    AudioMeasureSource, DepthDetail, DepthSend, DetailSet, MeasureSource, MeasurementDetail,
    MixMeasurement,
};
pub use probe::{ChainProbeReport, ProbeSpec, ProbeStage, ProbedStage};
pub use stem::{StemBitDepth, StemSource, StemTarget};
pub use export::{
    BitDepth, ExportFormat, ExportMetadata, ExportSettings, FlacLevel, Mp3Rate, NormalizeMode,
    NormalizeSpec, OpusOptimize,
};
pub use tempo::{
    arrival_bpm_at_bar, avg_bpm_for_bar, bar_len_quarters, bar_len_ticks, beat_len_ticks,
    bpm_at_bar, deserialize_bpm, sanitize_bpm, sample_frac_to_tick_frac, tick_frac_to_sample_frac,
    ticks_to_quarters, InputDeviceInfo, ParamInfo, PluginDescInfo, PluginScanFailure, ScannedPlugin, SignaturePoint,
    TempoMap, TempoPoint, DEFAULT_BPM, MAX_BPM, MIN_BPM, TICKS_PER_QUARTER_NOTE,
    TICKS_PER_WHOLE_NOTE,
};
pub use track::{any_top_level_solo, snapshot_top_level_solo, Bus, MasterBus, Track, TrackMap};
