//! Bit-exact DSP golden for the compressor (ba todo #1373).
//!
//! The rest of this crate's suite pins *properties*: the transfer curve
//! is monotonic, a key really replaces the detector, smoothers land on
//! their target. None of that notices if the attack coefficient, the
//! knee interpolation or the RMS detector's averaging window changes —
//! the plugin would compress differently and every assertion would still
//! hold. This test pins the rendered samples.
//!
//! # The signal, and why it is the revealing one
//!
//! A compressor is a *level-dependent* device, so the material has to
//! move in level, fast and slow, above and below the threshold:
//!
//! - **Transients** — a 12 Hz train of percussive hits peaking at
//!   −3 dBFS over a sustained −24 dBFS body. A hit is 30 dB over the
//!   threshold and decays through it, which is the only thing that
//!   separates a peak detector from an RMS one (they disagree by ~10 dB
//!   on a spiky signal and agree on a steady one) and the only thing
//!   that makes the attack coefficient audible at all. A steady tone
//!   would pin nothing but the static curve.
//! - **Steps** — a tone that jumps between −30 and −6 dBFS every
//!   80 ms. Each edge is a clean exponential attack and release with
//!   nothing else happening, so the ballistics are pinned in isolation.
//! - **Low-heavy** — a −6 dBFS 45 Hz sine under a −18 dBFS 1 kHz tone.
//!   This is the signal the sidechain HPF exists for: with the filter
//!   off the bass pumps the whole band, with it on it does not.
//! - **Kick key** — an external key carrying material that is nowhere
//!   in the main buffer, so the ducking scenario cannot be matched by a
//!   build that silently falls back to the internal detector.
//!
//! # Tolerance: bit-exact
//!
//! The audio path is scalar f32 throughout — log/exp level conversion,
//! one-pole ballistics, a biquad sidechain HPF, a gain multiply and a
//! dry/wet blend. No FFT, no runtime SIMD dispatch, no RNG, no clock, so
//! the render is reproducible and any deviation is a real change. (Bits
//! can move between machines only through libm's rounding of
//! `log10`/`exp`/`sin`; that appears as most of the render differing at
//! the ~1e-7 level, never as a few samples differing visibly.)
//!
//! To (re)generate the golden after an *intentional* audio change:
//!
//!     RESONANCE_BLESS=1 cargo test -p resonance-compressor --test dsp_golden
//!
//! A pure refactor must never need this.

use std::path::PathBuf;

use resonance_compressor::params::CompressorParams;
use resonance_compressor::ResonanceCompressor;
use resonance_dsp_test_support as golden;
use resonance_plugin::{EventIterator, KeyBuffer, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
/// The activation buffer size; individual blocks may be shorter.
const MAX_BLOCK: usize = 320;
/// Blocks rendered per scenario — ~130 ms, several transient periods
/// and a couple of level steps, which is enough to show every stage of
/// the envelope without bloating the fixture.
const BLOCKS: usize = 24;

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "dsp_golden.f32")
}

/// `RESONANCE_BLESS=1` is the workspace-wide convention (CLAUDE.md); the
/// narrower name blesses only this file inside a wider run.
fn blessing() -> bool {
    golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_DSP_GOLDEN"])
}

const TAU: f32 = std::f32::consts::TAU;

#[derive(Clone, Copy)]
enum Signal {
    /// Percussive hits well over the threshold, decaying through it.
    Transients,
    /// Square level steps between −30 and −6 dBFS.
    Steps,
    /// Loud low sine under a quieter midrange tone.
    LowHeavy,
    /// A sustained pad that never crosses anything on its own.
    Pad,
}

impl Signal {
    fn sample(self, n: u64) -> (f32, f32) {
        let t = n as f32 / SR;
        match self {
            Signal::Transients => {
                let x = t % (1.0 / 12.0);
                // Fast attack, ~40 ms decay: a drum-shaped envelope.
                let env = (-25.0 * x).exp();
                let hit = 0.7 * env * ((180.0 * t * TAU).sin() + 0.5 * (940.0 * t * TAU).sin());
                let body = 0.063 * (220.0 * t * TAU).sin();
                let v = hit + body;
                // The right channel is 2 dB down so a mono-summed
                // detector and a max-of-both detector differ in the
                // golden.
                (v, v * 0.79)
            }
            Signal::Steps => {
                let loud = (t / 0.08) as u32 % 2 == 0;
                let amp = if loud { 0.5 } else { 0.0316 };
                let v = amp * (330.0 * t * TAU).sin();
                (v, v * 0.79)
            }
            Signal::LowHeavy => {
                let v = 0.5 * (45.0 * t * TAU).sin() + 0.126 * (1000.0 * t * TAU).sin();
                (v, v * 0.79)
            }
            Signal::Pad => {
                let v = 0.2 * ((196.0 * t * TAU).sin() + 0.6 * (294.0 * t * TAU).sin());
                (v, v * 0.9)
            }
        }
    }
}

/// A kick-shaped external key: a 10 Hz train of 55 Hz bursts.
fn key_sample(n: u64) -> f32 {
    let t = n as f32 / SR;
    let x = t % 0.1;
    let env = (-60.0 * x).exp();
    0.8 * env * (55.0 * t * TAU).sin()
}

/// A parameter edit applied *between* blocks, so the next block picks
/// it up. `None` for static scenarios.
type MidRunEdit = fn(&CompressorParams, usize);

struct Scenario {
    name: &'static str,
    signal: Signal,
    /// Host block sizes, cycled across the run.
    blocks: &'static [usize],
    key: bool,
    setup: fn(&CompressorParams),
    edit: Option<MidRunEdit>,
}

fn scenarios() -> Vec<Scenario> {
    vec![
        // 1. Factory defaults on transients: −18 dB threshold, 4:1,
        //    10 ms attack, 120 ms release, 6 dB knee, 30% RMS detector.
        //    The sound a user hears before touching anything.
        Scenario {
            name: "factory_defaults_transients",
            signal: Signal::Transients,
            blocks: &[256],
            key: false,
            setup: |_| {},
            edit: None,
        },
        // 2. Limiting: near-infinite ratio, fastest attack, fastest
        //    release, hard knee, pure peak detector. Every ballistic
        //    constant is at an extreme, so a coefficient change shows up
        //    at full scale rather than being smoothed away.
        Scenario {
            name: "peak_limiting_hard_knee",
            signal: Signal::Transients,
            blocks: &[64, 197, 320],
            key: false,
            setup: |p| {
                p.threshold.set_value(-24.0);
                p.ratio.set_value(20.0);
                p.attack.set_value(0.1);
                p.release.set_value(5.0);
                p.knee.set_value(0.0);
                p.detector_mix.set_value(0.0);
            },
            edit: None,
        },
        // 3. The opposite corner on clean level steps: widest knee,
        //    lowest ratio, slowest ballistics, pure RMS detector. The
        //    steps make the attack and release curves readable as
        //    curves; the wide knee means the *whole* transfer function
        //    is in play, not just its two straight segments.
        Scenario {
            name: "soft_knee_rms_steps",
            signal: Signal::Steps,
            blocks: &[128, 96],
            key: false,
            setup: |p| {
                p.threshold.set_value(-20.0);
                p.ratio.set_value(2.0);
                p.attack.set_value(60.0);
                p.release.set_value(600.0);
                p.knee.set_value(12.0);
                p.detector_mix.set_value(1.0);
            },
            edit: None,
        },
        // 4. Sidechain HPF on low-heavy material. With the filter off
        //    the 45 Hz sine drives the detector and the whole signal
        //    pumps; with it on at 500 Hz it barely does. Nothing else
        //    in the suite renders that difference.
        Scenario {
            name: "sc_hpf_on_low_heavy",
            signal: Signal::LowHeavy,
            blocks: &[256, 160],
            key: false,
            setup: |p| {
                p.threshold.set_value(-24.0);
                p.ratio.set_value(8.0);
                p.attack.set_value(3.0);
                p.release.set_value(80.0);
                p.sc_hpf_on.set_value(true);
                p.sc_hpf_freq.set_value(500.0);
                p.detector_mix.set_value(0.5);
            },
            edit: None,
        },
        // 5. Auto-makeup with parallel mix: the makeup gain is derived
        //    from threshold and ratio rather than read off a slider, and
        //    the output is a blend of compressed and dry. Both are pure
        //    arithmetic that no behavioural assertion pins.
        Scenario {
            name: "auto_makeup_parallel",
            signal: Signal::Transients,
            blocks: &[192, 320],
            key: false,
            setup: |p| {
                p.threshold.set_value(-30.0);
                p.ratio.set_value(6.0);
                p.attack.set_value(1.0);
                p.release.set_value(150.0);
                p.knee.set_value(9.0);
                p.auto_makeup.set_value(true);
                p.mix.set_value(0.35);
            },
            edit: None,
        },
        // 6. External key: a sustained pad ducked by a kick that exists
        //    only on the key port. Every level move in the output comes
        //    from material the compressor never outputs.
        Scenario {
            name: "external_key_ducks_pad",
            signal: Signal::Pad,
            blocks: &[256, 320, 101],
            key: true,
            setup: |p| {
                p.threshold.set_value(-30.0);
                p.ratio.set_value(10.0);
                p.attack.set_value(0.5);
                p.release.set_value(120.0);
                p.knee.set_value(3.0);
                p.detector_mix.set_value(0.2);
            },
            edit: None,
        },
        // 7. Every control swept between blocks, so each block re-reads
        //    the params and recomputes its coefficients mid-envelope.
        //    Also toggles the two booleans, which is the only way to
        //    reach the HPF's engage/disengage and the auto-makeup
        //    handover in one run.
        Scenario {
            name: "param_sweeps_between_blocks",
            signal: Signal::Transients,
            blocks: &[128, 320, 64],
            key: false,
            setup: |p| {
                p.threshold.set_value(-24.0);
            },
            edit: Some(|p, block| {
                let t = block as f32 / BLOCKS as f32;
                p.threshold.set_value(-48.0 + 40.0 * t);
                p.ratio.set_value(1.5 + 16.0 * t);
                p.attack.set_value(0.1 + 40.0 * t);
                p.release.set_value(10.0 + 600.0 * t);
                p.knee.set_value(12.0 * (1.0 - t));
                p.makeup.set_value(-6.0 + 18.0 * t);
                p.mix.set_value(0.2 + 0.8 * t);
                p.detector_mix.set_value(t);
                p.sc_hpf_freq.set_value(20.0 + 400.0 * t);
                p.sc_hpf_on.set_value(block % 10 >= 5);
                p.auto_makeup.set_value(block % 14 >= 7);
            }),
        },
    ]
}

/// Deterministic render of one scenario into the golden sample stream.
fn render_scenario(s: &Scenario) -> Vec<f32> {
    let mut plugin = ResonanceCompressor::new();
    (s.setup)(&plugin.params);
    plugin.initialize(SR, MAX_BLOCK as u32);
    plugin.reset();

    let mut out = Vec::new();
    let mut left = vec![0.0f32; MAX_BLOCK];
    let mut right = vec![0.0f32; MAX_BLOCK];
    let mut key_l = vec![0.0f32; MAX_BLOCK];
    let mut key_r = vec![0.0f32; MAX_BLOCK];
    let mut n: u64 = 0;

    for block in 0..BLOCKS {
        if let Some(edit) = s.edit {
            edit(&plugin.params, block);
        }
        let frames = s.blocks[block % s.blocks.len()];
        for i in 0..frames {
            let (l, r) = s.signal.sample(n + i as u64);
            left[i] = l;
            right[i] = r;
            if s.key {
                let k = key_sample(n + i as u64);
                key_l[i] = k;
                key_r[i] = k * 0.7;
            }
        }
        {
            let mut outs = [OutputBuffer {
                left: &mut left[..frames],
                right: &mut right[..frames],
            }];
            let mut ev = EventIterator::empty();
            let key = s.key.then(|| KeyBuffer {
                left: &key_l[..frames],
                right: &key_r[..frames],
            });
            plugin.process_with_key(&mut outs, key, frames, &mut ev, None);
        }
        n += frames as u64;
        out.extend_from_slice(&left[..frames]);
        out.extend_from_slice(&right[..frames]);
    }
    out
}

fn render_all() -> Vec<f32> {
    let mut all = Vec::new();
    for s in scenarios() {
        all.extend(render_scenario(&s));
    }
    all
}

#[test]
fn compressor_output_is_bit_exact() {
    let rendered = render_all();
    assert!(
        rendered.iter().all(|s| s.is_finite()),
        "render produced non-finite samples"
    );

    let path = golden_path();
    if blessing() {
        golden::bless_f32(&path, &rendered);
        return;
    }

    let want = golden::load_golden_f32(&path, rendered.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_f32(&rendered, &want);

    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "compressor DSP output changed: {}/{} samples differ, peak \
             delta {:.3e}; first at sample {i} (got {got:?} / {:#010x}, want \
             {want:?} / {:#010x}).\nA refactor of the compressor path must be \
             bit-exact. If the change was intended, re-bless with \
             RESONANCE_BLESS=1.\nA peak delta at ~1e-7 spread over most of the \
             render is libm rounding, not a DSP change.",
            diff.diff_count,
            rendered.len(),
            diff.max_abs,
            got.to_bits(),
            want.to_bits(),
        );
    }
}

/// Guards the scenario table: every scenario's gain must actually
/// *depend on level*. Crest factor is the wrong measure here — a fast
/// release pumps the quiet parts back up and can widen it — so this
/// compares the plugin's output against the same input rendered dry,
/// window by window, and requires the implied gain to move. A constant
/// implied gain is a bypass (or a trim), and a golden over a bypass
/// pins nothing.
#[test]
fn every_scenario_applies_level_dependent_gain() {
    const WINDOW: usize = 512;

    for s in scenarios() {
        let out = render_scenario(&s);
        let dry = render_dry(&s);
        assert_eq!(out.len(), dry.len());
        assert!(
            out.iter().all(|x| x.is_finite()),
            "scenario `{}` rendered non-finite samples",
            s.name
        );
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-3, "scenario `{}` rendered silence", s.name);

        // Settled second half only: the first half includes the
        // detector warming up from its reset state.
        let half = (out.len() / 2) & !1;
        let mut gains: Vec<f32> = Vec::new();
        for (o, d) in out[half..].chunks(WINDOW).zip(dry[half..].chunks(WINDOW)) {
            let dr = rms(d);
            if dr > 1e-4 {
                gains.push(rms(o) / dr);
            }
        }
        assert!(
            gains.len() >= 4,
            "scenario `{}` produced too few measurable windows",
            s.name
        );

        // Either the gain *moves* (classic compression on a dynamic
        // source) or it sits well under unity (steady-state levelling,
        // which is what a fast source under a slow release actually
        // does). Requiring both would fail honest settings; requiring
        // neither would let a bypass through.
        let hi = gains.iter().copied().fold(0.0f32, f32::max);
        let lo = gains.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(
            hi / lo > 1.25 || lo < 0.9,
            "scenario `{}` neither moved its gain nor reduced it ({lo:.3}..{hi:.3}) \
             — it is rendering a bypass, and a golden over a bypass pins nothing",
            s.name
        );
    }
}

/// The same input stream a scenario feeds the plugin, untouched. Only
/// used by the guard test above.
fn render_dry(s: &Scenario) -> Vec<f32> {
    let mut out = Vec::new();
    let mut n: u64 = 0;
    for block in 0..BLOCKS {
        let frames = s.blocks[block % s.blocks.len()];
        let mut left = Vec::with_capacity(frames);
        let mut right = Vec::with_capacity(frames);
        for i in 0..frames {
            let (l, r) = s.signal.sample(n + i as u64);
            left.push(l);
            right.push(r);
        }
        n += frames as u64;
        out.extend_from_slice(&left);
        out.extend_from_slice(&right);
    }
    out
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}
