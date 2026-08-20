//! Bit-exact DSP golden for the 8-band parametric EQ (ba todo #1373).
//!
//! `tests/butterworth.rs` checks the cut bands' *magnitude response* at a
//! handful of frequencies and `tests/output_gain.rs` checks the trim.
//! Both stay true under a change of filter topology, of coefficient
//! formula, or of how the cascaded sections are ordered — all of which
//! move the phase, the transient behaviour and often the magnitude
//! between the probe points. This test pins the rendered samples.
//!
//! # The signal, and why it is the revealing one
//!
//! For a filter, the revealing signal is a **unit impulse**: the output
//! *is* the impulse response, so a golden over it pins every biquad
//! coefficient — magnitude and phase, at every frequency at once, not
//! just where a probe happened to look. Two coefficients that produce
//! the same magnitude curve with different phase are indistinguishable
//! to `butterworth.rs` and produce visibly different impulse responses
//! here. The impulses are offset between L and R so the two channel
//! chains cannot be swapped or shared without the golden noticing.
//!
//! A **logarithmic sweep** covers what the impulse cannot: sustained
//! excitation at every frequency, which is what makes the output-gain
//! smoother's per-sample ramp and any denormal/flush behaviour in a long
//! decaying state observable. A **multi-tone** exercises all eight bands
//! simultaneously, at levels where a mis-ordered cascade clips
//! differently.
//!
//! # Tolerance: bit-exact
//!
//! The audio path is a cascade of scalar f32 biquads plus a smoothed
//! gain multiply. The crate's FFT (`analyzer.rs`) feeds the editor's
//! curve only and never touches the output buffer, so the rendered
//! audio has no FFT, no runtime SIMD dispatch, no RNG and no clock in
//! it, and is reproducible. (Bits can only move between machines
//! through libm's rounding inside the coefficient formulas — `sin`,
//! `cos`, `powf` — which is a whole-render ~1e-7 shift, not a localised
//! difference.)
//!
//! To (re)generate the golden after an *intentional* audio change:
//!
//!     RESONANCE_BLESS=1 cargo test -p resonance-eq --test dsp_golden
//!
//! A pure refactor must never need this.

use std::path::PathBuf;

use resonance_eq::params::EqParams;
use resonance_eq::ResonanceEq;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
const MAX_BLOCK: usize = 256;
/// Blocks per scenario. At 256 frames that is 6144 samples of impulse
/// response — long enough for the 48 dB/oct low cut's tail to fall
/// below the f32 floor.
const BLOCKS: usize = 24;

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/dsp_golden.f32")
}

/// `RESONANCE_BLESS=1` is the workspace-wide convention (CLAUDE.md); the
/// narrower name blesses only this file inside a wider run.
fn blessing() -> bool {
    std::env::var("RESONANCE_BLESS").as_deref() == Ok("1")
        || std::env::var("RESONANCE_BLESS_DSP_GOLDEN").as_deref() == Ok("1")
}

const TAU: f32 = std::f32::consts::TAU;

#[derive(Clone, Copy)]
enum Signal {
    /// A unit impulse on L at sample 0 and on R at sample 13, then
    /// silence: the output is the impulse response of each channel's
    /// chain, offset so the channels can't be confused for each other.
    Impulse,
    /// Logarithmic sweep 20 Hz → 20 kHz across the whole render.
    Sweep,
    /// Eight simultaneous tones, one near each band's default centre,
    /// so every section is excited at once.
    MultiTone,
}

impl Signal {
    fn sample(self, n: u64, total: u64) -> (f32, f32) {
        let t = n as f32 / SR;
        match self {
            Signal::Impulse => (
                if n == 0 { 1.0 } else { 0.0 },
                if n == 13 { 1.0 } else { 0.0 },
            ),
            Signal::Sweep => {
                // Exponential chirp: phase is the integral of the
                // instantaneous frequency, in closed form, so the
                // sample at index n never depends on block chopping.
                let f0 = 20.0f32;
                let f1 = 20_000.0f32;
                let dur = total as f32 / SR;
                let k = (f1 / f0).ln() / dur;
                let phase = TAU * f0 * ((k * t).exp() - 1.0) / k;
                let v = 0.4 * phase.sin();
                (v, v * 0.8)
            }
            Signal::MultiTone => {
                let mut v = 0.0f32;
                for f in [40.0, 120.0, 250.0, 600.0, 1500.0, 4000.0, 9000.0, 16000.0] {
                    v += (f * t * TAU).sin();
                }
                let v = 0.09 * v;
                (v, v * 0.8)
            }
        }
    }
}

/// A parameter edit applied *between* blocks, so the next block's
/// `update_from_params` picks it up. `None` for static scenarios.
type MidRunEdit = fn(&EqParams, usize);

struct Scenario {
    name: &'static str,
    signal: Signal,
    blocks: &'static [usize],
    setup: fn(&EqParams),
    edit: Option<MidRunEdit>,
}

/// Enable one band with an explicit kind/freq/gain/Q/slope.
/// Kinds: 0=Bell 1=LowShelf 2=HighShelf 3=LowCut 4=HighCut.
/// Slopes: 0=12 1=24 2=48 dB/oct (cut bands only).
fn band(p: &EqParams, i: usize, kind: i32, freq: f32, gain: f32, q: f32, slope: i32) {
    let b = &p.bands[i];
    b.enabled.set_value(true);
    b.kind.set_value(kind);
    b.freq.set_value(freq);
    b.gain.set_value(gain);
    b.q.set_value(q);
    b.slope.set_value(slope);
}

fn scenarios() -> Vec<Scenario> {
    vec![
        // 1. Every band type at once, driven by an impulse. This one
        //    scenario pins all five filter kinds' coefficients, both
        //    shelves' gain law, three different Qs and the order the
        //    cascade runs in — because reordering a cascade of
        //    non-commuting rounding steps changes the impulse response
        //    even when the ideal transfer function is identical.
        Scenario {
            name: "impulse_all_band_kinds",
            signal: Signal::Impulse,
            blocks: &[256],
            setup: |p| {
                band(p, 0, 3, 45.0, 0.0, 0.707, 1); // low cut, 24 dB/oct
                band(p, 1, 1, 140.0, 6.0, 0.9, 1); // low shelf, boost
                band(p, 2, 0, 320.0, -9.0, 4.5, 1); // narrow bell cut
                band(p, 3, 0, 800.0, 4.5, 0.6, 1); // wide bell boost
                band(p, 4, 0, 2200.0, -3.0, 1.4, 1); // mid bell cut
                band(p, 5, 0, 5000.0, 7.5, 2.2, 1); // presence bell
                band(p, 6, 2, 9500.0, -5.0, 0.8, 1); // high shelf cut
                band(p, 7, 4, 15000.0, 0.0, 0.707, 1); // high cut
                p.output_gain.set_value(-2.5);
            },
            edit: None,
        },
        // 2. The cut bands at all three slopes. `butterworth.rs` checks
        //    the -3 dB point and the asymptote; the impulse response
        //    pins how the cascaded sections are actually built, which is
        //    where a Butterworth Q table gets mis-indexed.
        Scenario {
            name: "impulse_cut_slopes",
            signal: Signal::Impulse,
            blocks: &[256],
            setup: |p| {
                band(p, 0, 3, 120.0, 0.0, 0.707, 0); // 12 dB/oct
                band(p, 1, 3, 240.0, 0.0, 0.707, 1); // 24 dB/oct
                band(p, 2, 3, 480.0, 0.0, 0.707, 2); // 48 dB/oct
                band(p, 5, 4, 3000.0, 0.0, 0.707, 2); // 48 dB/oct high cut
                band(p, 6, 4, 6000.0, 0.0, 0.707, 1);
                band(p, 7, 4, 12000.0, 0.0, 0.707, 0);
            },
            edit: None,
        },
        // 3. Extremes of the Q and gain ranges on an impulse: Q 0.1 and
        //    Q 10, ±24 dB. These are where a coefficient formula loses
        //    precision or a resonant section rings for thousands of
        //    samples, and where a "harmless" reformulation stops being
        //    harmless.
        Scenario {
            name: "impulse_extreme_q_and_gain",
            signal: Signal::Impulse,
            blocks: &[256],
            setup: |p| {
                band(p, 0, 0, 60.0, 24.0, 10.0, 1); // maximum boost, maximum Q
                band(p, 2, 0, 400.0, -24.0, 10.0, 1); // maximum cut, maximum Q
                band(p, 4, 0, 2000.0, 24.0, 0.1, 1); // maximum boost, minimum Q
                band(p, 6, 1, 8000.0, -24.0, 0.1, 1); // shelf at the rails
                p.output_gain.set_value(-24.0);
            },
            edit: None,
        },
        // 4. Sustained excitation across the whole spectrum with a
        //    swept output trim. The sweep is what makes the smoother's
        //    per-sample ramp observable — on an impulse the gain has
        //    nothing left to scale after the first few samples.
        Scenario {
            name: "sweep_with_gain_ramp",
            signal: Signal::Sweep,
            blocks: &[256, 100, 37],
            setup: |p| {
                band(p, 0, 3, 80.0, 0.0, 0.707, 2);
                band(p, 3, 0, 900.0, 8.0, 1.2, 1);
                band(p, 6, 2, 7000.0, 6.0, 0.7, 1);
                p.output_gain.set_value(-24.0);
            },
            edit: Some(|p, block| {
                // Full −24 → +24 dB travel across the run, so the
                // smoother is always mid-ramp at a block boundary.
                let t = block as f32 / BLOCKS as f32;
                p.output_gain.set_value(-24.0 + 48.0 * t);
            }),
        },
        // 5. All eight bands excited simultaneously while their
        //    frequencies, gains, Qs and kinds move between blocks, and
        //    bands switch on and off. This is the scenario that pins
        //    the per-block coefficient refresh and the enabled/bypassed
        //    branch — a band that stops being recomputed sounds fine
        //    until you automate it.
        Scenario {
            name: "multitone_param_sweeps",
            signal: Signal::MultiTone,
            blocks: &[128, 256, 64],
            setup: |p| {
                for i in 0..8 {
                    band(p, i, 0, 100.0 * (i + 1) as f32, 0.0, 1.0, 1);
                }
            },
            edit: Some(|p, block| {
                let t = block as f32 / BLOCKS as f32;
                for i in 0..8 {
                    let b = &p.bands[i];
                    // Each band sweeps over its own octave so they
                    // never collapse onto the same frequency.
                    let base = 60.0 * 1.9f32.powi(i as i32);
                    b.freq.set_value(base * (1.0 + t));
                    b.gain
                        .set_value(if i % 2 == 0 { 12.0 * t } else { -12.0 * t });
                    b.q.set_value(0.3 + 3.0 * t);
                    // Bands 0 and 7 change *kind* mid-run, which is the
                    // path that has to rebuild a section rather than
                    // retune one.
                    if i == 0 {
                        b.kind.set_value(if block % 8 >= 4 { 3 } else { 1 });
                    }
                    if i == 7 {
                        b.kind.set_value(if block % 6 >= 3 { 4 } else { 2 });
                    }
                    // Every band drops out for part of the run.
                    b.enabled.set_value((block + i) % 7 != 0);
                }
                p.output_gain.set_value(-6.0 + 6.0 * t);
            }),
        },
    ]
}

/// Deterministic render of one scenario into the golden sample stream.
fn render_scenario(s: &Scenario) -> Vec<f32> {
    let mut plugin = ResonanceEq::new();
    (s.setup)(&plugin.params);
    plugin.initialize(SR, MAX_BLOCK as u32);
    plugin.reset();

    let total: u64 = (0..BLOCKS)
        .map(|b| s.blocks[b % s.blocks.len()] as u64)
        .sum();

    let mut out = Vec::new();
    let mut left = vec![0.0f32; MAX_BLOCK];
    let mut right = vec![0.0f32; MAX_BLOCK];
    let mut n: u64 = 0;

    for block in 0..BLOCKS {
        if let Some(edit) = s.edit {
            edit(&plugin.params, block);
        }
        let frames = s.blocks[block % s.blocks.len()];
        for i in 0..frames {
            let (l, r) = s.signal.sample(n + i as u64, total);
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
fn eq_output_is_bit_exact() {
    let rendered = render_all();
    assert!(
        rendered.iter().all(|s| s.is_finite()),
        "render produced non-finite samples"
    );

    let path = golden_path();
    if blessing() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes: Vec<u8> = rendered.iter().flat_map(|s| s.to_le_bytes()).collect();
        std::fs::write(&path, bytes).unwrap();
        eprintln!(
            "blessed golden: {} samples -> {}",
            rendered.len(),
            path.display()
        );
        return;
    }

    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {}: {e}\nregenerate with RESONANCE_BLESS=1",
            path.display()
        )
    });
    assert_eq!(
        bytes.len(),
        rendered.len() * 4,
        "golden length mismatch — the scenario set changed"
    );

    let golden = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]));

    let mut diff_count = 0usize;
    let mut max_abs = 0.0f32;
    let mut first_diff = None;
    for (i, (a, b)) in rendered.iter().zip(golden).enumerate() {
        if a.to_bits() != b.to_bits() {
            diff_count += 1;
            max_abs = max_abs.max((a - b).abs());
            if first_diff.is_none() {
                first_diff = Some((i, *a, b));
            }
        }
    }

    if let Some((i, got, want)) = first_diff {
        panic!(
            "EQ DSP output changed: {diff_count}/{} samples differ, peak delta \
             {max_abs:.3e}; first at sample {i} (got {got:?} / {:#010x}, want \
             {want:?} / {:#010x}).\nA refactor of the filter path must be \
             bit-exact. If the change was intended, re-bless with \
             RESONANCE_BLESS=1.\nA peak delta at ~1e-7 spread over most of the \
             render is libm rounding in the coefficient formulas, not a DSP \
             change.",
            rendered.len(),
            got.to_bits(),
            want.to_bits(),
        );
    }
}

/// Guards the scenario table: every scenario must actually filter. The
/// impulse scenarios must produce a *response* — more than a single
/// scaled spike — and the sustained ones must depart from their input.
/// A scenario whose bands were all silently disabled would render a
/// trim and the golden would pin the trim.
#[test]
fn every_scenario_filters() {
    for s in scenarios() {
        let out = render_scenario(&s);
        assert!(
            out.iter().all(|x| x.is_finite()),
            "scenario `{}` rendered non-finite samples",
            s.name
        );
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-4, "scenario `{}` rendered silence", s.name);

        match s.signal {
            Signal::Impulse => {
                // A pass-through would put everything in the first two
                // samples. Require real energy after the first 32.
                let tail: f32 = out
                    .iter()
                    .skip(64)
                    .map(|x| x * x)
                    .sum::<f32>();
                assert!(
                    tail > 1e-6,
                    "scenario `{}` has no impulse-response tail — its bands are \
                     not filtering",
                    s.name
                );
            }
            _ => {
                // Compare against the raw input over the same window:
                // a filtered signal cannot be a scalar multiple of it.
                let total: u64 = (0..BLOCKS)
                    .map(|b| s.blocks[b % s.blocks.len()] as u64)
                    .sum();
                let mut dry = Vec::with_capacity(out.len());
                let mut n = 0u64;
                for block in 0..BLOCKS {
                    let frames = s.blocks[block % s.blocks.len()];
                    for i in 0..frames {
                        dry.push(s.signal.sample(n + i as u64, total).0);
                    }
                    for i in 0..frames {
                        dry.push(s.signal.sample(n + i as u64, total).1);
                    }
                    n += frames as u64;
                }
                // Best-fit scalar between out and dry; a pure trim
                // leaves no residual.
                let dot: f64 = out
                    .iter()
                    .zip(&dry)
                    .map(|(o, d)| (*o as f64) * (*d as f64))
                    .sum();
                let den: f64 = dry.iter().map(|d| (*d as f64) * (*d as f64)).sum();
                let k = dot / den.max(1e-30);
                let resid: f64 = out
                    .iter()
                    .zip(&dry)
                    .map(|(o, d)| {
                        let e = *o as f64 - k * *d as f64;
                        e * e
                    })
                    .sum();
                let energy: f64 = out.iter().map(|o| (*o as f64) * (*o as f64)).sum();
                assert!(
                    resid > 0.05 * energy,
                    "scenario `{}` is within 5% of a plain gain change on its \
                     input — its bands are not filtering",
                    s.name
                );
            }
        }
    }
}
