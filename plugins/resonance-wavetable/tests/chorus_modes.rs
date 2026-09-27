//! The chorus's circuit modes: Classic (the original, bit-identical) and the
//! BBD-style Juno I / II / I+II and Ensemble modes.
//!
//! Driven at the `Chorus` level — the modes are a pure function of the input
//! stream and the per-sample arguments, and the engine-level render goldens
//! (`render_block_regression`, `null_test`) already pin the Classic path end
//! to end.

use resonance_dsp::DelayLine;
use resonance_plugin::param::Param;
use resonance_wavetable::dsp::effects::{Chorus, ChorusMode};
use resonance_wavetable::params::WavetableParams;

const SR: f32 = 48_000.0;

const BBD_MODES: [ChorusMode; 4] = [
    ChorusMode::JunoI,
    ChorusMode::JunoII,
    ChorusMode::JunoBoth,
    ChorusMode::Ensemble,
];

fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / SR).sin())
        .collect()
}

/// Run a mono signal (fed identically to both channels) through `mode`.
/// The chorus adopts `mode` immediately — this is what the engine does for a
/// chorus that was off — so no switch crossfade is in the measurement.
fn run(mode: ChorusMode, input: &[f32], depth: f32, noise: f32, mix: f32) -> Vec<(f32, f32)> {
    let mut c = Chorus::new(SR);
    c.set_mode_immediate(mode);
    input
        .iter()
        .map(|&x| c.process_mode(x, x, mode, 1.0, depth, noise, mix))
        .collect()
}

fn rms(xs: impl Iterator<Item = f32>) -> f32 {
    let (sum, n) = xs.fold((0.0f64, 0usize), |(s, n), x| (s + (x as f64).powi(2), n + 1));
    (sum / n.max(1) as f64).sqrt() as f32
}

/// The chorus exactly as it shipped before the modes existed, transcribed
/// from the pre-change `effects.rs`. Classic must match it to the bit.
struct ReferenceChorus {
    delay_l: DelayLine,
    delay_r: DelayLine,
    lfo_phase: f32,
    sample_rate: f32,
}

impl ReferenceChorus {
    fn new(sample_rate: f32) -> Self {
        let max_samples = (sample_rate * 0.02) as usize + 256;
        Self {
            delay_l: DelayLine::new(max_samples),
            delay_r: DelayLine::new(max_samples),
            lfo_phase: 0.0,
            sample_rate,
        }
    }

    fn process(&mut self, left: f32, right: f32, rate_hz: f32, depth: f32, mix: f32) -> (f32, f32) {
        let base_delay = 0.007 * self.sample_rate;
        let mod_range = 0.003 * self.sample_rate * depth;
        let lfo_l = (self.lfo_phase * std::f32::consts::TAU).sin();
        let lfo_r = ((self.lfo_phase + 0.25) * std::f32::consts::TAU).sin();
        let delay_l = base_delay + lfo_l * mod_range;
        let delay_r = base_delay + lfo_r * mod_range;
        self.delay_l.push(left);
        self.delay_r.push(right);
        let wet_l = self.delay_l.tap_linear(delay_l);
        let wet_r = self.delay_r.tap_linear(delay_r);
        self.lfo_phase += rate_hz / self.sample_rate;
        self.lfo_phase -= self.lfo_phase.floor();
        (
            left * (1.0 - mix) + wet_l * mix,
            right * (1.0 - mix) + wet_r * mix,
        )
    }
}

#[test]
fn classic_is_bit_identical_to_the_original_chorus() {
    let l = sine(220.0, 0.7, 20_000);
    let r = sine(331.0, 0.5, 20_000);
    let mut reference = ReferenceChorus::new(SR);
    let mut chorus = Chorus::new(SR);
    for i in 0..l.len() {
        // Vary the arguments so every term of the formula is exercised; a
        // non-zero noise must not leak into Classic either.
        let rate = 0.3 + (i / 4000) as f32 * 0.7;
        let depth = 0.2 + (i % 7) as f32 * 0.1;
        let mix = 0.25 + (i / 5000) as f32 * 0.2;
        let want = reference.process(l[i], r[i], rate, depth, mix);
        let got = chorus.process_mode(l[i], r[i], ChorusMode::Classic, rate, depth, 0.8, mix);
        assert_eq!(
            (got.0.to_bits(), got.1.to_bits()),
            (want.0.to_bits(), want.1.to_bits()),
            "Classic diverged from the original chorus at sample {i}"
        );
    }
}

#[test]
fn defaults_select_classic_with_no_noise() {
    let p = WavetableParams::new();
    assert_eq!(
        ChorusMode::from_int(p.chorus.mode.value()),
        ChorusMode::Classic
    );
    assert_eq!(p.chorus.noise.value(), 0.0);
    assert_eq!(p.chorus.mode.min_plain(), 0.0);
    assert_eq!(
        p.chorus.mode.max_plain(),
        (ChorusMode::LABELS.len() - 1) as f64
    );
    for (i, label) in ChorusMode::LABELS.iter().enumerate() {
        let mode = ChorusMode::from_int(i as i32);
        assert_eq!(mode as usize, i, "ChorusMode::from_int({i}) round-trip");
        assert_eq!(&mode.label(), label);
        assert_eq!(p.chorus.mode.display(i as f64), *label);
    }
}

#[test]
fn every_mode_stays_bounded_and_finite() {
    // Full-scale square-ish input (a hard-clipped sine is the worst case for
    // the BBD saturator), maximum depth, noise and mix.
    let input: Vec<f32> = sine(97.0, 3.0, 48_000)
        .into_iter()
        .map(|x| x.clamp(-1.0, 1.0))
        .collect();
    for mode in [ChorusMode::Classic].into_iter().chain(BBD_MODES) {
        for &(l, r) in &run(mode, &input, 1.0, 1.0, 1.0) {
            assert!(l.is_finite() && r.is_finite(), "{mode:?} produced a non-finite sample");
            assert!(
                l.abs() < 1.6 && r.abs() < 1.6,
                "{mode:?} left its bounds: ({l}, {r})"
            );
        }
    }
}

#[test]
fn bbd_modes_decorrelate_a_mono_input() {
    let input = sine(1000.0, 0.5, 48_000);
    for mode in BBD_MODES {
        let out = run(mode, &input, 0.5, 0.0, 1.0);
        // Skip the first 100 ms: the line is still filling.
        let tail = &out[4800..];
        let side = rms(tail.iter().map(|&(l, r)| l - r));
        let mid = rms(tail.iter().map(|&(l, r)| 0.5 * (l + r)));
        assert!(
            side > 0.05 * mid,
            "{mode:?}: a mono input came out near-mono (side {side}, mid {mid})"
        );
    }
}

#[test]
fn juno_modes_ignore_the_rate_parameter() {
    let input = sine(440.0, 0.5, 9_600);
    for mode in [ChorusMode::JunoI, ChorusMode::JunoII, ChorusMode::JunoBoth] {
        assert!(!mode.uses_rate() && mode.fixed_rate_label().is_some());
        let mut a = Chorus::new(SR);
        let mut b = Chorus::new(SR);
        a.set_mode_immediate(mode);
        b.set_mode_immediate(mode);
        for &x in &input {
            let ya = a.process_mode(x, x, mode, 0.1, 0.5, 0.0, 0.5);
            let yb = b.process_mode(x, x, mode, 5.0, 0.5, 0.0, 0.5);
            assert_eq!(ya, yb, "{mode:?} reacted to Rate");
        }
    }
    assert!(ChorusMode::Classic.uses_rate() && ChorusMode::Ensemble.uses_rate());
}

#[test]
fn bbd_wet_path_is_band_limited() {
    for mode in BBD_MODES {
        // Mix 100 %: the whole output is the wet path.
        let hi_in = sine(15_000.0, 0.5, 24_000);
        let lo_in = sine(1_000.0, 0.5, 24_000);
        let hi = run(mode, &hi_in, 0.5, 0.0, 1.0);
        let lo = run(mode, &lo_in, 0.5, 0.0, 1.0);
        let gain = |out: &[(f32, f32)], inp: &[f32]| {
            20.0 * (rms(out[4800..].iter().map(|p| p.0)) / rms(inp[4800..].iter().copied()))
                .log10()
        };
        let g_hi = gain(&hi, &hi_in);
        let g_lo = gain(&lo, &lo_in);
        // The passband is not flat for Ensemble — three taps a few ms apart
        // partly cancel wherever their phases disagree — so the 15 kHz
        // figure is also held against the 1 kHz one, not just against unity.
        assert!(g_hi < -12.0, "{mode:?}: 15 kHz only {g_hi:.1} dB down");
        assert!(
            g_hi < g_lo - 12.0,
            "{mode:?}: 15 kHz ({g_hi:.1} dB) is not well below 1 kHz ({g_lo:.1} dB)"
        );
        assert!(g_lo > -6.0, "{mode:?}: 1 kHz lost {g_lo:.1} dB in the wet path");
    }
}

#[test]
fn zero_noise_adds_nothing() {
    let silence = vec![0.0f32; 24_000];
    for mode in BBD_MODES {
        let quiet = run(mode, &silence, 0.5, 0.0, 1.0);
        assert!(
            quiet.iter().all(|&(l, r)| l == 0.0 && r == 0.0),
            "{mode:?}: noise 0 still produced output from silence"
        );
        let hiss = run(mode, &silence, 0.5, 1.0, 1.0);
        let level = rms(hiss.iter().map(|p| p.0));
        assert!(
            level > 0.0 && level < 0.01,
            "{mode:?}: noise 100 % should be a faint hiss, got rms {level}"
        );
    }
    // Classic has no BBD and never reads Noise.
    let classic = run(ChorusMode::Classic, &silence, 0.5, 1.0, 1.0);
    assert!(classic.iter().all(|&(l, r)| l == 0.0 && r == 0.0));
}

#[test]
fn switching_mode_while_running_does_not_click() {
    // A low sine's biggest sample-to-sample step is tiny; a hard switch
    // between two taps several ms apart would jump by a large fraction of
    // the amplitude.
    let input = sine(110.0, 0.8, 48_000);
    let modes = [
        ChorusMode::Classic,
        ChorusMode::JunoI,
        ChorusMode::Ensemble,
        ChorusMode::JunoBoth,
        ChorusMode::Classic,
    ];
    let mut c = Chorus::new(SR);
    let mut prev = (0.0f32, 0.0f32);
    let mut max_step = 0.0f32;
    for (i, &x) in input.iter().enumerate() {
        let mode = modes[(i / 9600).min(modes.len() - 1)];
        let y = c.process_mode(x, x, mode, 0.5, 1.0, 0.0, 1.0);
        if i > 0 {
            max_step = max_step.max((y.0 - prev.0).abs()).max((y.1 - prev.1).abs());
        }
        prev = y;
    }
    // 0.8 · 2π · 110 / 48000 ≈ 0.0115 is the input's own largest step.
    assert!(max_step < 0.03, "mode switch stepped the output by {max_step}");
    assert_eq!(c.active_mode(), ChorusMode::Classic);
}

#[test]
fn reset_makes_a_bbd_mode_repeatable() {
    let input = sine(300.0, 0.5, 4_800);
    let mut c = Chorus::new(SR);
    c.set_mode_immediate(ChorusMode::Ensemble);
    let first: Vec<_> = input
        .iter()
        .map(|&x| c.process_mode(x, 0.5 * x, ChorusMode::Ensemble, 0.7, 0.6, 0.5, 0.6))
        .collect();
    c.reset();
    let second: Vec<_> = input
        .iter()
        .map(|&x| c.process_mode(x, 0.5 * x, ChorusMode::Ensemble, 0.7, 0.6, 0.5, 0.6))
        .collect();
    assert_eq!(first, second);
}
