//! DSP golden for the mastering chain (ba todo #1373).
//!
//! The `stages_*` tests check each stage in isolation and against its
//! own contract — the limiter holds the ceiling, the crossover sums
//! flat, the saturator's shapers are monotonic. None of that pins what
//! the *chain* renders: stage order, the latency-matched dry delay, the
//! input-trim ramp, and the way one stage's output lands on the next
//! one's detector are all invisible to a per-stage test. This test pins
//! the rendered samples.
//!
//! # The signal, and why it is the revealing one
//!
//! The chain ends in a true-peak limiter, so the material has to
//! **actually exceed the ceiling** — a signal that never reaches it
//! would render the limiter as a wire. The driver is therefore a 9 Hz
//! train of drum-shaped transients peaking at +5 dBFS (1.8 linear),
//! roughly 5 dB over the loudest ceiling any scenario sets. Every hit
//! forces real gain reduction, and the gaps between hits let the release
//! run to completion, so both halves of the limiter's envelope are in
//! the golden.
//!
//! Two other signals cover what a transient train cannot:
//!
//! - a **four-band mix** (60 Hz / 400 Hz / 2.5 kHz / 9 kHz tones plus
//!   deterministic noise) so the multiband crossover has energy in
//!   every band and each band compressor has something to work on. A
//!   crossover change that is inaudible on a single tone shifts the
//!   band split here.
//! - a **decorrelated stereo pair** for the imager, whose entire job is
//!   the side channel: on a mono-ish source the width control does
//!   nothing to pin.
//!
//! # Latency, and why only the tail is stored
//!
//! Two linear-phase FIR EQs, the multiband's linear-phase lowpass and
//! the limiter's lookahead put this plugin's latency in the tens of
//! thousands of samples. A short render would capture nothing but the
//! convolvers' pre-fill. So each scenario renders [`PRIME_BLOCKS`]
//! blocks that are *not* stored — the run-up — and only the last
//! [`CAPTURE_BLOCKS`] go into the golden. `latency_is_covered_by_the_prime_window`
//! asserts the run-up really is longer than the chain's reported
//! latency, so this stays true if a stage's latency grows.
//!
//! # Tolerance: an explicit epsilon, not bit equality
//!
//! Unlike the other plugins' DSP goldens, this one **cannot** be
//! bit-exact across machines. The linear-phase EQs and the multiband
//! crossover convolve through `resonance_dsp::FftConvolver`, which uses
//! rustfft's `FftPlanner` — and that planner picks an AVX / SSE / NEON /
//! scalar kernel at *runtime* from CPU feature detection. The same
//! binary on two different CPUs computes the same transform with
//! different (equally correct) rounding. Requiring bit equality here
//! would produce a test that fails on hardware rather than on changes.
//!
//! The bounds below are the f32 FFT error floor with headroom: an
//! 8192-point f32 FFT round trip lands within ~1e-6 relative, which on
//! this material's ~2.0 peak is ~2e-6 absolute. [`MAX_PEAK_DELTA`] sits
//! two decades above that, which leaves room for the limiter — a
//! nonlinear feedback stage — to amplify an upstream ULP near a
//! threshold decision, while staying ~80 dB below anything a real DSP
//! change would move. The test compares bitwise first and prints the
//! measured deltas either way, so a same-machine refactor that moves a
//! single bit is still visible in the log even though it does not fail.
//!
//! To (re)generate the golden after an *intentional* audio change:
//!
//!     RESONANCE_BLESS=1 cargo test -p resonance-mastering --test dsp_golden
//!
//! A pure refactor must never need this.

use std::path::PathBuf;

use resonance_mastering::params::MasteringParams;
use resonance_mastering::ResonanceMastering;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
const MAX_BLOCK: usize = 640;
/// Blocks rendered but *not* stored, so the FIR convolvers, the
/// multiband crossover and the limiter lookahead are all past their
/// pre-fill before the capture starts.
const PRIME_BLOCKS: usize = 56;
/// Blocks stored in the golden, after the prime window.
const CAPTURE_BLOCKS: usize = 8;
const BLOCKS: usize = PRIME_BLOCKS + CAPTURE_BLOCKS;

/// Peak absolute deviation allowed between a render and the golden.
/// See the module header: the floor is the f32 FFT's ~2e-6 on this
/// material, and this is two decades above it.
const MAX_PEAK_DELTA: f32 = 2.0e-4;
/// RMS deviation allowed across the whole render. FFT rounding is
/// zero-mean noise, so its RMS is far below its peak; a real DSP change
/// moves the RMS as much as the peak.
const MAX_RMS_DELTA: f64 = 2.0e-5;

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

/// Deterministic pseudo-noise from the absolute sample index.
fn noise(n: u64) -> f32 {
    let mut s = n.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1) as u32;
    s ^= s >> 16;
    s = s.wrapping_mul(2_246_822_519);
    s ^= s >> 13;
    (s >> 8) as f32 * (2.0 / (1 << 23) as f32) - 1.0
}

#[derive(Clone, Copy)]
enum Signal {
    /// 9 Hz train of drum-shaped hits peaking at +5 dBFS. Every hit is
    /// over every ceiling the scenarios set.
    HotTransients,
    /// Energy in all four multiband bands at once, plus a noise bed.
    BandedMix,
    /// Decorrelated L/R, for the imager's side channel.
    WideStereo,
}

impl Signal {
    fn sample(self, n: u64) -> (f32, f32) {
        let t = n as f32 / SR;
        match self {
            Signal::HotTransients => {
                let x = t % (1.0 / 9.0);
                let env = (-22.0 * x).exp();
                let body = 0.12 * (110.0 * t * TAU).sin();
                let v = 1.8 * env * ((70.0 * t * TAU).sin() + 0.45 * (1600.0 * t * TAU).sin())
                    + body;
                (v, v * 0.86)
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
                // Same tones, opposite phase relationships: a large
                // side component that the imager can widen or collapse.
                let l = 0.4 * (220.0 * t * TAU).sin() + 0.25 * (660.0 * t * TAU).sin()
                    + 0.1 * noise(n);
                let r = 0.4 * (220.0 * t * TAU).sin() - 0.25 * (660.0 * t * TAU).sin()
                    + 0.1 * noise(n + 4_651);
                (l, r)
            }
        }
    }
}

/// A parameter edit applied *between* blocks. `None` for static
/// scenarios.
type MidRunEdit = fn(&MasteringParams, usize);

struct Scenario {
    name: &'static str,
    signal: Signal,
    blocks: &'static [usize],
    setup: fn(&MasteringParams),
    edit: Option<MidRunEdit>,
}

fn scenarios() -> Vec<Scenario> {
    vec![
        // 1. Factory defaults: every stage off. Not a no-op — the two
        //    linear-phase EQs still convolve, with a flat FIR, and the
        //    trim still ramps. This scenario pins that the "off" chain
        //    really is transparent apart from its latency; a designer
        //    bug that leaves a tilt in the flat FIR shows up here and
        //    nowhere else.
        Scenario {
            name: "defaults_all_stages_off",
            signal: Signal::HotTransients,
            blocks: &[512],
            setup: |_| {},
            edit: None,
        },
        // 2. Whole chain engaged at once, on material 5 dB over the
        //    ceiling. This is the scenario that pins stage *order*: the
        //    saturator sees the glue compressor's output, the multiband
        //    sees the tonal EQ's, the limiter sees the imager's. Swap
        //    any two and the samples move even though each stage still
        //    passes its own test.
        Scenario {
            name: "full_chain_over_ceiling",
            signal: Signal::HotTransients,
            blocks: &[512, 384, 640],
            setup: |p| {
                // Hot into the chain on purpose: the glue compressor
                // and the multiband both pull the level down, and
                // without this trim the signal reaches the limiter
                // already under the ceiling — which would render the
                // limiter as a wire in the one scenario meant to cover
                // the whole chain. `every_scenario_that_limits_reduces_gain`
                // asserts it really is limiting.
                p.input_trim_db.set_value(6.0);
                // Corrective EQ: a high-pass-ish cut and a narrow notch.
                eq_band(&p.corrective_eq.bands[0], 3, 35.0, 0.707, 0.0);
                eq_band(&p.corrective_eq.bands[1], 0, 320.0, 6.0, -4.5);
                // Glue: gentle bus compression.
                p.glue_compressor.on.set_value(true);
                p.glue_compressor.threshold.set_value(-18.0);
                p.glue_compressor.ratio.set_value(3.0);
                p.glue_compressor.attack.set_value(20.0);
                p.glue_compressor.release.set_value(200.0);
                p.glue_compressor.knee.set_value(6.0);
                p.glue_compressor.makeup.set_value(2.0);
                p.glue_compressor.mix.set_value(0.8);
                // Saturator.
                p.saturator.on.set_value(true);
                p.saturator.drive.set_value(0.6);
                p.saturator.character.set_value(0.7);
                p.saturator.mix.set_value(0.5);
                // Tonal EQ: a shelf pair.
                eq_band(&p.tonal_eq.bands[0], 1, 120.0, 0.8, 3.0);
                eq_band(&p.tonal_eq.bands[3], 2, 8000.0, 0.8, 2.5);
                // Multiband on all four bands.
                p.multiband.on.set_value(true);
                for (i, b) in p.multiband.bands.iter().enumerate() {
                    b.on.set_value(true);
                    b.threshold.set_value(-24.0 + 3.0 * i as f32);
                    b.ratio.set_value(2.5);
                    b.attack.set_value(10.0);
                    b.release.set_value(150.0);
                    b.knee.set_value(6.0);
                    b.mix.set_value(1.0);
                    b.gain.set_value(1.0);
                }
                // Imager, limiter, dither.
                p.imager.on.set_value(true);
                p.imager.width.set_value(1.3);
                p.limiter.on.set_value(true);
                p.limiter.ceiling.set_value(-0.3);
                p.limiter.release.set_value(50.0);
                p.dither.on.set_value(true);
                p.dither.target_bits.set_value(16);
                p.dither.noise_shape.set_value(true);
            },
            edit: None,
        },
        // 3. Limiter alone, hard against a −3 dBTP ceiling. Isolating it
        //    means every sample difference is the limiter's lookahead,
        //    its gain-computation and its release — nothing upstream can
        //    mask a change to them.
        Scenario {
            name: "limiter_only_hard_ceiling",
            signal: Signal::HotTransients,
            blocks: &[512, 256],
            setup: |p| {
                p.limiter.on.set_value(true);
                p.limiter.ceiling.set_value(-3.0);
                p.limiter.release.set_value(5.0);
            },
            edit: None,
        },
        // 4. Multiband alone on four-band material, with the crossovers
        //    moved off their defaults. The crossover is linear-phase FIR
        //    work; where the splits sit decides which compressor sees
        //    which tone.
        Scenario {
            name: "multiband_crossovers",
            signal: Signal::BandedMix,
            blocks: &[512, 640],
            setup: |p| {
                p.multiband.on.set_value(true);
                p.multiband.xo1.set_value(150.0);
                p.multiband.xo2.set_value(1200.0);
                p.multiband.xo3.set_value(6000.0);
                for (i, b) in p.multiband.bands.iter().enumerate() {
                    b.on.set_value(true);
                    b.threshold.set_value(-30.0);
                    b.ratio.set_value(4.0);
                    b.attack.set_value(2.0 + 8.0 * i as f32);
                    b.release.set_value(80.0 + 60.0 * i as f32);
                    b.knee.set_value(3.0);
                    b.mix.set_value(0.9);
                    // Per-band make-up spread, so a band that stops
                    // being routed to its own compressor is obvious.
                    b.gain.set_value(0.7 + 0.2 * i as f32);
                }
            },
            edit: None,
        },
        // 5. Saturator with its shaper stepped between blocks, plus the
        //    imager on decorrelated material. Both are pure waveshaping
        //    / matrixing with no memory, so the golden is the only
        //    place their exact curves are recorded.
        Scenario {
            name: "saturator_shapers_and_imager",
            signal: Signal::WideStereo,
            blocks: &[384, 512],
            setup: |p| {
                p.saturator.on.set_value(true);
                p.saturator.drive.set_value(0.9);
                p.saturator.character.set_value(1.0);
                p.saturator.mix.set_value(1.0);
                p.imager.on.set_value(true);
                p.imager.width.set_value(1.6);
                p.imager.side_hpf_on.set_value(true);
                p.imager.side_hpf_freq.set_value(180.0);
            },
            edit: Some(|p, block| {
                // Cycle every shaper; the range is 0..=max, and
                // `IntParam::set_value` clamps, so walking past the end
                // is safe and still covers all of them.
                p.saturator.shaper.set_value((block % 4) as i32);
                let t = block as f32 / BLOCKS as f32;
                p.imager.width.set_value(0.2 + 1.6 * t);
            }),
        },
        // 6. Whole-plugin bypass. The bypassed path is *not* a wire: it
        //    is the raw input delayed by exactly the chain's latency so
        //    A/B stays aligned. Off-by-one there is inaudible in
        //    isolation and obvious against a golden.
        Scenario {
            name: "bypass_latency_aligned",
            signal: Signal::HotTransients,
            blocks: &[512, 384],
            setup: |p| {
                p.bypass.set_value(true);
                // Set stages up anyway, so a bypass that leaks into the
                // processed path would be unmistakable.
                p.limiter.on.set_value(true);
                p.limiter.ceiling.set_value(-6.0);
                p.saturator.on.set_value(true);
                p.saturator.drive.set_value(1.0);
            },
            edit: None,
        },
        // 7. Everything swept between blocks, including the bypass and
        //    the stage on/off switches, so the chain crosses every
        //    engage/disengage boundary within one run.
        Scenario {
            name: "param_sweeps_between_blocks",
            signal: Signal::HotTransients,
            blocks: &[256, 640, 384],
            setup: |p| {
                p.limiter.on.set_value(true);
                p.glue_compressor.on.set_value(true);
            },
            edit: Some(|p, block| {
                let t = block as f32 / BLOCKS as f32;
                p.input_trim_db.set_value(-6.0 + 12.0 * t);
                p.limiter.ceiling.set_value(-6.0 + 6.0 * t);
                p.limiter.release.set_value(5.0 + 400.0 * t);
                p.glue_compressor.threshold.set_value(-36.0 + 30.0 * t);
                p.glue_compressor.ratio.set_value(1.5 + 8.0 * t);
                p.glue_compressor.mix.set_value(0.3 + 0.7 * t);
                // Stages switch on and off at different periods.
                p.saturator.on.set_value(block % 9 >= 4);
                p.saturator.drive.set_value(t);
                p.multiband.on.set_value(block % 13 >= 6);
                p.imager.on.set_value(block % 7 >= 3);
                p.dither.on.set_value(block % 11 >= 5);
                // A moving EQ band forces a FIR redesign mid-run, which
                // is the expensive path the cache normally avoids.
                eq_band(
                    &p.tonal_eq.bands[2],
                    0,
                    500.0 + 3000.0 * t,
                    1.2,
                    -8.0 + 16.0 * t,
                );
            }),
        },
    ]
}

/// Enable one linear-phase EQ band. Types: 0=Bell 1=LowShelf
/// 2=HighShelf 3=LowCut 4=HighCut.
fn eq_band(
    b: &resonance_mastering::params::eq_stage::BandParams,
    band_type: i32,
    freq: f32,
    q: f32,
    gain_db: f32,
) {
    b.on.set_value(true);
    b.band_type.set_value(band_type);
    b.freq.set_value(freq);
    b.q.set_value(q);
    b.gain.set_value(gain_db);
}

/// Deterministic render of one scenario. Only the blocks after the
/// prime window are stored; see the module header.
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

fn render_all() -> Vec<f32> {
    let mut all = Vec::new();
    for s in scenarios() {
        all.extend(render_scenario(&s));
    }
    all
}

#[test]
fn mastering_output_matches_golden() {
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

    let golden: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let mut diff_count = 0usize;
    let mut max_abs = 0.0f32;
    let mut first_diff = None;
    let mut sq_err = 0.0f64;
    for (i, (a, b)) in rendered.iter().zip(golden.iter()).enumerate() {
        let d = a - b;
        sq_err += (d as f64) * (d as f64);
        if a.to_bits() != b.to_bits() {
            diff_count += 1;
            if d.abs() > max_abs {
                max_abs = d.abs();
            }
            if first_diff.is_none() {
                first_diff = Some((i, *a, *b));
            }
        }
    }
    let rms_err = (sq_err / rendered.len() as f64).sqrt();

    eprintln!(
        "mastering golden: {diff_count}/{} samples differ | peak delta \
         {max_abs:.3e} (limit {MAX_PEAK_DELTA:.1e}) | rms delta {rms_err:.3e} \
         (limit {MAX_RMS_DELTA:.1e})",
        rendered.len()
    );

    if let Some((i, got, want)) = first_diff {
        assert!(
            max_abs <= MAX_PEAK_DELTA && rms_err <= MAX_RMS_DELTA,
            "mastering chain output moved beyond the FFT-rounding budget: \
             {diff_count}/{} samples differ, peak delta {max_abs:.3e} (limit \
             {MAX_PEAK_DELTA:.1e}), rms delta {rms_err:.3e} (limit \
             {MAX_RMS_DELTA:.1e}); first at sample {i}: got {got:?}, want \
             {want:?}.\nThis is a real change to the chain, not CPU-dependent \
             FFT rounding. If it was intended, re-bless with RESONANCE_BLESS=1.",
            rendered.len()
        );
    }
}

/// The prime window has to be longer than the chain's own latency, or
/// the golden would be storing the convolvers' pre-fill instead of
/// audio. Asserted rather than assumed, so a stage that grows its
/// latency fails here with a clear reason instead of silently turning
/// the golden into a pin over zeros.
#[test]
fn latency_is_covered_by_the_prime_window() {
    for s in scenarios() {
        let mut plugin = ResonanceMastering::new();
        (s.setup)(plugin.params());
        plugin.initialize(SR, MAX_BLOCK as u32);
        let latency = plugin.latency_samples() as usize;
        let primed: usize = (0..PRIME_BLOCKS)
            .map(|b| s.blocks[b % s.blocks.len()])
            .sum();
        assert!(
            primed > latency + 2 * MAX_BLOCK,
            "scenario `{}`: prime window is {primed} frames but the chain reports \
             {latency} samples of latency — raise PRIME_BLOCKS",
            s.name
        );
    }
}

/// Guards the scenario table: every scenario must render audio, and
/// every scenario that engages the limiter must actually be *limited* —
/// the limiter has to report real gain reduction inside the captured
/// window, not merely pass a signal that happens to sit under the
/// ceiling. That distinction matters: a chain whose upstream stages
/// already tame the level renders the limiter as a wire, and a golden
/// over a wire pins nothing about it. (This is not hypothetical — the
/// full-chain scenario needed its input trim raised for exactly this
/// reason.)
#[test]
fn every_scenario_renders_and_limits() {
    for s in scenarios() {
        let params = MasteringParams::default();
        (s.setup)(&params);
        let limiting = params.limiter.on.value() && !params.bypass.value();

        let (out, max_gr_db) = render_scenario_with_gr(&s);
        assert!(
            out.iter().all(|x| x.is_finite()),
            "scenario `{}` rendered non-finite samples",
            s.name
        );
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-3, "scenario `{}` rendered silence", s.name);

        if !limiting {
            continue;
        }
        let ceiling = 10.0f32.powf(params.limiter.ceiling.value() / 20.0);
        // A little headroom: the ceiling is true-peak, measured on an
        // upsampled signal, so the sample-domain peak sits at or just
        // under it rather than exactly on it.
        assert!(
            peak <= ceiling * 1.05,
            "scenario `{}` peaked at {peak:.4} against a ceiling of {ceiling:.4} \
             — the limiter is not holding, so the golden is pinning an \
             unlimited signal",
            s.name
        );
        assert!(
            max_gr_db > 1.0,
            "scenario `{}` reported only {max_gr_db:.2} dB of limiter gain \
             reduction in the captured window — the material never reaches the \
             ceiling there, so the limiter contributes nothing to the golden",
            s.name
        );
    }
}

/// [`render_scenario`], plus the peak limiter gain reduction the chain
/// published across the captured blocks. Only the guard test needs it,
/// so it stays out of the render path the golden uses.
fn render_scenario_with_gr(s: &Scenario) -> (Vec<f32>, f32) {
    let mut plugin = ResonanceMastering::new();
    (s.setup)(plugin.params());
    plugin.initialize(SR, MAX_BLOCK as u32);
    plugin.reset();

    let mut out = Vec::new();
    let mut left = vec![0.0f32; MAX_BLOCK];
    let mut right = vec![0.0f32; MAX_BLOCK];
    let mut n: u64 = 0;
    let mut max_gr = 0.0f32;

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
            max_gr = max_gr.max(plugin.viz().limiter_gr_db());
            out.extend_from_slice(&left[..frames]);
            out.extend_from_slice(&right[..frames]);
        }
    }
    (out, max_gr)
}
