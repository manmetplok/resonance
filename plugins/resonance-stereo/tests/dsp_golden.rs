//! Bit-exact DSP golden for the stereo width tool.
//!
//! `tests/stereo.rs` asserts behaviour (mono-sum invariance, the
//! mono-maker's correlation, width/balance/rotation). All of that stays
//! true if someone retunes the velvet seed, the all-pass layout, the
//! micro-shift voices or a crossover; the plugin would sound different
//! and the suite would stay green. This file pins the samples.
//!
//! Every scenario pins **every** parameter (defaults first, then its own
//! values), so a changed default cannot silently move a scenario, and
//! each one is checked to be non-silent and to differ from its input —
//! a scenario that degenerated into a passthrough or silence would pin
//! nothing (silent goldens are vacuous).
//!
//! The path is scalar f32 plus `sin`/`cos`/`exp` in coefficient setup and
//! the rotation; no FFT, no RNG at runtime, no clock. Re-bless after an
//! intentional sound change only:
//!
//!     RESONANCE_BLESS=1 cargo test -p resonance-stereo --test dsp_golden

mod common;

use std::path::PathBuf;

use common::{noise, SR};
use resonance_dsp_test_support as golden;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
use resonance_stereo::dsp::{MonoSlope, WidenMode};
use resonance_stereo::params::{StereoParams, PARAM_COUNT};
use resonance_stereo::ResonanceStereo;

const MAX_BLOCK: usize = 320;
const BLOCKS: usize = 20;

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "dsp_golden.f32")
}

fn blessing() -> bool {
    golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_DSP_GOLDEN"])
}

/// The input: a centred 440 Hz tone, a 55 Hz bass leaning left, and two
/// independent noise beds (so there is side content at every frequency
/// for the mono-maker and width to act on).
fn input(len: usize) -> (Vec<f32>, Vec<f32>) {
    let a = noise(len, 0.2, 101);
    let b = noise(len, 0.2, 202);
    let tau = std::f32::consts::TAU;
    let l = (0..len)
        .map(|i| {
            let t = i as f32 / SR;
            0.3 * (440.0 * tau * t).sin() + 0.3 * (55.0 * tau * t).sin() + a[i]
        })
        .collect();
    let r = (0..len)
        .map(|i| {
            let t = i as f32 / SR;
            0.3 * (440.0 * tau * t).sin() + 0.2 * (55.0 * tau * t).sin() + b[i] + 0.3 * a[i]
        })
        .collect();
    (l, r)
}

type Edit = fn(&StereoParams, usize);

struct Scenario {
    name: &'static str,
    blocks: &'static [usize],
    /// `(param id, plain value)` on top of the declared defaults.
    pins: &'static [(&'static str, f64)],
    edit: Option<Edit>,
}

const DECORRELATE: f64 = 1.0;
const DIFFUSE: f64 = 2.0;
const MICRO: f64 = 3.0;
const HAAS: f64 = 4.0;

fn scenarios() -> Vec<Scenario> {
    // Keep the mode constants in step with the enum they name.
    assert_eq!(WidenMode::Decorrelate.index() as f64, DECORRELATE);
    assert_eq!(WidenMode::Diffuse.index() as f64, DIFFUSE);
    assert_eq!(WidenMode::MicroShift.index() as f64, MICRO);
    assert_eq!(WidenMode::Haas.index() as f64, HAAS);
    assert_eq!(MonoSlope::Db24.index(), 2);
    vec![
        Scenario {
            name: "decorrelate_focused",
            blocks: &[256],
            pins: &[
                ("widen_mode", DECORRELATE),
                ("widen_amount", 0.7),
                ("focus_low", 200.0),
                ("focus_high", 8_000.0),
            ],
            edit: None,
        },
        Scenario {
            name: "diffuse_wide",
            blocks: &[128, 320],
            pins: &[("widen_mode", DIFFUSE), ("widen_amount", 0.6), ("focus_low", 150.0)],
            edit: None,
        },
        Scenario {
            name: "micro_shift_double",
            blocks: &[256, 97],
            pins: &[
                ("widen_mode", MICRO),
                ("widen_amount", 0.5),
                ("focus_low", 250.0),
                ("focus_high", 12_000.0),
            ],
            edit: None,
        },
        Scenario {
            name: "haas_with_exclude",
            blocks: &[192],
            pins: &[("widen_mode", HAAS), ("widen_amount", 0.4), ("focus_low", 250.0)],
            edit: None,
        },
        Scenario {
            name: "width_and_mono_maker",
            blocks: &[256, 64],
            pins: &[("width", 1.6), ("mono_below", 150.0), ("mono_slope", 2.0)],
            edit: None,
        },
        Scenario {
            name: "mono_maker_6db",
            blocks: &[256],
            pins: &[("mono_below", 300.0), ("mono_slope", 0.0), ("width", 0.6)],
            edit: None,
        },
        Scenario {
            name: "balance_rotation",
            blocks: &[160, 320],
            pins: &[("balance", -0.35), ("rotation", 20.0)],
            edit: None,
        },
        Scenario {
            name: "solo_side_audition",
            blocks: &[256],
            pins: &[("solo_side", 1.0), ("width", 1.3)],
            edit: None,
        },
        Scenario {
            name: "sweeps_between_blocks",
            blocks: &[128, 320, 64],
            pins: &[("widen_mode", DECORRELATE)],
            edit: Some(|p, block| {
                let t = block as f32 / BLOCKS as f32;
                p.width.set_value(0.5 + 1.2 * t);
                p.widen_amount.set_value(0.2 + 0.7 * t);
                p.rotation.set_value(-30.0 + 50.0 * t);
                p.balance.set_value(0.4 - 0.6 * t);
                p.mono_below.set_value(if block % 10 >= 5 { 180.0 } else { 0.0 });
                // Mode switches mid-run, so the incoming mode's reset runs.
                p.widen_mode.set_value(match block / 5 {
                    0 => WidenMode::Decorrelate,
                    1 => WidenMode::Diffuse,
                    2 => WidenMode::MicroShift,
                    _ => WidenMode::Haas,
                }
                .index());
            }),
        },
    ]
}

fn pin(p: &StereoParams, pins: &[(&str, f64)]) {
    for i in 0..PARAM_COUNT {
        let q = p.param_at(i);
        q.set_plain(q.default_plain());
    }
    for &(id, v) in pins {
        let q = (0..PARAM_COUNT)
            .map(|i| p.param_at(i))
            .find(|q| q.id() == id)
            .unwrap_or_else(|| panic!("scenario pins unknown param {id}"));
        q.set_plain(v);
        assert_eq!(q.get_plain() as f32, v as f32, "pin {id} = {v} did not land (out of range?)");
    }
}

/// Render one scenario; returns `(output, input)` interleaved per block
/// as L-block then R-block.
fn render_scenario(s: &Scenario) -> (Vec<f32>, Vec<f32>) {
    let mut plugin = ResonanceStereo::new();
    pin(&plugin.params, s.pins);
    plugin.initialize(SR, MAX_BLOCK as u32);
    plugin.reset();

    let total: usize = (0..BLOCKS).map(|b| s.blocks[b % s.blocks.len()]).sum();
    let (il, ir) = input(total);
    let mut out = Vec::with_capacity(total * 2);
    let mut dry = Vec::with_capacity(total * 2);
    let mut pos = 0;
    for block in 0..BLOCKS {
        if let Some(edit) = s.edit {
            edit(&plugin.params, block);
        }
        let frames = s.blocks[block % s.blocks.len()];
        let mut left = il[pos..pos + frames].to_vec();
        let mut right = ir[pos..pos + frames].to_vec();
        {
            let mut outs = [OutputBuffer {
                left: &mut left,
                right: &mut right,
            }];
            plugin.process(&mut outs, frames, &mut EventIterator::empty(), None);
        }
        out.extend_from_slice(&left);
        out.extend_from_slice(&right);
        dry.extend_from_slice(&il[pos..pos + frames]);
        dry.extend_from_slice(&ir[pos..pos + frames]);
        pos += frames;
    }
    (out, dry)
}

fn render_all() -> Vec<f32> {
    scenarios().iter().flat_map(|s| render_scenario(s).0).collect()
}

#[test]
fn stereo_output_is_bit_exact() {
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
            "stereo DSP output changed: {}/{} samples differ, peak delta {:.3e}; first at \
             sample {i} (got {got:?}, want {want:?}). If the change was intended, re-bless \
             with RESONANCE_BLESS=1.",
            diff.diff_count,
            rendered.len(),
            diff.max_abs,
        );
    }
}

/// Guards the scenario table: every scenario renders sound and changes
/// it. A golden over a silent or passthrough scenario pins nothing.
#[test]
fn every_scenario_is_audible_and_does_something() {
    for s in scenarios() {
        let (out, dry) = render_scenario(&s);
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.05, "scenario `{}` rendered near-silence (peak {peak:e})", s.name);
        let residual = golden::residual_fraction(&out, &dry);
        assert!(
            residual > 1e-3,
            "scenario `{}` is (nearly) a passthrough: residual {residual:e}",
            s.name
        );
    }
}
