//! Core types for the Resonance audio engine. Split into sub-modules by
//! concern — everything is re-exported so `use resonance_audio::types::*`
//! keeps working unchanged.
pub type TrackId = u64;
pub type ClipId = u64;
pub type SamplePos = u64;
/// Unlike [`TrackId`]/[`BusId`]/[`SendId`] below, this space has no
/// engine-vs-app partition: the app allocates every plugin instance id
/// (`Resonance::allocate_plugin_id`) and the engine only ever honours the
/// one it is given — an `AudioCommand::AddPlugin`/`AddPluginToBus`/
/// `AddPluginToMaster` carries a concrete `id`, not an optional hint, and
/// the add is rejected with `EngineError::internal` if that id is already
/// live (ARCH-04 D-1, `refactor-intent.md` Epic D). There is no base to
/// name here because there is only one owner.
pub type PluginInstanceId = u64;
pub type BusId = u64;
pub type SendId = u64;
/// Identifier for an imported media-pool asset. Allocated by the engine
/// on `AudioCommand::ImportAudioToPool` and carried by the
/// import-lifecycle events. Independent of [`ClipId`]: an asset lives in
/// the project pool and may back zero, one, or many clips.
pub type AssetId = u64;

/// First track id the app allocates itself — sub-tracks, bounce targets,
/// control-API adds, and track groups (app-only entities that share the
/// track id space). The engine's own tracks count up from 1, and the
/// track add paths bump `next_track_id` only for hints *below* this
/// base: a hint at or above it is app-owned, and bumping past it would
/// put the next GUI add (`id_hint: None`) onto an id the app may already
/// hold for a group the engine never hears about.
pub const SUB_TRACK_ID_BASE: TrackId = 1_000_000_000;

/// First bus id the app allocates itself (FX returns created up front,
/// `bus.create`). Engine busses count up from 1; `AddBus` bumps
/// `next_bus_id` only for hints below this base, as for tracks.
pub const RETURN_BUS_ID_BASE: BusId = 2_000_000_000;

/// First aux-send id the app allocates itself (`track.add_send`). Engine
/// sends count up from 1; `SetAuxSend` bumps `next_send_id` only for
/// hints below this base, as for tracks.
pub const CONTROL_SEND_ID_BASE: SendId = 2_000_000_000;

/// First clip id the app allocates itself — the clips it derives from
/// chords, drum patterns and vocal renders, and the other clips it must
/// name before the engine echoes (`ComposeState::fresh_derived_clip_id`).
/// Engine clips (GUI-drawn MIDI clips, recordings, imports) count up from
/// 1, and every path that hands the engine a concrete clip id
/// (`LoadMidiClipDirect`, `LoadClipFromWav`, the take restore, the
/// STATE-08 `audio/clip_<id>.wav` scan) bumps `next_clip_id` only for ids
/// *below* this base, as for tracks. Before that rule (FU-A6a) the first
/// derived clip dragged the engine's counter to `base + 1`, which is
/// exactly where the app's derived counter allocated next.
pub const DERIVED_CLIP_ID_BASE: ClipId = 1 << 40;

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
pub use commands::AudioCommand;
pub use error::{EngineError, EngineErrorKind};
pub use events::{
    AudioEvent, BouncedClipData, ExportErrorKind, ExportPhase, ImportStage, PluginEditorFailure,
};
pub use reference::{ABSource, ReferenceAnalysisStage, ReferenceId, ReferenceMarker};
pub use measure::{MeasureSource, MixMeasurement};
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
pub use track::{any_top_level_solo, Bus, MasterBus, Track};
