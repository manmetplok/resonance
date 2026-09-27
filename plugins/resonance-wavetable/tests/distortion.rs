//! The distortion character work: the master stage's modes, oversampling,
//! tone and auto gain, and the per-voice pre-filter drive.
//!
//! Sound-neutral defaults are pinned twice: here, sample for sample
//! against the original `Distortion::process`, and by the untouched
//! goldens in `render_block_regression.rs` / `null_test.rs`, whose
//! scenarios run the distortion enabled at several drives.

use resonance_dsp::OversampleFactor;
use resonance_plugin::param::Param;
use resonance_plugin::{EventIterator, NoteEvent};
use resonance_wavetable::dsp::effects::{
    voice_saturate, DistMode, DistSettings, Distortion, DistortionStage, TONE_OPEN_HZ,
};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::dsp::modulation::ModDest;
use resonance_wavetable::params::WavetableParams;
use std::f64::consts::TAU;

const SR: f32 = 48_000.0;

const MODES: [DistMode; 6] = [
    DistMode::Soft,
    DistMode::Tube,
    DistMode::Fold,
    DistMode::Hard,
    DistMode::Crush,
    DistMode::Rectify,
];
const FACTORS: [OversampleFactor; 3] =
    [OversampleFactor::Off, OversampleFactor::X2, OversampleFactor::X4];

fn settings(mode: DistMode, oversample: OversampleFactor) -> DistSettings {
    DistSettings {
        mode,
        oversample,
        ..DistSettings::default()
    }
}

fn stage(s: DistSettings) -> DistortionStage {
    let mut st = DistortionStage::new(SR);
    st.configure(s);
    st
}

fn sine(freq: f64, amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (TAU * freq * i as f64 / SR as f64).sin() as f32 * amp)
        .collect()
}

/// Mono signal through the stage (the same signal on both channels; the
/// left output is returned).
fn run(st: &mut DistortionStage, x: &[f32], drive: f32, mix: f32) -> Vec<f32> {
    x.iter().map(|&s| st.process(s, s, drive, mix).0).collect()
}

/// Peak amplitude of `x` at `freq` (single-bin DFT).
fn tone_level(x: &[f32], freq: f64) -> f64 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (n, &s) in x.iter().enumerate() {
        let ph = TAU * freq * n as f64 / SR as f64;
        re += s as f64 * ph.cos();
        im -= s as f64 * ph.sin();
    }
    2.0 * (re * re + im * im).sqrt() / x.len() as f64
}

fn mean(x: &[f32]) -> f64 {
    x.iter().map(|&s| s as f64).sum::<f64>() / x.len() as f64
}

// ---------------------------------------------------------------------------
// Defaults are the original stage
// ---------------------------------------------------------------------------

#[test]
fn default_settings_are_the_declared_param_defaults() {
    let p = WavetableParams::new();
    let d = DistSettings::default();
    assert_eq!(p.distortion.mode.value(), d.mode as i32);
    assert_eq!(p.distortion.oversample.value(), d.oversample as i32);
    assert_eq!(p.distortion.tone.value(), d.tone_hz);
    assert_eq!(p.distortion.tone.value(), TONE_OPEN_HZ);
    assert_eq!(p.distortion.auto_gain.value(), d.auto_gain);
    assert_eq!(p.distortion.bits.value(), d.bits);
    assert_eq!(p.distortion.crush_rate.value(), d.crush_rate);
    assert_eq!(p.distortion.voice_drive.value(), 0.0);
}

#[test]
fn stage_at_defaults_is_bit_identical_to_the_original_distortion() {
    let mut st = DistortionStage::new(SR);
    let x = sine(1_234.0, 1.3, 2_000);
    for (drive, mix) in [(1.0, 0.5), (3.0, 0.5), (20.0, 1.0), (7.5, 0.0), (1.7, 0.33)] {
        for (i, &s) in x.iter().enumerate() {
            let r = s * -0.7;
            let (a, b) = st.process(s, r, drive, mix);
            let (ea, eb) = Distortion::process(s, r, drive, mix);
            assert_eq!(
                (a.to_bits(), b.to_bits()),
                (ea.to_bits(), eb.to_bits()),
                "sample {i}, drive {drive}, mix {mix}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Every mode is bounded and finite
// ---------------------------------------------------------------------------

#[test]
fn every_mode_is_bounded_and_nan_free() {
    // Loud sine, full-scale noise-ish chirp, DC, an impulse train.
    let mut inputs: Vec<Vec<f32>> = vec![
        sine(110.0, 4.0, 4_800),
        sine(9_000.0, 2.0, 4_800),
        vec![1.5; 4_800],
        (0..4_800).map(|i| if i % 97 == 0 { 8.0 } else { 0.0 }).collect(),
    ];
    let mut seed = 0x1234_5678u32;
    inputs.push(
        (0..4_800)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed as f32 / u32::MAX as f32) * 6.0 - 3.0
            })
            .collect(),
    );

    for mode in MODES {
        for factor in FACTORS {
            for (tone_hz, auto_gain) in [(TONE_OPEN_HZ, false), (1_500.0, true)] {
                let s = DistSettings {
                    tone_hz,
                    auto_gain,
                    bits: 4.0,
                    crush_rate: 6_000.0,
                    ..settings(mode, factor)
                };
                for x in &inputs {
                    let peak_in = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                    for (drive, mix) in [(1.0, 1.0), (20.0, 1.0), (20.0, 0.5)] {
                        let mut st = stage(s);
                        let y = run(&mut st, x, drive, mix);
                        // The wet curves peak at 1.46 (Tube's negative
                        // side, which the DC blocker can push a little
                        // further while it settles); auto gain's makeup
                        // is ≈ 1 at drive 1.
                        // Oversampled, the downsampler's steep half-band
                        // rings: a clipped impulse — a burst of near-
                        // Nyquist content at the high rate — overshoots by
                        // up to its impulse response's L1 norm, so the
                        // oversampled bound is looser.
                        let ring = if factor == OversampleFactor::Off { 1.0 } else { 3.0 };
                        let bound = (peak_in * (1.0 - mix) + 2.0) * ring;
                        for (i, v) in y.iter().enumerate() {
                            assert!(
                                v.is_finite() && v.abs() <= bound,
                                "{mode:?} {factor:?} tone {tone_hz} ag {auto_gain} \
                                 drive {drive} mix {mix}: sample {i} = {v} (bound {bound})"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn voice_saturate_is_bounded_and_continuous_from_zero() {
    for amount in [1e-6f32, 0.01, 0.5, 1.0] {
        for x in [-4.0f32, -1.0, -0.1, 0.0, 0.3, 1.0, 4.0] {
            let (l, r) = voice_saturate(x, -x, amount);
            assert!(l.is_finite() && r.is_finite());
            // Between the clean input and a unit-bounded shaped one.
            assert!(l.abs() <= x.abs().max(1.0) + 1e-6, "{x} @ {amount} -> {l}");
            assert_eq!(l, -r, "odd-symmetric");
        }
    }
    // Continuous at 0: a vanishing amount is (almost) the clean signal.
    let (l, _) = voice_saturate(0.8, 0.8, 1e-6);
    assert!((l - 0.8).abs() < 1e-5);
}

// ---------------------------------------------------------------------------
// Oversampling reduces aliasing
// ---------------------------------------------------------------------------

/// Fraction of the output's power that is *not* at a harmonic of `f0`
/// below Nyquist (nor DC), in dB. For a static waveshaper on a pure sine
/// that is the aliasing. `f0` must sit on a 10 Hz bin (the window is 0.1 s)
/// so the harmonic bins are leakage-free.
fn alias_ratio_db(y: &[f32], f0: f64) -> f64 {
    let total: f64 = y.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / y.len() as f64;
    let mut harmonic = mean(y).powi(2);
    let mut k = 1.0;
    while k * f0 < SR as f64 / 2.0 {
        // Peak amplitude → mean power of a sinusoid.
        harmonic += tone_level(y, k * f0).powi(2) / 2.0;
        k += 1.0;
    }
    10.0 * ((total - harmonic).max(1e-30) / total).log10()
}

fn aliasing_at(mode: DistMode, factor: OversampleFactor, f0: f64, drive: f32) -> f64 {
    let mut st = stage(settings(mode, factor));
    // Settle, then measure a 0.1 s window.
    let x = sine(f0, 0.8, 4_800 * 2);
    let y = run(&mut st, &x, drive, 1.0);
    alias_ratio_db(&y[4_800..], f0)
}

#[test]
fn oversampling_measurably_reduces_aliasing_of_a_high_sine_at_high_drive() {
    // 5.01 kHz at drive 20: every harmonic from the 5th up is above
    // Nyquist, so without oversampling the output is thick with aliases.
    // Measured 2026-09-27 (off / 2x / 4x, dB of non-harmonic power):
    // Soft −11.7 / −22.8 / −40.2, Hard −11.1 / −20.6 / −35.5, Tube −11.7 /
    // −23.0 / −38.6, Fold at drive 2 −30.0 / −73.3 / −98.6.
    //
    // Fold runs at drive 2 because at 20 it folds a 0.8 sine sixteen times
    // per half cycle: its partials reach far past even 4× Nyquist and no
    // oversampling factor helps. Rectify is left out because its large DC
    // is still draining through the 5 Hz blocker in this window, and that
    // slow decay reads as non-harmonic power.
    let f0 = 5_010.0;
    for (mode, drive) in [
        (DistMode::Soft, 20.0),
        (DistMode::Hard, 20.0),
        (DistMode::Tube, 20.0),
        (DistMode::Fold, 2.0),
    ] {
        let off = aliasing_at(mode, OversampleFactor::Off, f0, drive);
        let x2 = aliasing_at(mode, OversampleFactor::X2, f0, drive);
        let x4 = aliasing_at(mode, OversampleFactor::X4, f0, drive);
        assert!(
            off > -40.0,
            "{mode:?}: the unoversampled case should alias audibly, got {off:.1} dB"
        );
        assert!(
            x2 < off - 6.0,
            "{mode:?}: 2x ({x2:.1} dB) must beat off ({off:.1} dB) by 6 dB"
        );
        assert!(
            x4 < x2 - 3.0,
            "{mode:?}: 4x ({x4:.1} dB) must beat 2x ({x2:.1} dB) by 3 dB"
        );
    }
}

#[test]
fn oversampled_dry_path_does_not_comb_filter_the_mix() {
    // At mix 0 the oversampled stage is its up/down filter alone, which is
    // flat across the band: a dry path that skipped the filter's delay
    // while the wet one took it would notch here instead.
    for factor in [OversampleFactor::X2, OversampleFactor::X4] {
        for f in [200.0, 5_000.0, 15_000.0] {
            let mut st = stage(settings(DistMode::Soft, factor));
            let x = sine(f, 0.5, 9_600);
            let y = run(&mut st, &x, 1.0, 0.0);
            let level = tone_level(&y[4_800..], f) / 0.5;
            assert!(
                (level - 1.0).abs() < 0.01,
                "{factor:?} {f} Hz: dry level {level:.4}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Mode character
// ---------------------------------------------------------------------------

#[test]
fn tube_and_rectify_leave_no_dc_after_the_blocker() {
    for mode in [DistMode::Tube, DistMode::Rectify] {
        for factor in FACTORS {
            let mut st = stage(settings(mode, factor));
            // 2 s of a 220 Hz sine; the 5 Hz blocker settles in ~0.2 s.
            let x = sine(220.0, 0.5, 96_000);
            let y = run(&mut st, &x, 10.0, 1.0);
            let tail = &y[48_000..];
            let dc = mean(tail);
            let rms = (tail.iter().map(|&s| (s as f64).powi(2)).sum::<f64>()
                / tail.len() as f64)
                .sqrt();
            assert!(
                dc.abs() < 1e-3 * rms.max(1e-3),
                "{mode:?} {factor:?}: DC {dc:.2e} against rms {rms:.3}"
            );
        }
    }
}

#[test]
fn tube_adds_even_harmonics_and_soft_does_not() {
    let f0 = 220.0;
    let second = |mode| {
        let mut st = stage(settings(mode, OversampleFactor::Off));
        let y = run(&mut st, &sine(f0, 0.5, 9_600), 4.0, 1.0);
        let tail = &y[4_800..];
        tone_level(tail, 2.0 * f0) / tone_level(tail, f0)
    };
    let soft = second(DistMode::Soft);
    let tube = second(DistMode::Tube);
    assert!(soft < 1e-4, "soft (symmetric) 2nd harmonic {soft:.2e}");
    assert!(tube > 0.05, "tube 2nd harmonic only {tube:.3} of the fundamental");
}

#[test]
fn crush_quantises_to_the_bit_depth_and_holds_at_the_crush_rate() {
    let s = DistSettings {
        bits: 3.0,
        crush_rate: 4_800.0,
        ..settings(DistMode::Crush, OversampleFactor::Off)
    };
    let mut st = stage(s);
    let y = run(&mut st, &sine(97.0, 0.9, 4_800), 1.0, 1.0);
    // 3 bits: steps of 1/4.
    for (i, v) in y.iter().enumerate() {
        let steps = v * 4.0;
        assert!(
            (steps - steps.round()).abs() < 1e-6,
            "sample {i} = {v} is not a multiple of 1/4"
        );
    }
    // 4.8 kHz hold at 48 kHz: the value changes at most every 10 samples.
    let mut last_change = 0usize;
    for i in 1..y.len() {
        if y[i] != y[i - 1] {
            assert!(i - last_change >= 10 || last_change == 0, "changed after {}", i - last_change);
            last_change = i;
        }
    }
}

#[test]
fn fold_folds_back_past_unity() {
    let mut st = stage(settings(DistMode::Fold, OversampleFactor::Off));
    // Drive 2 on a 1.0 ramp: past 1 the output must come back down.
    let ramp: Vec<f32> = (0..=100).map(|i| i as f32 / 100.0).collect();
    let y = run(&mut st, &ramp, 2.0, 1.0);
    let peak = y.iter().cloned().fold(f32::MIN, f32::max);
    assert!((peak - 1.0).abs() < 1e-3, "fold peak {peak}");
    assert!(y[100] < 0.1, "u = 2 folds back to ~0, got {}", y[100]);
}

#[test]
fn tone_darkens_the_wet_signal() {
    let level_at = |tone_hz| {
        let s = DistSettings {
            tone_hz,
            ..settings(DistMode::Hard, OversampleFactor::Off)
        };
        let mut st = stage(s);
        let y = run(&mut st, &sine(1_000.0, 0.8, 9_600), 20.0, 1.0);
        // The hard clip's 5th harmonic.
        tone_level(&y[4_800..], 5_000.0)
    };
    let open = level_at(TONE_OPEN_HZ);
    let dark = level_at(800.0);
    assert!(dark < open * 0.3, "tone 800 Hz left the 5th at {dark:.4} vs {open:.4}");
}

/// Auto gain is peak-matched at −12 dBFS: a sine driven square keeps its
/// peak, and reads louder only by its smaller crest factor.
#[test]
fn auto_gain_holds_the_level_across_drive() {
    let peak_at = |drive: f32, auto_gain| {
        let s = DistSettings {
            auto_gain,
            ..settings(DistMode::Soft, OversampleFactor::Off)
        };
        let mut st = stage(s);
        let y = run(&mut st, &sine(440.0, 0.25, 4_800), drive, 1.0);
        y.iter().fold(0.0f32, |m, v| m.max(v.abs()))
    };
    let raw = peak_at(20.0, false) / peak_at(1.0, false);
    let comp = peak_at(20.0, true) / peak_at(1.0, true);
    assert!(raw > 3.0, "drive 20 should raise a -12 dBFS sine a lot, got {raw:.2}x");
    assert!(
        (0.97..1.03).contains(&comp),
        "auto gain should hold its peak, got {comp:.3}x"
    );
}

#[test]
fn switching_oversampling_mid_stream_stays_finite() {
    let mut st = DistortionStage::new(SR);
    let x = sine(3_000.0, 1.0, 1_000);
    for factor in [OversampleFactor::X4, OversampleFactor::Off, OversampleFactor::X2] {
        st.configure(settings(DistMode::Hard, factor));
        for v in run(&mut st, &x, 10.0, 0.7) {
            assert!(v.is_finite() && v.abs() < 3.0);
        }
    }
}

// ---------------------------------------------------------------------------
// Engine: voice drive and the two mod destinations
// ---------------------------------------------------------------------------

const BLOCK: usize = 2_048;
const SRC_VELOCITY: i32 = 5;

fn bare_params() -> WavetableParams {
    let p = WavetableParams::new();
    p.filter.enabled.set_value(false);
    p.chorus.enabled.set_value(false);
    p.delay.enabled.set_value(false);
    p.distortion.enabled.set_value(false);
    p.osc1.enabled.set_value(true);
    p.osc1.level.set_value(1.0);
    p.amp_env.attack.set_value(0.001);
    p.amp_env.decay.set_value(0.001);
    p.amp_env.sustain.set_value(1.0);
    p
}

fn route(p: &WavetableParams, slot: usize, dest: ModDest, amount: f32) {
    p.mod_slots[slot].source.set_plain(SRC_VELOCITY as f64);
    p.mod_slots[slot].destination.set_plain(dest as i32 as f64);
    p.mod_slots[slot].amount.set_value(amount);
}

/// A three-note chord, one block, stereo interleaved into one vec.
fn render_chord(params: &WavetableParams) -> Vec<f32> {
    let mut engine = SynthEngine::new();
    engine.initialize(SR);
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let events = [57u8, 61, 64].map(|note| NoteEvent::NoteOn {
        note,
        velocity: 1.0,
        timing: 0,
    });
    let mut iter = EventIterator::new(&events);
    engine.render_block(&mut left, &mut right, BLOCK, params, &mut iter, None);
    left.extend_from_slice(&right);
    left
}

fn bits(x: &[f32]) -> Vec<u32> {
    x.iter().map(|v| v.to_bits()).collect()
}

#[test]
fn voice_drive_zero_is_bit_identical_even_with_a_zero_route() {
    let plain = bare_params();
    let routed = bare_params();
    routed.distortion.voice_drive.set_value(0.0);
    route(&routed, 0, ModDest::VoiceDrive, 0.0);
    assert_eq!(bits(&render_chord(&plain)), bits(&render_chord(&routed)));

    // And modulation that pulls a set drive back to 0 is the bypass too:
    // 0.5 − velocity(+1)·0.5 clamps to exactly 0.
    let pulled = bare_params();
    pulled.distortion.voice_drive.set_value(0.5);
    route(&pulled, 0, ModDest::VoiceDrive, -0.5);
    assert_eq!(bits(&render_chord(&plain)), bits(&render_chord(&pulled)));
}

#[test]
fn voice_drive_saturates_each_voice_before_the_filter() {
    let clean = render_chord(&bare_params());
    let driven_p = bare_params();
    driven_p.distortion.voice_drive.set_value(0.8);
    let driven = render_chord(&driven_p);
    assert!(clean.iter().zip(&driven).any(|(a, b)| a != b));
    // Per voice, three voices of ≤ 1 each: the sum stays within 3 even
    // at heavy drive, where one clipped bus would have sat at 1.
    let peak = driven.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 1.0 && peak <= 3.0, "chord peak {peak}");

    // Runs with the filter off (above) and on.
    let filtered_p = bare_params();
    filtered_p.filter.enabled.set_value(true);
    filtered_p.distortion.voice_drive.set_value(0.8);
    let filtered_clean_p = bare_params();
    filtered_clean_p.filter.enabled.set_value(true);
    assert_ne!(
        bits(&render_chord(&filtered_p)),
        bits(&render_chord(&filtered_clean_p))
    );
}

#[test]
fn voice_drive_modulation_equals_moving_the_parameter() {
    let modulated = bare_params();
    route(&modulated, 0, ModDest::VoiceDrive, 0.6);
    let reference = bare_params();
    reference.distortion.voice_drive.set_value(0.6);
    assert_eq!(bits(&render_chord(&modulated)), bits(&render_chord(&reference)));
}

#[test]
fn dist_drive_modulation_moves_the_master_drive() {
    let dist_on = |p: &WavetableParams| {
        p.distortion.enabled.set_value(true);
        p.distortion.mix.set_value(1.0);
        p.distortion.drive.set_value(1.0);
    };
    let unmodulated = bare_params();
    dist_on(&unmodulated);

    // Velocity +1 × 0.5 → half the log range: drive 1 → √20.
    let modulated = bare_params();
    dist_on(&modulated);
    route(&modulated, 0, ModDest::DistDrive, 0.5);

    let reference = bare_params();
    dist_on(&reference);
    reference.distortion.drive.set_value(20.0f32.sqrt());

    let m = render_chord(&modulated);
    let u = render_chord(&unmodulated);
    let r = render_chord(&reference);
    assert!(m.iter().zip(&u).any(|(a, b)| a != b), "DistDrive did nothing");
    // After the drive smoother's 5 ms ramp, modulated ≈ the moved param
    // (not bit-exact: one multiplies an exp2, the other ramps to √20).
    let settle = 480;
    for ch in 0..2 {
        for i in settle..BLOCK {
            let (a, b) = (m[ch * BLOCK + i], r[ch * BLOCK + i]);
            assert!((a - b).abs() < 1e-4, "ch {ch} sample {i}: {a} vs {b}");
        }
    }
}

#[test]
fn the_new_destinations_are_labelled_and_available() {
    assert_eq!(ModDest::LABELS[ModDest::DistDrive as usize], "Dist Drive");
    assert_eq!(ModDest::LABELS[ModDest::VoiceDrive as usize], "Voice Drive");
    assert!(ModDest::DistDrive.is_available());
    assert!(ModDest::VoiceDrive.is_available());
    let p = WavetableParams::new();
    // Reachable from the slot's destination parameter.
    let max = p.mod_slots[0].destination.max_plain();
    assert!(max >= ModDest::DistDrive as i32 as f64 && max >= ModDest::VoiceDrive as i32 as f64);
}
