//! Core types for the Resonance audio engine. Split into sub-modules by
//! concern — everything is re-exported so `use resonance_audio::types::*`
//! keeps working unchanged.
pub type TrackId = u64;
pub type ClipId = u64;
pub type SamplePos = u64;
pub type PluginInstanceId = u64;
pub type BusId = u64;
/// Identifier for an imported media-pool asset. Allocated by the engine
/// on `AudioCommand::ImportAudioToPool` and carried by the
/// import-lifecycle events. Independent of [`ClipId`]: an asset lives in
/// the project pool and may back zero, one, or many clips.
pub type AssetId = u64;

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

mod clip;
mod commands;
mod events;
mod export;
mod tempo;
mod track;
mod vocal_tuning;

pub use clip::{
    compute_waveform_peaks, AudioClip, ClipSource, FadeCurve, MidiClip, MidiNote,
    PendingNoteEvent, WAVEFORM_PEAK_FRAMES,
};
pub use vocal_tuning::{F0Frame, GlobalTuning, NoteBlob, NoteEdit, TuningScale, VocalTuning};
pub use commands::AudioCommand;
pub use events::{
    AudioEvent, BouncedClipData, ExportErrorKind, ExportPhase, ImportStage,
};
pub use export::{
    BitDepth, ExportFormat, ExportMetadata, ExportSettings, FlacLevel, Mp3Rate, NormalizeMode,
    NormalizeSpec, OpusOptimize,
};
pub use tempo::{
    arrival_bpm_at_bar, avg_bpm_for_bar, bpm_at_bar, sample_frac_to_tick_frac,
    tick_frac_to_sample_frac, InputDeviceInfo, ParamInfo, PluginDescInfo, ScannedPlugin,
    SignaturePoint, TempoMap, TempoPoint, TICKS_PER_QUARTER_NOTE,
};
pub use track::{any_top_level_solo, Bus, MasterBus, Track};
