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
//! - [`paths`]: voicebank file resolution + per-bank phoneme/language conventions.
//! - [`segment`]: `DsSegment` construction (phoneme allocation, f0 + tension curves).
//! - [`post`]: AP gating, safety gain, mono→stereo, resample, WAV writer.

use std::path::PathBuf;

use resonance_audio::types::MidiNote;
use resonance_music_theory::VocalParams;
use resonance_svs::pipeline::{self, PipelineArgs};
use resonance_svs::stages::common::ExecutionProvider;

mod paths;
mod post;
mod segment;

pub use paths::{curve_supported, CurveKind};
pub use post::write_stereo_wav;

use paths::locate_voicebank;
use post::{
    apply_ap_gate, collect_ap_intervals, mono_to_stereo_with_gain, resample_to, safety_gain_factor,
};
use segment::build_segment;

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
/// stays correct. Visible to descendant modules (`segment`) without
/// a `pub` modifier — Rust grants child modules access to a parent's
/// private items.
const SEGMENT_PAD_SEC: f64 = 0.3;

/// Render a vocal MIDI clip + lyric draft to a singing waveform. Returns
/// `None` when the SVS model files aren't installed (the PoC's download
/// script hasn't been run, env var unset, etc.) so the caller can fall
/// back to its existing MIDI-only behaviour silently.
///
/// `lyrics` is a per-note annotation slice (parallel to `notes`). Notes
/// whose lyric equals the OpenUtau slur marker (`"+"`) get treated as
/// melisma continuations of the previous syllable instead of consuming
/// a fresh phoneme list. An empty `lyrics` slice is equivalent to the
/// legacy "every note is its own syllable" mode and is what the engine
/// sees for non-vocal clips rendered through the same code path during
/// testing.
pub fn render_vocal_clip_with_lyrics(
    notes: &[MidiNote],
    params: &VocalParams,
    lyrics: &[String],
    ticks_per_quarter: u32,
    bpm: f32,
    engine_sample_rate: u32,
) -> Result<Option<RenderedVocal>, String> {
    if notes.is_empty() {
        return Ok(None);
    }
    let Some(paths) = locate_voicebank(params.voicebank, params) else {
        return Ok(None);
    };

    let segment = build_segment(notes, params, lyrics, ticks_per_quarter, bpm);
    if segment.ph_seq.is_empty() {
        return Ok(None);
    }

    let args = PipelineArgs {
        ds_file: PathBuf::new(),
        acoustic_config: paths.acoustic_config,
        vocoder_config: paths.vocoder_config,
        out: PathBuf::new(),
        execution_provider: ExecutionProvider::Cpu,
        device_index: 0,
        speaker: paths.speaker,
        // The TIGER voicebank's working CLI command uses speedup=20.
        // (We previously thought 20 produced diffusion artifacts, but
        // those artifacts were actually from feeding the mel into the
        // wrong vocoder.)
        speedup: 20,
        depth: 1000,
    };

    // Pre-compute AP/SP intervals from the segment so we can gate the
    // rendered audio later. Must be done before `render_segments`
    // consumes the segment.
    let ap_intervals = collect_ap_intervals(&segment.ph_seq, &segment.ph_dur);

    let rendered = pipeline::render_segments(&[segment], &args)
        .map_err(|e| format!("svs render: {e:#}"))?;
    let model_sr = rendered.sample_rate;
    let mut mono: Vec<f32> = rendered.samples;

    apply_ap_gate(&mut mono, &ap_intervals, model_sr);
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
