//! Bit-exact DSP golden for the character plugin.
//!
//! The rest of the suite pins *properties* (harmonic signatures, matched
//! loudness, exact bypass paths). This pins the rendered samples, so a
//! change to a filter coefficient, the ADAA path, the smoothing or the
//! flutter crossfade that keeps every property intact still fails here.
//!
//! Every scenario sets **every** setting explicitly (a full `Settings`
//! literal, no defaults), so a change of default cannot silently move a
//! scenario, and every scenario must render non-silent audio that is not
//! a plain gain of its input — a golden over silence or a bypass pins
//! nothing.
//!
//! # Tolerance: bit-exact
//!
//! The path is scalar: f64 curve maths (`tanh`, `ln`, `sin`, `sqrt` in
//! the antiderivatives), f32 biquads and allpasses, no FFT, no SIMD
//! dispatch, no clock and no RNG (the test signal's noise is seeded).
//! Bits can move between machines only through libm's rounding.
//!
//! To (re)generate after an *intentional* audio change:
//!
//!     RESONANCE_BLESS=1 cargo test -p resonance-color --test dsp_golden

mod common;

use std::path::PathBuf;

use common::*;
use resonance_color::dsp::Settings;
use resonance_color::params::{ColorParams, Mode};
use resonance_color::ResonanceColor;
use resonance_dsp::{OversampleFactor, SimpleRng};
use resonance_dsp_test_support as golden;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const MAX_BLOCK: usize = 320;
const BLOCKS: usize = 24;
const TAU: f32 = std::f32::consts::TAU;

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "dsp_golden.f32")
}

fn blessing() -> bool {
    golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_DSP_GOLDEN"])
}

#[derive(Clone, Copy)]
enum Signal {
    /// A bass note and a mid tone under decaying hits: level-dependent
    /// material for the curves and the HF-loss envelope.
    Program,
    /// A loud 55 Hz sine under a quiet 2 kHz one: what LF weighting and
    /// the head bump act on.
    BassHeavy,
}

impl Signal {
    fn fill(self, n0: u64, l: &mut [f32], r: &mut [f32], rng: &mut SimpleRng) {
        for i in 0..l.len() {
            let n = n0 + i as u64;
            let t = n as f32 / SR;
            let (a, b) = match self {
                Signal::Program => {
                    let x = t % 0.125;
                    let hit = (-30.0 * x).exp();
                    let u = (rng.next_u32() >> 8) as f32 / (1u32 << 24) as f32;
                    let noise = 2.0 * u - 1.0;
                    let v = 0.3 * (TAU * 110.0 * t).sin()
                        + 0.15 * (TAU * 1_300.0 * t).sin()
                        + 0.4 * hit * noise;
                    (v, 0.8 * v + 0.1 * (TAU * 330.0 * t).sin())
                }
                Signal::BassHeavy => {
                    let v = 0.6 * (TAU * 55.0 * t).sin() + 0.05 * (TAU * 2_000.0 * t).sin();
                    (v, v * 0.9)
                }
            };
            l[i] = a;
            r[i] = b;
        }
    }
}

type Edit = fn(&ColorParams, usize);

struct Scenario {
    name: &'static str,
    signal: Signal,
    blocks: &'static [usize],
    settings: Settings,
    edit: Option<Edit>,
}

fn scenarios() -> Vec<Scenario> {
    vec![
        // 1. The default voicing (Tube, 2x) pushed to a track-level drive.
        Scenario {
            name: "tube_track_2x",
            signal: Signal::Program,
            blocks: &[256],
            settings: Settings {
                mode: Mode::Tube,
                drive: 0.6,
                bias: 0.5,
                response_db: 0.0,
                tone_db: 0.0,
                mix: 1.0,
                auto_gain: true,
                output_db: 0.0,
                oversample: OversampleFactor::X2,
                speed_ips: 15.0,
                flutter: 0.0,
            },
            edit: None,
        },
        // 2. Tape at 7.5 ips with flutter on from the first sample, at 1x:
        //    head bump, HF loss, the flutter tap and the ADAA 1x path.
        Scenario {
            name: "tape_7_5_flutter_1x",
            signal: Signal::Program,
            blocks: &[64, 197, 320],
            settings: Settings {
                mode: Mode::Tape,
                drive: 0.65,
                bias: 0.6,
                response_db: 0.0,
                tone_db: -2.0,
                mix: 1.0,
                auto_gain: true,
                output_db: 0.0,
                oversample: OversampleFactor::Off,
                speed_ips: 7.5,
                flutter: 0.6,
            },
            edit: None,
        },
        // 3. Transformer at 4x on bass: the LF-weighted stage, sub-sonic
        //    HPF and HF resonance, plus a response tilt toward the lows.
        Scenario {
            name: "transformer_bass_4x",
            signal: Signal::BassHeavy,
            blocks: &[128, 96],
            settings: Settings {
                mode: Mode::Transformer,
                drive: 0.8,
                bias: 0.4,
                response_db: 6.0,
                tone_db: 1.5,
                mix: 1.0,
                auto_gain: true,
                output_db: -1.0,
                oversample: OversampleFactor::X4,
                speed_ips: 15.0,
                flutter: 0.0,
            },
            edit: None,
        },
        // 4. Console in parallel with auto-gain off and a trim: the one
        //    mode whose drive lives in the curve, blended half and half.
        Scenario {
            name: "console_parallel_no_auto",
            signal: Signal::Program,
            blocks: &[256, 160],
            settings: Settings {
                mode: Mode::Console,
                drive: 1.0,
                bias: 0.0,
                response_db: 0.0,
                tone_db: 0.0,
                mix: 0.5,
                auto_gain: false,
                output_db: -3.0,
                oversample: OversampleFactor::X2,
                speed_ips: 15.0,
                flutter: 0.0,
            },
            edit: None,
        },
        // 5. Warm with the drive weighted toward the highs and a bright
        //    tone, at 1x.
        Scenario {
            name: "warm_highs_bright_1x",
            signal: Signal::Program,
            blocks: &[192, 320],
            settings: Settings {
                mode: Mode::Warm,
                drive: 0.75,
                bias: 0.9,
                response_db: -6.0,
                tone_db: 4.0,
                mix: 0.8,
                auto_gain: true,
                output_db: 0.0,
                oversample: OversampleFactor::Off,
                speed_ips: 15.0,
                flutter: 0.0,
            },
            edit: None,
        },
        // 6. Everything moved between blocks: every smoother mid-ramp,
        //    a mode change, an oversampling change and a flutter
        //    crossfade in one run.
        Scenario {
            name: "param_sweeps_between_blocks",
            signal: Signal::Program,
            blocks: &[128, 320, 64],
            settings: Settings {
                mode: Mode::Tape,
                drive: 0.3,
                bias: 0.2,
                response_db: 0.0,
                tone_db: 0.0,
                mix: 1.0,
                auto_gain: true,
                output_db: 0.0,
                oversample: OversampleFactor::X2,
                speed_ips: 15.0,
                flutter: 0.0,
            },
            edit: Some(|p, block| {
                let t = block as f32 / BLOCKS as f32;
                p.drive.set_value(0.2 + 0.7 * t);
                p.bias.set_value(1.0 - t);
                p.response.set_value(-8.0 + 16.0 * t);
                p.tone.set_value(3.0 - 6.0 * t);
                p.mix.set_value(0.4 + 0.6 * t);
                p.output.set_value(-2.0 + 3.0 * t);
                p.flutter.set_value(if (6..14).contains(&block) { 0.5 } else { 0.0 });
                if block == 16 {
                    p.mode.set_value(Mode::Transformer as i32);
                    p.oversample.set_value(OversampleFactor::X4 as i32);
                    p.speed.set_value(2);
                }
                if block == 20 {
                    p.auto_gain.set_value(false);
                }
            }),
        },
    ]
}

/// Render a scenario. `dry_path` holds `mix` at 0 throughout, which
/// renders the plugin's own dry path — the input through the same
/// oversampler round trip and flutter stage — so the guard below
/// compares against exactly what a do-nothing wet path would give.
fn render_scenario(s: &Scenario, dry_path: bool) -> Vec<f32> {
    let mut plugin = ResonanceColor::new();
    apply_settings(&plugin.params, &s.settings);
    if dry_path {
        plugin.params.mix.set_value(0.0);
    }
    plugin.initialize(SR, MAX_BLOCK as u32);
    plugin.reset();
    let mut rng = SimpleRng::new(0xC0102);
    let mut out = Vec::new();
    let mut left = vec![0.0f32; MAX_BLOCK];
    let mut right = vec![0.0f32; MAX_BLOCK];
    let mut n = 0u64;
    for block in 0..BLOCKS {
        if let Some(edit) = s.edit {
            edit(&plugin.params, block);
            if dry_path {
                plugin.params.mix.set_value(0.0);
            }
        }
        let frames = s.blocks[block % s.blocks.len()];
        s.signal.fill(n, &mut left[..frames], &mut right[..frames], &mut rng);
        let mut outs = [OutputBuffer {
            left: &mut left[..frames],
            right: &mut right[..frames],
        }];
        plugin.process(&mut outs, frames, &mut EventIterator::empty(), None);
        n += frames as u64;
        out.extend_from_slice(&left[..frames]);
        out.extend_from_slice(&right[..frames]);
    }
    out
}

fn render_all() -> Vec<f32> {
    scenarios().iter().flat_map(|s| render_scenario(s, false)).collect()
}

#[test]
fn color_output_is_bit_exact() {
    let rendered = render_all();
    assert!(rendered.iter().all(|s| s.is_finite()), "non-finite samples");
    let path = golden_path();
    if blessing() {
        golden::bless_f32(&path, &rendered);
        return;
    }
    let want = golden::load_golden_f32(&path, rendered.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_f32(&rendered, &want);
    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "color DSP output changed: {}/{} samples differ, peak delta {:.3e}; first at \
             sample {i} (got {got:?}, want {want:?}).\nA refactor must be bit-exact; if the \
             change was intended, re-bless with RESONANCE_BLESS=1.",
            diff.diff_count,
            rendered.len(),
            diff.max_abs,
        );
    }
}

/// Every scenario renders real audio, and audio that is not its own dry
/// path times a constant: the residual after the best-fit gain is a real
/// share of the output's energy.
#[test]
fn every_scenario_is_audible_and_not_a_bypass() {
    for s in scenarios() {
        let out = render_scenario(&s, false);
        let dry = render_scenario(&s, true);
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.05, "scenario `{}` rendered (near) silence: peak {peak}", s.name);
        let resid = golden::residual_fraction(&out, &dry);
        eprintln!("{:28} peak {peak:.3} residual {resid:.2e}", s.name);
        assert!(
            resid > 1e-4,
            "scenario `{}` is a plain gain of its input (residual {resid:.2e}) — a golden \
             over a bypass pins nothing",
            s.name
        );
    }
}
