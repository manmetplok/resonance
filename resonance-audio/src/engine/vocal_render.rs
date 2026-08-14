//! Vocal-tuning retune for the render + bounce path (doc #160, todo #358).
//!
//! Turns a clip's non-destructive [`VocalTuning`] edits into corrected PCM
//! that the shared mixer reads. The work is split so the realtime thread
//! never pays for it:
//!
//! * [`ensure_tuning_caches`] runs **off** the realtime thread (the engine
//!   control thread / offline bounce-export workers). It takes the clips
//!   write lock once and, for every clip whose tuning carries edits,
//!   rebuilds its [`AudioClip::tuning_render_cache`] with the
//!   formant-preserving resynthesis from `resonance-dsp`. Untuned / identity
//!   clips have their cache cleared, restoring the zero-overhead path.
//! * The mixer hot path then reads [`AudioClip::render_frames`], which hands
//!   back the cached corrected buffer (or the untouched source).
//!
//! NOTE on live playback: the cache is only ever built by the four
//! offline paths (bounce / wav / stem / freeze) -- nothing on the engine
//! control thread calls [`ensure_tuning_caches`] -- so live playback
//! reflects a tuning edit only after an export has run. It is not the
//! "identical in playback and in bounce" guarantee this comment used to
//! claim. Today that is invisible because nothing in production can set
//! `has_edits()`: there is no tuning-edit `AudioCommand`, the app sends
//! only `AnalyzeClipPitch`, and the default correction amount is 0.0.
//! Whoever lands the tuning-edit handlers has to call
//! [`ensure_tuning_caches`] from the control thread (or rebuild per
//! clip on edit) to make the guarantee real.
//!
//! The original [`ClipSource`](crate::types::ClipSource) PCM is never
//! mutated — the cache is a separate owned buffer with the same frame
//! layout, so trim/fade/gain indexing is unchanged.
//!
//! # Pitch model
//!
//! The effective per-frame pitch ratio blends the *detected* pitch toward a
//! *target* by the per-note correction strength times the global correction
//! amount, with the target snapped to the in-key scale degree via
//! `resonance-music-theory`:
//!
//! ```text
//! eff          = note.correction_strength · global.correction_amount
//! target_grid  = snap(note.mean + note.semitone_offset)   // scale-snapped
//! centre_delta = (target_grid − note.mean) · eff           // how far the note moves
//! Δ(frame)     = centre_delta + dev(frame) · (drift − 1)    // keep/flatten vibrato
//! ratio(frame) = 2^(Δ(frame) / 12)
//! ```
//!
//! where `dev(frame)` is the detected cents-deviation from the note's mean
//! (its vibrato/drift). `drift = 1` keeps the natural deviation; `drift = 0`
//! flattens it toward a steady tone. With no correction and full drift the
//! ratio is unity, so an analysed-but-untouched clip resynthesises to within
//! numerical precision of the original.

use std::sync::Arc;

use parking_lot::RwLock;
use resonance_dsp::FormantShifter;
use resonance_music_theory::pitch::PitchClass;
use resonance_music_theory::scale::{Mode, Scale};

use crate::types::{AudioClip, TuningScale, VocalTuning};

/// Rebuild (or clear) the [`AudioClip::tuning_render_cache`] of every clip in
/// `clips`, so the next render reads correctly-tuned audio. Call once before
/// an offline render loop, or whenever vocal-tuning edits change, from a
/// thread that may block — never from the realtime audio callback.
///
/// Clips whose tuning carries edits ([`VocalTuning::has_edits`]) get a fresh
/// corrected buffer; all others have their cache cleared, restoring the
/// zero-overhead source path. Returns the number of clips whose cache was
/// (re)built, mainly for tests/telemetry.
pub fn ensure_tuning_caches(clips: &Arc<RwLock<Vec<AudioClip>>>, sample_rate: u32) -> usize {
    let mut guard = clips.write();

    // Nothing to do — and nothing stale to clear — when no clip is tuned and
    // none holds a leftover cache. Keeps the common (untuned) project off the
    // FFT-shifter allocation entirely.
    let any_tuned = guard.iter().any(|c| {
        c.tuning_render_cache.is_some()
            || c.vocal_tuning.as_ref().is_some_and(|t| t.has_edits())
    });
    if !any_tuned {
        return 0;
    }

    // One shifter for the whole pass: it holds the FFT plans + window and is
    // re-entrant, so it can retune every clip without per-clip setup cost.
    let shifter = FormantShifter::new(sample_rate as f32);
    let mut rebuilt = 0;
    for clip in guard.iter_mut() {
        let cache = match clip.vocal_tuning.as_ref() {
            Some(tuning) if tuning.has_edits() => {
                rebuilt += 1;
                Some(retune_clip(clip.source.as_frames(), &shifter, tuning))
            }
            _ => None,
        };
        clip.tuning_render_cache = cache;
    }
    rebuilt
}

/// Resynthesise `source` (interleaved stereo `[l, r, …]`) into a corrected
/// interleaved-stereo buffer of the **same length**, applying `tuning`'s
/// per-frame pitch ratio with the formant-preserving `shifter`.
///
/// Mono or empty input is returned unchanged. The two channels are shifted
/// independently (each keeps its own phase coherence) by one shared ratio
/// curve, matching the single monophonic f0 contour the analysis produced.
pub fn retune_clip(source: &[f32], shifter: &FormantShifter, tuning: &VocalTuning) -> Vec<f32> {
    let total_frames = source.len() / 2;
    if total_frames == 0 {
        return source.to_vec();
    }

    // De-interleave into per-channel buffers for the shifter.
    let mut left = Vec::with_capacity(total_frames);
    let mut right = Vec::with_capacity(total_frames);
    for f in source.chunks_exact(2) {
        left.push(f[0]);
        right.push(f[1]);
    }

    let ratio_curve = pitch_ratio_curve(tuning, total_frames);
    let (l_out, r_out) = shifter.process_stereo(&left, &right, &ratio_curve);

    // Re-interleave; guard the lengths in case the shifter ever returns a
    // shorter buffer so indexing stays in bounds.
    let mut out = vec![0.0f32; total_frames * 2];
    for i in 0..total_frames {
        if let Some(&l) = l_out.get(i) {
            out[i * 2] = l;
        }
        if let Some(&r) = r_out.get(i) {
            out[i * 2 + 1] = r;
        }
    }
    out
}

/// Build the per-source-frame pitch-ratio curve (one frequency multiplier
/// per stereo frame) the [`FormantShifter`] sweeps across the clip.
///
/// Frames outside every detected note — and notes whose effective correction
/// and drift leave the pitch unchanged — get a unity ratio (`1.0`), so the
/// shifter passes those frames through untouched. Exposed (not just used
/// internally) so the model can be unit-tested without running the FFT.
pub fn pitch_ratio_curve(tuning: &VocalTuning, total_frames: usize) -> Vec<f32> {
    let mut curve = vec![1.0f32; total_frames];
    if total_frames == 0 {
        return curve;
    }

    let scale = Scale::new(
        PitchClass::from_semitone(tuning.global.key),
        to_mode(tuning.global.scale),
    );
    let global_amount = tuning.global.correction_amount.clamp(0.0, 1.0);

    for note in &tuning.notes {
        let start = (note.start_frame as usize).min(total_frames);
        let end = (note.end_frame as usize).min(total_frames);
        if end <= start {
            continue;
        }

        let edit = note.edit;
        let eff = edit.correction_strength.clamp(0.0, 1.0) * global_amount;
        let drift = edit.drift.clamp(0.0, 1.0);

        // Scale-snapped target, then blend the note centre from detected mean
        // toward it by the effective strength.
        let target = note.mean_pitch_midi + edit.semitone_offset;
        let target_grid = scale.snap_pitch(target, 1.0);
        let centre_delta = (target_grid - note.mean_pitch_midi) * eff;

        // When the note neither moves its centre nor flattens its vibrato the
        // ratio is unity for the whole span — leave the curve untouched so the
        // shifter passes the frames through (cheaper, exact reconstruction).
        if centre_delta == 0.0 && (drift - 1.0).abs() <= f32::EPSILON {
            continue;
        }

        let span = (end - start) as f32;
        for (i, f) in (start..end).enumerate() {
            let pos = if span > 1.0 { i as f32 / (span - 1.0) } else { 0.0 };
            let dev_semitones = sample_cents(&note.cents_contour, pos) / 100.0;
            let delta = centre_delta + dev_semitones * (drift - 1.0);
            curve[f] = 2.0f32.powf(delta / 12.0);
        }
    }

    curve
}

/// Map the engine's dependency-free [`TuningScale`] onto the music-theory
/// [`Mode`] whose interval table it mirrors. `Chromatic` disables snapping.
fn to_mode(scale: TuningScale) -> Mode {
    match scale {
        TuningScale::Chromatic => Mode::Chromatic,
        TuningScale::Major => Mode::Major,
        TuningScale::Minor => Mode::Minor,
        TuningScale::Dorian => Mode::Dorian,
        TuningScale::Phrygian => Mode::Phrygian,
        TuningScale::Lydian => Mode::Lydian,
        TuningScale::Mixolydian => Mode::Mixolydian,
        TuningScale::Locrian => Mode::Locrian,
        TuningScale::HarmonicMinor => Mode::HarmonicMinor,
        TuningScale::MelodicMinor => Mode::MelodicMinor,
    }
}

/// Sample a note's per-analysis-frame cents-deviation contour at normalised
/// position `pos ∈ [0, 1]`, linearly interpolating between points. An empty
/// contour (deviation unknown / flat) gives `0.0` cents.
fn sample_cents(contour: &[f32], pos: f32) -> f32 {
    match contour {
        [] => 0.0,
        [only] => *only,
        _ => {
            let p = pos.clamp(0.0, 1.0) * (contour.len() - 1) as f32;
            let i = p.floor() as usize;
            if i >= contour.len() - 1 {
                contour[contour.len() - 1]
            } else {
                let frac = p - i as f32;
                contour[i] + (contour[i + 1] - contour[i]) * frac
            }
        }
    }
}
