//! Core types for the Resonance audio engine. Split into sub-modules by
//! concern — everything is re-exported so `use resonance_audio::types::*`
//! keeps working unchanged.
pub type TrackId = u64;
pub type ClipId = u64;
pub type SamplePos = u64;
pub type PluginInstanceId = u64;
pub type BusId = u64;

/// Where a track's post-fader audio lands. Tracks either sum directly
/// into the master output (the default, matching pre-bus behaviour) or
/// route into a named bus for group processing before reaching master.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackOutput {
    Master,
    Bus(BusId),
}

/// Distinguishes audio recording/playback tracks from instrument (MIDI) tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackType {
    Audio,
    Instrument,
}

mod clip;
mod commands;
mod events;
mod tempo;
mod track;

pub use clip::{
    compute_waveform_peaks, AudioClip, ClipSource, MidiClip, MidiNote, PendingNoteEvent,
    WAVEFORM_PEAK_FRAMES,
};
pub use commands::AudioCommand;
pub use events::AudioEvent;
pub use tempo::{
    InputDeviceInfo, ParamInfo, PluginDescInfo, ScannedPlugin, TempoMap, TICKS_PER_QUARTER_NOTE,
};
pub use track::{Bus, Track};
