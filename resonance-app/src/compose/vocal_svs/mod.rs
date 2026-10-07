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
//! Split into [`registry`] (installed banks and each bank's facts),
//! [`segment`] (`DsSegment` construction) and [`post`] (AP gating, gain,
//! resample, WAV writer).

use std::path::PathBuf;

use resonance_audio::types::MidiNote;
use resonance_music_theory::g2p::AssignedSyllable;
use resonance_music_theory::VocalParams;
use resonance_svs::pipeline::{self, PipelineArgs};
use resonance_svs::stages::common::ExecutionProvider;

use crate::compose::expression::ExpressionCurves;

mod error;
mod paths;
mod phonemes;
mod post;
mod pronunciation;
pub mod registry;
mod render_cache;
mod segment;
mod validate;

pub use error::{VocalRenderError, VocalRenderErrorKind};
pub use paths::{svs_backend, CurveKind, CurveSupport};
pub use registry::{BankProfile, VoicebankMissing, VoicebankRegistry, VoicebankRoots};
pub use phonemes::{
    articulation_class, floor_duration_sec, min_articulation_sec, nearest_by_class,
    target_duration_sec, ArticulationClass, PhonemeFate, VoicebankPhonemes,
};
pub use post::write_stereo_wav;
pub use pronunciation::{
    canonicalize_phonemes, clean_word, resolve_clip,
    resolve_clip_with_bank, DictionaryEntry, DictionaryScope, PhonemeProvenance,
    PronunciationState, SyllableOverride,
};
pub use render_cache::{
    render_units_cached, split_render_units, RenderPlan, RenderUnit, SvsRenderCache,
};
pub use segment::build_segment;
pub use validate::{
    articulation_report, validate_for_voicebank, InvalidPhonemeReason, InvalidSyllable,
    NoteArticulation,
};

use post::{
    apply_ap_gate, collect_ap_intervals, mono_to_stereo_with_gain, resample_to, safety_gain_factor,
};

/// Output of `render_vocal_clip` — stereo-interleaved f32 samples ready
/// for `transcode_to_wav` / `write_stereo_f32_wav`, plus their rate
/// and the number of leading/trailing frames that are silence pads
/// (so the engine clip can trim past them via `trim_start_frames` /
/// `trim_end_frames`).
pub struct RenderedVocal {
    pub samples_stereo: Vec<f32>,
    pub sample_rate: u32,
    pub trim_start_frames: u64,
    pub trim_end_frames: u64,
}

/// Leading + trailing silence pad inserted into every rendered
/// segment, in seconds. Matches the reference `.ds` fixtures'
/// convention; the engine clip trims past it so timeline alignment
/// stays correct.
const SEGMENT_PAD_SEC: f64 = 0.3;

/// A gap to the next note longer than its sing duration by more than this
/// many seconds is treated as a genuine silence (breath / rest): the
/// duration builder inserts a trailing `AP` there, and
/// [`render_cache::split_render_units`] cuts a render-unit boundary at the
/// same point. Shared so the two never disagree about where silences are.
pub(crate) const SILENCE_GAP_SEC: f64 = 0.4;

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

/// Render a vocal MIDI clip + lyric draft to a singing waveform with
/// `bank`. `Ok(None)` when there are no notes; an error when the bank (or
/// its vocoder) is not installed ([`VocalRenderError::VoicebankMissing`]),
/// when nothing resolves to phonemes, or when the pipeline fails.
///
/// `assigned` is the per-note resolved + voicebank-validated syllable
/// stream (one entry per note), produced on the main thread by
/// [`resolve_clip_with_bank`] + [`validate_for_voicebank`]. Slur notes
/// hold the previous vowel (the last one also its coda). `cache` carries
/// per-unit rendered audio across renders of the same clip so an edit only
/// re-renders the segments it touched (see [`render_cache`]); after the
/// call its [`SvsRenderCache::last_plan`] holds the "N of M segments
/// changed" tally for the UI overlay.
#[allow(clippy::too_many_arguments)]
pub fn render_vocal_clip(
    notes: &[MidiNote],
    params: &VocalParams,
    assigned: &[AssignedSyllable],
    curves: &ExpressionCurves,
    ticks_per_quarter: u32,
    bpm: f32,
    engine_sample_rate: u32,
    cache: &mut SvsRenderCache,
    bank: &BankProfile,
) -> Result<Option<RenderedVocal>, VocalRenderError> {
    if notes.is_empty() {
        return Ok(None);
    }
    let target = bank
        .render_target(params.speaker.as_deref(), params.voice, None)
        .map_err(VocalRenderError::VoicebankMissing)?;

    // Split the clip into independently-renderable units at genuine
    // silences. A continuous phrase yields a single unit whose segment
    // matches the old whole-clip build.
    let units = split_render_units(notes, params, assigned, curves, ticks_per_quarter, bpm, bank);
    if units.iter().all(|u| u.segment.ph_seq.is_empty()) {
        return Err(VocalRenderError::NothingToSing);
    }

    let args = PipelineArgs {
        ds_file: PathBuf::new(),
        acoustic_config: target.acoustic_config,
        vocoder_config: target.vocoder_config,
        out: PathBuf::new(),
        execution_provider: ExecutionProvider::Cpu,
        device_index: 0,
        speaker: target.speaker,
        // The TIGER voicebank's working CLI command uses speedup=20.
        // (We previously thought 20 produced diffusion artifacts, but
        // those artifacts were actually from feeding the mel into the
        // wrong vocoder.)
        speedup: 20,
        depth: 1000,
    };

    // Render each missing unit through the acoustic + vocoder pipeline,
    // gating its own AP/SP rests to silence; cache hits skip all of this.
    let render_fn = |segment: &resonance_svs::ds::DsSegment| -> Result<(Vec<f32>, u32), String> {
        let ap_intervals = collect_ap_intervals(&segment.ph_seq, &segment.ph_dur);
        let rendered = pipeline::render_segments(std::slice::from_ref(segment), &args)
            .map_err(|e| format!("svs render: {e:#}"))?;
        let sr = rendered.sample_rate;
        let mut mono = rendered.samples;
        apply_ap_gate(&mut mono, &ap_intervals, sr);
        Ok((mono, sr))
    };

    let stitched = render_units_cached(&units, cache, render_fn)?;
    let model_sr = stitched.sample_rate;
    let mono = stitched.mono;

    let gain = safety_gain_factor(&mono);
    let stereo = mono_to_stereo_with_gain(&mono, gain);
    let (samples_stereo, out_sr) = resample_to(stereo, model_sr, engine_sample_rate);

    // The segment was rendered with `SEGMENT_PAD_SEC` of AP on each
    // end (matches the reference `.ds` fixtures so the model ramps in
    // and out cleanly). Translate that pad into frame counts at the
    // *final* sample rate so the engine's clip trim skips past it on
    // playback — keeps the timeline alignment between MIDI and audio.
    let pad_frames = (SEGMENT_PAD_SEC * out_sr as f64) as u64;

    Ok(Some(RenderedVocal {
        samples_stereo,
        sample_rate: out_sr,
        trim_start_frames: pad_frames,
        trim_end_frames: pad_frames,
    }))
}
