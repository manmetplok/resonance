//! DSP golden for the W9 mastering extensions (warmth-width-depth.md
//! §6.3): M/S EQ bands, per-band width, the clipper and the saturator
//! modes, each in a scenario of its own on material that exercises it.
//!
//! Every scenario pins every parameter it depends on (a fresh plugin
//! plus explicit settings), and each is checked for non-silence on its
//! own: a silent scenario would match its golden forever. The tolerance
//! is `dsp_golden.rs`'s FFT-rounding budget, for the same reason (rustfft
//! picks its kernel from the CPU at runtime).
//!
//! Also here: the chain's reported latency is the same in every one of
//! these configurations as in the default one.
//!
//! To (re)generate after an intentional change:
//!
//!     RESONANCE_BLESS=1 cargo test -p resonance-mastering --test w9_golden

use std::path::PathBuf;

use resonance_dsp_test_support as golden;
use resonance_mastering::params::eq_stage::BandParams;
use resonance_mastering::params::MasteringParams;
use resonance_mastering::stages::linear_phase_eq::MsMode;
use resonance_mastering::stages::saturator::SatMode;
use resonance_mastering::ResonanceMastering;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
const MAX_BLOCK: usize = 640;
const PRIME_BLOCKS: usize = 56;
const CAPTURE_BLOCKS: usize = 8;
const BLOCKS: usize = PRIME_BLOCKS + CAPTURE_BLOCKS;
const MAX_PEAK_DELTA: f32 = 2.0e-4;
const MAX_RMS_DELTA: f64 = 2.0e-5;
const TAU: f32 = std::f32::consts::TAU;

// Re-blessed once, for W12. The de-harsh stage delays everything after
// the corrective EQ by 2048 samples, even when off. The full pre- and
// post-W12 streams were compared after aligning them by 2048 samples
// (`docs/design/deharsh-resonance-suppressor.md` §5.2). No scenario here
// uses dither.
// - Static scenarios: at most −121.7 dB re peak, which is float
//   rounding from the moved FIR hop grid, with two exceptions:
//   - `imager_band_width_with_multiband`: −84.7 dB, and −97.7 dB in the
//     captured window. The multiband compressors turn rounding into
//     slightly different gain.
//   - `eq_mid_side_bands`: −24.3 dB once, at sample 19295. This is
//     inside the latency pre-fill, where the first M/S FIR designs land
//     on a hop boundary that now meets different audio. From sample
//     24000 (so over the whole capture) it is −131.3 dB.
// - The automated `w9_sweeps_between_blocks`: +1.0 dB. Its block-timed
//   edits now land 2048 samples later in the downstream audio, so this is
//   a content change, not rounding.
// - The "stripped" renders stay at or below −89.1 dB.
//
// Re-blessed again for review finding M1: the M/S cross pair now runs
// on its direct pair's hop grid instead of the half-slots in between.
// The pre- and post-fix streams were compared over 200 blocks. Only the
// two scenarios that run the cross pair moved, and only in the window
// where its first design crossfades in (output samples ~20k-33k): by
// −39.9 dB re peak in `eq_mid_side_bands` and −42.7 dB in
// `w9_sweeps_between_blocks`. That is the misaligned crossfade the fix
// removes. Outside that window it is at most −133 dB (FFT rounding of
// the moved cross grid) in the first, and bit-identical in the second.
fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "w9_golden.f32")
}

fn blessing() -> bool {
    golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_W9_GOLDEN"])
}

fn noise(n: u64) -> f32 {
    let mut s = n.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1) as u32;
    s ^= s >> 16;
    s = s.wrapping_mul(2_246_822_519);
    s ^= s >> 13;
    (s >> 8) as f32 * (2.0 / (1 << 23) as f32) - 1.0
}

#[derive(Clone, Copy)]
enum Signal {
    /// Drum-like hits peaking at +5 dBFS: for the clipper and limiter.
    HotTransients,
    /// Tones in every multiband band plus noise: for the saturator.
    BandedMix,
    /// Large side content at low and high frequencies, plus a mid tone:
    /// for M/S EQ and per-band width.
    WideStereo,
}

impl Signal {
    fn sample(self, n: u64) -> (f32, f32) {
        let t = n as f32 / SR;
        match self {
            Signal::HotTransients => {
                let env = (-22.0 * (t % (1.0 / 9.0))).exp();
                let v = 1.8 * env * ((70.0 * t * TAU).sin() + 0.45 * (1600.0 * t * TAU).sin())
                    + 0.12 * (110.0 * t * TAU).sin();
                (v, v * 0.86 + 0.03 * noise(n))
            }
            Signal::BandedMix => {
                let v = 0.30 * (60.0 * t * TAU).sin()
                    + 0.24 * (400.0 * t * TAU).sin()
                    + 0.18 * (2500.0 * t * TAU).sin()
                    + 0.12 * (9000.0 * t * TAU).sin()
                    + 0.05 * noise(n);
                (v, v * 0.8 + 0.05 * noise(n + 7_919))
            }
            Signal::WideStereo => {
                let mid = 0.3 * (400.0 * t * TAU).sin() + 0.05 * noise(n);
                let side = 0.25 * (45.0 * t * TAU).sin()
                    + 0.15 * (7000.0 * t * TAU).sin()
                    + 0.08 * noise(n + 4_651);
                (mid + side, mid - side)
            }
        }
    }
}

type Edit = fn(&MasteringParams, usize);

struct Scenario {
    name: &'static str,
    signal: Signal,
    blocks: &'static [usize],
    setup: fn(&MasteringParams),
    edit: Option<Edit>,
}

fn eq_band(b: &BandParams, band_type: i32, freq: f32, q: f32, gain_db: f32, ms: MsMode) {
    b.on.set_value(true);
    b.band_type.set_value(band_type);
    b.freq.set_value(freq);
    b.q.set_value(q);
    b.gain.set_value(gain_db);
    b.ms.set_value(ms.to_index());
}

fn sat_mode(p: &MasteringParams, mode: SatMode) {
    p.saturator.on.set_value(true);
    p.saturator.mode.set_value(mode.to_index());
    p.saturator.drive.set_value(6.0);
    p.saturator.mix.set_value(0.7);
    p.saturator.curve.set_value(0.0);
}

fn scenarios() -> Vec<Scenario> {
    vec![
        // Side-only low cut and air shelf, a mid-only bell: the M/S EQ
        // moves §6.3 names, across both EQ stages.
        Scenario {
            name: "eq_mid_side_bands",
            signal: Signal::WideStereo,
            blocks: &[512, 384],
            setup: |p| {
                eq_band(&p.corrective_eq.bands[0], 3, 150.0, 0.707, 0.0, MsMode::Side);
                eq_band(&p.corrective_eq.bands[2], 0, 400.0, 1.0, -3.0, MsMode::Mid);
                eq_band(&p.tonal_eq.bands[3], 2, 9000.0, 0.707, 4.0, MsMode::Side);
                eq_band(&p.tonal_eq.bands[1], 0, 2500.0, 0.8, 2.0, MsMode::Stereo);
            },
            edit: None,
        },
        // Per-band width alone (multiband compression off): low band to
        // mono, top band widened.
        Scenario {
            name: "imager_band_width",
            signal: Signal::WideStereo,
            blocks: &[512, 640],
            setup: |p| {
                p.imager.on.set_value(true);
                p.imager.width.set_value(1.0);
                for (w, v) in p.imager.band_width.iter().zip([0.0, 0.8, 1.3, 1.6]) {
                    w.set_value(v);
                }
            },
            edit: None,
        },
        // Per-band width on top of multiband compression and the global
        // width and side HPF.
        Scenario {
            name: "imager_band_width_with_multiband",
            signal: Signal::WideStereo,
            blocks: &[384, 512],
            setup: |p| {
                p.multiband.on.set_value(true);
                p.multiband.xo1.set_value(150.0);
                for b in &p.multiband.bands {
                    b.on.set_value(true);
                    b.threshold.set_value(-24.0);
                    b.ratio.set_value(3.0);
                }
                p.imager.on.set_value(true);
                p.imager.width.set_value(1.2);
                p.imager.side_hpf_on.set_value(true);
                p.imager.side_hpf_freq.set_value(60.0);
                for (w, v) in p.imager.band_width.iter().zip([0.3, 1.0, 1.0, 1.4]) {
                    w.set_value(v);
                }
            },
            edit: None,
        },
        // Hard clip ahead of the limiter on material 5 dB over it.
        Scenario {
            name: "clipper_hard_into_limiter",
            signal: Signal::HotTransients,
            blocks: &[512, 256],
            setup: |p| {
                p.clipper.on.set_value(true);
                p.clipper.drive.set_value(3.0);
                p.clipper.shape.set_value(0.0);
                p.limiter.on.set_value(true);
                p.limiter.ceiling.set_value(-1.0);
                p.limiter.release.set_value(50.0);
            },
            edit: None,
        },
        // Soft clip alone.
        Scenario {
            name: "clipper_soft",
            signal: Signal::HotTransients,
            blocks: &[640],
            setup: |p| {
                p.clipper.on.set_value(true);
                p.clipper.drive.set_value(6.0);
                p.clipper.shape.set_value(1.0);
            },
            edit: None,
        },
        Scenario {
            name: "sat_tube",
            signal: Signal::BandedMix,
            blocks: &[512],
            setup: |p| sat_mode(p, SatMode::Tube),
            edit: None,
        },
        Scenario {
            name: "sat_tape",
            signal: Signal::BandedMix,
            blocks: &[512],
            setup: |p| sat_mode(p, SatMode::Tape),
            edit: None,
        },
        Scenario {
            name: "sat_transformer",
            signal: Signal::BandedMix,
            blocks: &[512],
            setup: |p| sat_mode(p, SatMode::Transformer),
            edit: None,
        },
        Scenario {
            name: "sat_console",
            signal: Signal::BandedMix,
            blocks: &[512],
            setup: |p| sat_mode(p, SatMode::Console),
            edit: None,
        },
        Scenario {
            name: "sat_warm",
            signal: Signal::BandedMix,
            blocks: &[512],
            setup: |p| sat_mode(p, SatMode::Warm),
            edit: None,
        },
        Scenario {
            name: "sat_inflator",
            signal: Signal::BandedMix,
            blocks: &[512],
            setup: |p| {
                sat_mode(p, SatMode::Inflator);
                p.saturator.drive.set_value(0.0);
                p.saturator.curve.set_value(0.3);
            },
            edit: None,
        },
        // Everything new switched between blocks: an M/S band engaging
        // and releasing (the cross pair's warm-up and drain), band widths
        // moving, the clipper toggling and the saturator walking its
        // modes, into the limiter.
        Scenario {
            name: "w9_sweeps_between_blocks",
            signal: Signal::HotTransients,
            blocks: &[256, 640, 384],
            setup: |p| {
                p.input_trim_db.set_value(-3.0);
                eq_band(&p.tonal_eq.bands[3], 2, 8000.0, 0.707, 3.0, MsMode::Stereo);
                p.imager.on.set_value(true);
                p.saturator.on.set_value(true);
                p.saturator.drive.set_value(4.0);
                p.saturator.mix.set_value(0.5);
                p.limiter.on.set_value(true);
                p.limiter.ceiling.set_value(-1.0);
            },
            edit: Some(|p, block| {
                let side = (12..40).contains(&block);
                let ms = if side { MsMode::Side } else { MsMode::Stereo };
                p.tonal_eq.bands[3].ms.set_value(ms.to_index());
                let t = block as f32 / BLOCKS as f32;
                p.imager.band_width[0].set_value(1.0 - t);
                p.imager.band_width[3].set_value(1.0 + t);
                p.clipper.on.set_value(block % 10 >= 4);
                p.clipper.drive.set_value(1.0 + 4.0 * t);
                p.clipper.shape.set_value(t);
                p.saturator.mode.set_value((block / 9 % 7) as i32);
                p.saturator.curve.set_value(-0.5 + t);
            }),
        },
    ]
}

fn render_scenario(s: &Scenario) -> Vec<f32> {
    let mut plugin = ResonanceMastering::new();
    (s.setup)(plugin.params());
    plugin.initialize(SR, MAX_BLOCK as u32);
    plugin.reset();
    let mut out = Vec::new();
    let mut left = vec![0.0f32; MAX_BLOCK];
    let mut right = vec![0.0f32; MAX_BLOCK];
    let mut n: u64 = 0;
    for block in 0..BLOCKS {
        if let Some(edit) = s.edit {
            edit(plugin.params(), block);
        }
        let frames = s.blocks[block % s.blocks.len()];
        for i in 0..frames {
            let (l, r) = s.signal.sample(n + i as u64);
            left[i] = l;
            right[i] = r;
        }
        {
            let mut outs = [OutputBuffer {
                left: &mut left[..frames],
                right: &mut right[..frames],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, frames, &mut ev, None);
        }
        n += frames as u64;
        if block >= PRIME_BLOCKS {
            out.extend_from_slice(&left[..frames]);
            out.extend_from_slice(&right[..frames]);
        }
    }
    out
}

#[test]
fn w9_output_matches_golden() {
    let mut rendered = Vec::new();
    for s in scenarios() {
        let out = render_scenario(&s);
        assert!(out.iter().all(|x| x.is_finite()), "`{}` rendered non-finite samples", s.name);
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-2, "`{}` rendered silence (peak {peak:.2e})", s.name);
        rendered.extend(out);
    }
    let path = golden_path();
    if blessing() {
        golden::bless_f32(&path, &rendered);
        return;
    }
    let want = golden::load_golden_f32(&path, rendered.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_f32(&rendered, &want);
    eprintln!(
        "w9 golden: {}/{} samples differ | peak delta {:.3e} | rms delta {:.3e}",
        diff.diff_count,
        rendered.len(),
        diff.max_abs,
        diff.rms_err
    );
    if let Some((i, got, want)) = diff.first_diff {
        assert!(
            diff.max_abs <= MAX_PEAK_DELTA && diff.rms_err <= MAX_RMS_DELTA,
            "W9 render moved beyond the FFT-rounding budget: {}/{} samples differ, \
             peak {:.3e}, rms {:.3e}; first at {i}: got {got:?}, want {want:?}",
            diff.diff_count,
            rendered.len(),
            diff.max_abs,
            diff.rms_err
        );
    }
}

/// Each scenario's new feature must actually change the output: the
/// same render with only the W9 params put back to their defaults has
/// to differ, or the scenario pins nothing about them.
#[test]
fn every_scenario_exercises_its_feature() {
    for s in scenarios() {
        let with = render_scenario(&s);
        let stripped = render_stripped(&s);
        let moved = with.iter().zip(&stripped).any(|(a, b)| (a - b).abs() > 1e-4);
        assert!(moved, "`{}`: the W9 params made no difference", s.name);
    }
}

/// [`render_scenario`], with every W9 param forced back to its default
/// after the scenario's own setup and edits.
fn render_stripped(s: &Scenario) -> Vec<f32> {
    let mut plugin = ResonanceMastering::new();
    (s.setup)(plugin.params());
    strip_w9(plugin.params());
    plugin.initialize(SR, MAX_BLOCK as u32);
    plugin.reset();
    let mut out = Vec::new();
    let mut left = vec![0.0f32; MAX_BLOCK];
    let mut right = vec![0.0f32; MAX_BLOCK];
    let mut n: u64 = 0;
    for block in 0..BLOCKS {
        if let Some(edit) = s.edit {
            edit(plugin.params(), block);
            strip_w9(plugin.params());
        }
        let frames = s.blocks[block % s.blocks.len()];
        for i in 0..frames {
            let (l, r) = s.signal.sample(n + i as u64);
            left[i] = l;
            right[i] = r;
        }
        {
            let mut outs = [OutputBuffer {
                left: &mut left[..frames],
                right: &mut right[..frames],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, frames, &mut ev, None);
        }
        n += frames as u64;
        if block >= PRIME_BLOCKS {
            out.extend_from_slice(&left[..frames]);
            out.extend_from_slice(&right[..frames]);
        }
    }
    out
}

fn strip_w9(p: &MasteringParams) {
    for stage in [&p.corrective_eq, &p.tonal_eq] {
        for b in &stage.bands {
            b.ms.set_value(MsMode::Stereo.to_index());
        }
    }
    for w in &p.imager.band_width {
        w.set_value(1.0);
    }
    p.clipper.on.set_value(false);
    p.saturator.mode.set_value(SatMode::Blend.to_index());
}

/// The reported latency is the same in every W9 configuration as in the
/// default one.
#[test]
fn latency_is_the_same_in_every_configuration() {
    let latency_of = |setup: &dyn Fn(&MasteringParams)| {
        let mut plugin = ResonanceMastering::new();
        setup(plugin.params());
        plugin.initialize(SR, MAX_BLOCK as u32);
        let before = plugin.latency_samples();
        // And while running, through the M/S warm-up and mode changes.
        let mut l = vec![0.1f32; MAX_BLOCK];
        let mut r = vec![-0.1f32; MAX_BLOCK];
        for _ in 0..24 {
            let mut outs = [OutputBuffer {
                left: &mut l,
                right: &mut r,
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, MAX_BLOCK, &mut ev, None);
            assert_eq!(plugin.latency_samples(), before);
        }
        before
    };
    let default = latency_of(&|_| {});
    for s in scenarios() {
        assert_eq!(latency_of(&|p| (s.setup)(p)), default, "scenario `{}`", s.name);
    }
    for mode in 0..SatMode::LABELS.len() as i32 {
        let l = latency_of(&|p| {
            p.saturator.on.set_value(true);
            p.saturator.mode.set_value(mode);
        });
        assert_eq!(l, default, "sat_mode {mode}");
    }
    for ms in [MsMode::Mid, MsMode::Side] {
        let l = latency_of(&|p| {
            for stage in [&p.corrective_eq, &p.tonal_eq] {
                for b in &stage.bands {
                    b.on.set_value(true);
                    b.ms.set_value(ms.to_index());
                }
            }
        });
        assert_eq!(l, default, "every band {ms:?}");
    }
    // At other rates too: the geometry scales, the extensions do not add.
    for sr in [44_100.0f32, 96_000.0] {
        let lat = |setup: &dyn Fn(&MasteringParams)| {
            let mut plugin = ResonanceMastering::new();
            setup(plugin.params());
            plugin.initialize(sr, 512);
            plugin.latency_samples()
        };
        let base = lat(&|_| {});
        let all = lat(&|p| {
            p.clipper.on.set_value(true);
            p.saturator.on.set_value(true);
            p.saturator.mode.set_value(SatMode::Tape.to_index());
            p.imager.on.set_value(true);
            p.imager.band_width[0].set_value(0.0);
            p.corrective_eq.bands[0].on.set_value(true);
            p.corrective_eq.bands[0].ms.set_value(MsMode::Side.to_index());
        });
        assert_eq!(base, all, "at {sr} Hz");
    }
}
