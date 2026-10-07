//! The off-thread render of one vocal clip: split it into render units,
//! run the units the cache does not hold through the acoustic + vocoder
//! pipeline, and post-process the stitched take into engine-ready stereo.

use std::path::PathBuf;

use resonance_audio::types::MidiNote;
use resonance_music_theory::g2p::AssignedSyllable;
use resonance_music_theory::VocalParams;
use resonance_svs::pipeline::{self, PipelineArgs};
use resonance_svs::stages::common::ExecutionProvider;

use crate::compose::expression::ExpressionCurves;

use super::dynamics::{apply_dynamics_gain, velocity_gain_spans_with_breaths};
use super::rests::breath_lead_secs;
use super::post::{
    apply_ap_gate, collect_rest_intervals, mono_to_stereo_with_gain, resample_to,
    safety_gain_factor,
};
use super::{
    render_units_cached, split_render_units, BankProfile, SvsRenderCache, VocalRenderError,
    SEGMENT_PAD_SEC,
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

/// Render a vocal MIDI clip + lyric draft to a singing waveform with
/// `bank`. `Ok(None)` when there are no notes; an error when the bank (or
/// its vocoder) is not installed ([`VocalRenderError::VoicebankMissing`],
/// naming `install_dir` — the user voicebank folder — as where it goes),
/// when nothing resolves to phonemes, or when the pipeline fails.
///
/// `assigned` is the per-note resolved + voicebank-validated syllable
/// stream (one entry per note), produced on the main thread by
/// [`resolve_clip_with_bank`](super::resolve_clip_with_bank) +
/// [`validate_for_voicebank`](super::validate_for_voicebank) so
/// per-syllable overrides, dictionary entries (the project's and the
/// bank's) and the voicebank's substitutions are already folded into the
/// phonemes the model sings. Slur notes hold the previous vowel (the last
/// one also its coda); an empty `assigned` slice renders nothing.
///
/// `cache` carries per-unit rendered audio across renders of the same
/// clip so an edit only re-renders the segments it touched (see
/// [`render_cache`]); pass a fresh [`SvsRenderCache`] for a clip that has
/// never been rendered. After the call its [`SvsRenderCache::last_plan`]
/// holds the "N of M segments changed" tally for the UI overlay.
///
/// [`render_cache`]: super::render_cache
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
    install_dir: Option<PathBuf>,
) -> Result<Option<RenderedVocal>, VocalRenderError> {
    if notes.is_empty() {
        return Ok(None);
    }
    let target = bank
        .render_target(params.speaker.as_deref(), params.voice, install_dir)
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
    // gating its own rests to silence (breaths stay); cache hits skip all
    // of this.
    let render_fn = |segment: &resonance_svs::ds::DsSegment| -> Result<(Vec<f32>, u32), String> {
        let ap_intervals = collect_rest_intervals(&segment.ph_seq, &segment.ph_dur);
        let rendered = pipeline::render_segments(std::slice::from_ref(segment), &args)
            .map_err(|e| format!("svs render: {e:#}"))?;
        let sr = rendered.sample_rate;
        let mut mono = rendered.samples;
        apply_ap_gate(&mut mono, &ap_intervals, sr);
        Ok((mono, sr))
    };

    let stitched = render_units_cached(&units, cache, render_fn)?;
    let model_sr = stitched.sample_rate;
    let mut mono = stitched.mono;

    // Velocity → loudness on every bank (§9): one gain per note, on the
    // stitched take (the first note sits `SEGMENT_PAD_SEC` in), before the
    // safety gain so a +6 dB note cannot clip. A written breath takes
    // the gain of the note it leads into.
    let breaths = breath_lead_secs(notes, assigned, ticks_per_quarter, bpm, params);
    let spans = velocity_gain_spans_with_breaths(
        notes,
        &breaths,
        ticks_per_quarter,
        bpm,
        params.velocity_dynamics,
        SEGMENT_PAD_SEC,
    );
    apply_dynamics_gain(&mut mono, &spans, model_sr);

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
