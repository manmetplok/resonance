//! The off-thread render of one vocal clip: split it into render units,
//! run the units the cache does not hold through the acoustic + vocoder
//! pipeline, and post-process the stitched take into engine-ready stereo.

use std::path::PathBuf;

use resonance_audio::types::MidiNote;
use resonance_music_theory::g2p::AssignedSyllable;
use resonance_music_theory::VocalParams;
use resonance_svs::pipeline::{PipelineArgs, Renderer};
use resonance_svs::predict::BankModels;
use resonance_svs::stages::common::ExecutionProvider;

use super::dynamics::{apply_dynamics_gain_layers, velocity_gain_spans_with_breaths, GainSpan};
use super::rests::breath_lead_secs;
use super::post::{
    apply_ap_gate, collect_rest_intervals, mono_to_stereo_with_gain, resample_to,
    safety_gain_factor,
};
use super::bank_render;
use super::{
    render_units_cached_with, split_render_units, BankProfile, RenderUnit, SvsRenderCache,
    UnitAudio, VocalRenderError, SEGMENT_PAD_SEC,
};

/// Sampling steps of the acoustic model (passed as `speedup`; every
/// shipped acoustic model takes `steps`). TIGER's working CLI command used
/// 20; 50 measured no better at 1.7× the time (feature doc § Decisions).
pub(super) const ACOUSTIC_STEPS: i32 = 20;
/// Shallow-diffusion depth asked for; clamped to the bank's `max_depth`.
pub(super) const ACOUSTIC_DEPTH: i32 = 1000;

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
    /// Frames sung before the first note's onset — its onset consonants,
    /// or a written breath — which the clip keeps rather than trims: the
    /// install starts the clip this much before the note.
    pub pre_roll_frames: u64,
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
    let units = split_render_units(notes, params, assigned, ticks_per_quarter, bpm, bank);
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
        speedup: ACOUSTIC_STEPS,
        depth: ACOUSTIC_DEPTH,
        // Each unit is rendered with its content key as the seed (below).
        seed: 0,
    };

    // Render each missing unit through the acoustic + vocoder pipeline,
    // gating its own rests to silence (breaths stay); cache hits skip all
    // of this. The sessions load once, on the first unit that misses. A
    // unit the lane times or pitches with the bank's own models
    // (`bank_timing` / `bank_pitch`) is re-timed and re-pitched first.
    // Every model's noise is seeded with the unit's content key, so a
    // unit renders the same samples every time.
    let speaker = args.speaker.clone();
    let mut renderer: Option<Renderer> = None;
    let mut models: Option<BankModels> = None;
    let render_fn = |unit: &RenderUnit| -> Result<UnitAudio, String> {
        let (segment, fallback) = if bank_render::uses_bank_models(unit) {
            let models = models.get_or_insert_with(|| bank_render::load_models(unit));
            bank_render::refine_segment(unit, models, speaker.as_deref(), bank)
        } else {
            (unit.segment.clone(), false)
        };
        if renderer.is_none() {
            renderer = Some(Renderer::load(&args).map_err(|e| format!("svs render: {e:#}"))?);
        }
        let renderer = renderer.as_mut().expect("loaded above");
        let ap_intervals = collect_rest_intervals(&segment.ph_seq, &segment.ph_dur);
        let mut mono = renderer
            .render(&segment, bank_render::unit_seed(unit))
            .map_err(|e| format!("svs render: {e:#}"))?;
        let sample_rate = renderer.sample_rate();
        apply_ap_gate(&mut mono, &ap_intervals, sample_rate);
        Ok(UnitAudio {
            mono,
            sample_rate,
            lead_sec: segment.ph_dur.first().copied().unwrap_or(SEGMENT_PAD_SEC),
            fallback,
        })
    };

    let stitched = render_units_cached_with(&units, cache, render_fn)?;
    // What the first unit sings before its first note: the leading pad
    // minus the silence left in front (onset consonants or a breath took
    // the rest). The first unit starts the take, so its pad is the take's.
    let pre_roll_sec = (SEGMENT_PAD_SEC - stitched.first_lead_sec).clamp(0.0, SEGMENT_PAD_SEC);
    let model_sr = stitched.sample_rate;
    let mut mono = stitched.mono;

    // Velocity → loudness on every bank (§9): one gain per note, on the
    // stitched take (the first note sits `SEGMENT_PAD_SEC` in), before the
    // safety gain so a +6 dB note cannot clip. A written breath takes
    // the gain of the note it leads into. The Dynamics curve on a bank
    // without energy (§10) is a second layer, one span per curve frame,
    // each unit's spans moved to where the unit sits in the take; the two
    // layers' gains multiply.
    let breaths = breath_lead_secs(notes, assigned, ticks_per_quarter, bpm, params);
    let spans = velocity_gain_spans_with_breaths(
        notes,
        &breaths,
        ticks_per_quarter,
        bpm,
        params.velocity_dynamics,
        SEGMENT_PAD_SEC,
    );
    let curve_spans: Vec<GainSpan> = units
        .iter()
        .flat_map(|unit| {
            unit.curve_gain.iter().map(move |s| GainSpan {
                start_sec: unit.start_sec + s.start_sec,
                end_sec: unit.start_sec + s.end_sec,
                gain_db: s.gain_db,
            })
        })
        .collect();
    apply_dynamics_gain_layers(&mut mono, &[&spans, &curve_spans], model_sr);

    let gain = safety_gain_factor(&mono);
    let stereo = mono_to_stereo_with_gain(&mono, gain);
    let (samples_stereo, out_sr) = resample_to(stereo, model_sr, engine_sample_rate);

    // The segment was rendered with `SEGMENT_PAD_SEC` of AP on each
    // end (matches the reference `.ds` fixtures so the model ramps in
    // and out cleanly). Translate that pad into frame counts at the
    // *final* sample rate so the engine's clip trim skips past it on
    // playback — keeps the timeline alignment between MIDI and audio.
    let pad_frames = super::pad_frames(out_sr);
    let trim_start_frames = ((SEGMENT_PAD_SEC - pre_roll_sec) * out_sr as f64) as u64;

    Ok(Some(RenderedVocal {
        samples_stereo,
        sample_rate: out_sr,
        trim_start_frames,
        trim_end_frames: pad_frames,
        pre_roll_frames: pad_frames.saturating_sub(trim_start_frames),
    }))
}
