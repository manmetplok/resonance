//! f0 curve construction. Three passes:
//!  1. Sample a piecewise-constant f0 curve from the phoneme/note
//!     duration arrays, with continuous carrier-pitch fill across rest
//!     phonemes (silence is signalled by `ph_seq[i] == "AP"`, not by
//!     `f0 == 0`).
//!  2. Apply linear portamento across adjacent voiced step changes —
//!     the reference fixtures train the model on real human pitch
//!     curves that always slide between notes.
//!  3. Apply per-frame sinusoidal vibrato, gated by note duration and
//!     a brief onset ramp.
//!
//! The per-frame metadata arrays produced here (`frame_velocity`,
//! `frame_note_total_sec`, `frame_in_note_sec`) carry the segment's
//! note-level information down to the tension curve.

use resonance_music_theory::VocalParams;

use super::duration::PhonemeTrack;

/// Sampling interval for the f0 / gender / tension `SampleCurve`s.
/// The pipeline resamples to its internal frame rate; we just need a
/// grid dense enough to capture every note boundary.
pub(super) const F0_TIMESTEP: f64 = 0.005;

/// Output of [`build_f0_curve`]: the per-frame f0 samples plus the
/// parallel metadata arrays the tension curve needs.
pub(super) struct F0Curve {
    pub samples: Vec<f64>,
    pub frame_velocity: Vec<f32>,
    pub frame_note_total_sec: Vec<f64>,
    pub frame_in_note_sec: Vec<f64>,
}

/// Build the f0 curve and per-frame metadata, then apply portamento
/// and vibrato in-place.
pub(super) fn build_f0_curve(track: &PhonemeTrack, params: &VocalParams) -> F0Curve {
    let mut curve = sample_piecewise_constant(track);
    fill_unvoiced(&mut curve.samples);
    apply_portamento(&mut curve.samples, params.portamento_ms);
    apply_vibrato(&mut curve, params);
    curve
}

/// Step 1: sample the f0 grid + parallel per-frame metadata. Each
/// frame inherits its parent phoneme-entry's note metadata so the
/// tension curve and vibrato gate know which note a given frame
/// belongs to.
fn sample_piecewise_constant(track: &PhonemeTrack) -> F0Curve {
    // f0_seq: piecewise constant pitch following the note sequence. The
    // pipeline resamples this to its internal frame rate; we just need a
    // grid dense enough to capture every note boundary.
    let total_sec: f64 = track.ph_dur.iter().sum();
    let n_samples = (total_sec / F0_TIMESTEP).ceil() as usize + 1;
    let mut samples = Vec::with_capacity(n_samples);
    // Parallel per-frame metadata for the dynamic tension curve and
    // vibrato gate. Filled in lockstep with `samples` so each frame
    // knows its parent note's velocity, total duration, and how far
    // we are into the note.
    let mut frame_velocity: Vec<f32> = Vec::with_capacity(n_samples);
    let mut frame_note_total_sec: Vec<f64> = Vec::with_capacity(n_samples);
    let mut frame_in_note_sec: Vec<f64> = Vec::with_capacity(n_samples);
    let mut t = 0.0;
    let mut idx = 0;
    let mut accum = track.note_dur.first().copied().unwrap_or(0.0);
    for _ in 0..n_samples {
        while t > accum && idx + 1 < track.note_dur.len() {
            idx += 1;
            accum += track.note_dur[idx];
        }
        let midi = track.note_seq_midi.get(idx).copied().unwrap_or(0);
        let hz = if midi <= 0 { 0.0 } else { midi_to_hz(midi as u8) };
        samples.push(hz);
        // Per-frame metadata: note velocity / duration / elapsed.
        let vel = track.entry_note_velocity.get(idx).copied().unwrap_or(0.0);
        let nts = track.entry_note_total_sec.get(idx).copied().unwrap_or(0.0);
        let entry_start_t = accum - track.note_dur[idx];
        let elapsed_in_entry = (t - entry_start_t).max(0.0);
        let offset = track.entry_note_start_offset.get(idx).copied().unwrap_or(0.0);
        frame_velocity.push(vel);
        frame_note_total_sec.push(nts);
        frame_in_note_sec.push(offset + elapsed_in_entry);
        t += F0_TIMESTEP;
    }
    F0Curve {
        samples,
        frame_velocity,
        frame_note_total_sec,
        frame_in_note_sec,
    }
}

/// Step 2 (preprocessing for portamento + vibrato): fill unvoiced
/// frames (rests, leading/trailing AP) with a continuous carrier
/// pitch. The reference fixtures keep f0 > 0 throughout the segment —
/// silence is signalled by the phoneme being "AP", not by f0 being
/// zero. Zeroing f0 instead causes the vocoder to emit subtle noise
/// during the silence pads (the user's "noise in silent parts"
/// report). Forward-fill from the next voiced frame for the leading
/// pad, then back-fill from the previous voiced frame for everything
/// else.
fn fill_unvoiced(samples: &mut [f64]) {
    let first_voiced_idx = samples.iter().position(|v| *v > 0.0);
    let Some(first_idx) = first_voiced_idx else {
        return;
    };
    let leading_hz = samples[first_idx];
    for v in samples.iter_mut().take(first_idx) {
        *v = leading_hz;
    }
    let mut last_voiced = leading_hz;
    for v in samples.iter_mut().skip(first_idx) {
        if *v > 0.0 {
            last_voiced = *v;
        } else {
            *v = last_voiced;
        }
    }
}

/// Step 3: smooth f0 step jumps between adjacent voiced notes with a
/// brief linear portamento. The reference fixtures train the model on
/// real human pitch curves that always slide between notes, so hard
/// pitch steps at every syllable boundary push the acoustic model into
/// a regime it doesn't render cleanly. The user controls the slide
/// duration (10..200 ms in the inspector); 0 disables portamento
/// entirely (hard step, only useful for stylistic hard-attack
/// effects). Skips frames that are exactly equal to the previous (no
/// slide needed).
fn apply_portamento(samples: &mut [f64], portamento_ms: f32) {
    let portamento_sec = (portamento_ms.clamp(0.0, 250.0) as f64) / 1000.0;
    let portamento_frames = (portamento_sec / F0_TIMESTEP).round() as usize;
    if portamento_frames < 2 || samples.len() <= portamento_frames {
        return;
    }
    let snapshot = samples.to_vec();
    let mut last_change_idx = 0usize;
    let mut last_val = snapshot[0];
    for (i, &cur) in snapshot.iter().enumerate().skip(1) {
        if (cur - last_val).abs() > 0.5 {
            // Pitch change detected at index i. Linearly ramp the
            // previous `portamento_frames` from `last_val` (the
            // pre-change pitch) to `cur`.
            let start = i.saturating_sub(portamento_frames).max(last_change_idx);
            let span = i.saturating_sub(start);
            if span >= 1 {
                for (offset, sample) in samples[start..i].iter_mut().enumerate() {
                    let t = (offset + 1) as f64 / (span + 1) as f64;
                    *sample = last_val * (1.0 - t) + cur * t;
                }
            }
            last_val = cur;
            last_change_idx = i;
        }
    }
}

/// Step 4: sinusoidal modulation of the f0 curve. Rate (4–7 Hz) is
/// user-controlled via `vibrato_rate`; depth scales peak deviation up
/// to ~20 cents at max. Real singers don't apply vibrato to short
/// syllables and let it ramp in after the consonant attack, so we
/// gate two ways:
///   1. Skip notes whose total sing duration is below
///      `VIBRATO_MIN_NOTE_SEC` — too short for vibrato to make
///      musical sense (it'd just sound like a wobble on the
///      consonant).
///   2. Within longer notes, fade vibrato in over
///      `VIBRATO_ONSET_SEC` after the note's start so the
///      consonant attack stays clean.
fn apply_vibrato(curve: &mut F0Curve, params: &VocalParams) {
    const VIBRATO_MIN_NOTE_SEC: f64 = 0.35;
    const VIBRATO_ONSET_SEC: f64 = 0.15;
    let vibrato_depth = params.vibrato.clamp(0.0, 1.0) as f64;
    if vibrato_depth <= 0.001 {
        return;
    }
    let max_cents = 20.0_f64;
    let rate_hz = params.vibrato_rate.clamp(2.0, 10.0) as f64;
    let two_pi = std::f64::consts::TAU;
    for (i, v) in curve.samples.iter_mut().enumerate() {
        if *v <= 0.0 {
            continue;
        }
        let note_dur_s = curve.frame_note_total_sec.get(i).copied().unwrap_or(0.0);
        if note_dur_s < VIBRATO_MIN_NOTE_SEC {
            continue;
        }
        let elapsed = curve.frame_in_note_sec.get(i).copied().unwrap_or(0.0);
        let onset_gain = (elapsed / VIBRATO_ONSET_SEC).clamp(0.0, 1.0);
        if onset_gain <= 0.0 {
            continue;
        }
        let t = i as f64 * F0_TIMESTEP;
        let cents = max_cents * vibrato_depth * onset_gain * (two_pi * rate_hz * t).sin();
        *v *= 2.0_f64.powf(cents / 1200.0);
    }
}

fn midi_to_hz(midi: u8) -> f64 {
    // A4 (MIDI 69) = 440 Hz.
    440.0 * (2.0_f64).powf((midi as f64 - 69.0) / 12.0)
}
