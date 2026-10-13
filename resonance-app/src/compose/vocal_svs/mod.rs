//! Bridge between the Compose vocal generator and the `resonance-svs` DiffSinger
//! pipeline. Given the MIDI notes produced by `derive_vocal` plus the lane's
//! `VocalParams`, this module builds a one-segment DiffSinger score, runs
//! the acoustic + vocoder ONNX pipeline, and writes a stereo 32-bit-float
//! WAV the engine can mmap via `AudioCommand::LoadClipFromWav`.
//!
//! Each syllable from the lyric draft is run through
//! `resonance_music_theory::g2p` (CMU dictionary + rule-based fallback)
//! to produce ARPAbet phonemes; the note's duration is shared across
//! those phonemes (consonants short, vowel long) so words like "morning"
//! actually sound like "morning". The f0 curve is piecewise constant per
//! note with a ~40 ms portamento between adjacent notes; an optional
//! 5 Hz vibrato modulates the curve when the user's `vibrato` slider is
//! non-zero.
//!
//! The implementation is split into:
//! - [`registry`]: the installed voicebanks and each bank's facts (speakers,
//!   inventory, language ids, curve flags, its English dictionary).
//! - [`pronunciation`]: the per-syllable overrides, the project dictionary
//!   and the resolver that layers them over the bank dictionary and CMU.
//! - [`segment`]: `DsSegment` construction (phoneme allocation, f0 + tension curves).
//! - [`render`]: the off-thread render of one clip through the pipeline.
//! - [`rests`]: where a line rests (`rest_threshold_beats`) and breathes.
//! - [`dynamics`]: the post-vocoder dynamics gain envelope (velocity).
//! - [`post`]: rest gating, safety gain, mono→stereo, resample, WAV writer.

mod bank_render;
mod dynamics;
mod error;
mod paths;
mod phonemes;
mod post;
mod pronunciation;
pub mod registry;
mod render;
mod render_cache;
mod rests;
mod segment;
mod validate;
pub mod voice_presets;

pub use dynamics::{
    apply_dynamics_gain, apply_dynamics_gain_layers, dynamics_gain_envelope,
    dynamics_gain_envelope_layers, velocity_gain_db, velocity_gain_spans,
    velocity_gain_spans_with_breaths, velocity_modulation, GainSpan, DYNAMICS_CURVE_MAX_DB,
    DYNAMICS_SMOOTH_SEC, NEUTRAL_VELOCITY, VELOCITY_GAIN_MAX_DB,
};
pub use error::{VocalRenderError, VocalRenderErrorKind};
pub use paths::{svs_backend, CurveKind, CurveSupport};
pub use registry::{
    BankProfile, RenderTarget, VoicebankMissing, VoicebankRegistry, VoicebankRoots,
};
pub use phonemes::{
    articulation_class, floor_duration_sec, min_articulation_sec, nearest_by_class,
    target_duration_sec, ArticulationClass, PhonemeFate, VoicebankPhonemes,
};
pub use post::write_stereo_wav;
pub use render::{render_vocal_clip, RenderedVocal};
pub use rests::breath_lead_secs;
pub use pronunciation::{
    canonicalize_phonemes, clean_word, resolve_clip, resolve_clip_with_bank, DictionaryEntry,
    DictionaryScope, PhonemeProvenance, PronunciationState, SyllableOverride,
};
pub use render_cache::{
    plan_unit_keys, render_units_cached, render_units_cached_with, split_render_units,
    RenderPlan, RenderUnit, SvsRenderCache, UnitAudio, FALLBACK_KEY_MARK,
};
pub(crate) use render_cache::model_identity;
pub use segment::{build_segment, build_unit_segment, UnitSegment};
pub use validate::{
    articulation_report_for, validate_for_voicebank, InvalidPhonemeReason, InvalidSyllable,
    NoteArticulation,
};

/// Leading + trailing silence pad inserted into every rendered
/// segment, in seconds. Matches the reference `.ds` fixtures'
/// convention; the engine clip trims past it so timeline alignment
/// stays correct.
const SEGMENT_PAD_SEC: f64 = 0.3;

/// [`SEGMENT_PAD_SEC`] in frames at `sample_rate`: the pad a rendered
/// take carries before its first note's onset. A take installed with
/// pre-roll keeps `pad − trim_start` of it, so `start_sample + pad −
/// trim_start` is where the first note sits — how a reload recognises a
/// take saved without its owner (`ComposeState::rebuild_vocal_audio_clips`).
pub fn pad_frames(sample_rate: u32) -> u64 {
    (SEGMENT_PAD_SEC * sample_rate as f64) as u64
}

/// Where a lane's rendered audio belongs on the timeline: the section's
/// start, advanced by the lane's first note.
///
/// The SVS render is relative to that first note — [`render_cache`]
/// subtracts `base_tick = notes[0].start_tick` when laying segments out —
/// so the synthesised waveform begins *at* the note with no leading
/// silence. Placing it at the section boundary therefore plays the whole
/// phrase early by however far into the section it was written: a lane
/// whose first note sits at beat 8 sounded at beat 0, silently, with
/// relative timing inside the phrase preserved (ba doc #272 V-1).
///
/// Goes through the tempo map rather than a flat samples-per-tick so the
/// offset stays right across a tempo change inside the section.
///
/// [`render_cache`]: crate::compose::vocal_svs::render_cache
pub fn vocal_audio_start(
    tempo_map: &resonance_audio::types::TempoMap,
    section_start_sample: u64,
    lead_ticks: u64,
    sample_rate: u32,
) -> u64 {
    if lead_ticks == 0 {
        return section_start_sample;
    }
    tempo_map.tick_to_abs_sample(section_start_sample, lead_ticks, sample_rate)
}
