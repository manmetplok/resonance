//! DSP golden for the IR convolution engine (ba todo #1373).
//!
//! `tests/dsp_block.rs` proves `IrEngine::process_block` matches the
//! per-sample loop it replaced, and `tests/latency_mode.rs` checks the
//! block-size arithmetic. Neither pins what the plugin *sounds* like:
//! the convolution itself, the wet/dry blend against the latency-aligned
//! dry path, the output-gain ramp and the swap crossfade all sit behind
//! a reference implementation or a scalar assertion. This test pins the
//! rendered samples.
//!
//! # Where the impulse response comes from
//!
//! A convolver with nothing loaded is a wire, so a golden needs an IR —
//! and this crate ships no impulse file. Rather than depend on one on
//! disk (which would make the test skip on every machine but one), the
//! IR is **synthesised in the test** by a documented, deterministic
//! recipe: a guitar-cabinet-shaped response built from a short direct
//! spike, two resonant modes and an exponentially decaying, low-passed
//! pseudo-noise tail, with the left and right sides given different
//! resonances so a channel swap cannot go unnoticed. See
//! [`cab_impulse_response`].
//!
//! That IR is then **encoded as a 16-bit stereo 44.1 kHz WAV in memory
//! and decoded back through `ir_loader::load_ir_from_bytes` at
//! 48 kHz**, so the golden also covers the real file path this plugin
//! uses: the WAV decode, the channel de-interleave and the sample-rate
//! conversion. Only the parts that genuinely need the filesystem are
//! left out — see "What this does not cover" below.
//!
//! # The signal, and why it is the revealing one
//!
//! Two drivers:
//!
//! - a **unit impulse** (L at sample 0, R at sample 29). Convolving an
//!   impulse with the IR returns the IR, so this scenario's golden *is*
//!   the loaded impulse response, sample for sample, after the
//!   convolver's partitioning and overlap-add. Any error in a partition
//!   boundary, an FFT size or the overlap accumulation shows up as a
//!   glitch at a known offset. Nothing else pins a convolver so
//!   completely.
//! - a **DI-style pluck train**, because an impulse leaves the wet/dry
//!   blend and the output-gain ramp nothing to scale after the first few
//!   samples. Sustained, transient-rich material makes the dry
//!   alignment audible: if the bypass delay is off by one, the blend
//!   comb-filters.
//!
//! # Tolerance: an explicit epsilon, not bit equality
//!
//! The convolver is `resonance_dsp::FftConvolver`, built on rustfft's
//! `FftPlanner`, which selects an AVX / SSE / NEON / scalar kernel at
//! *runtime* from CPU feature detection. Two machines compute the same
//! transform with different (equally correct) rounding, so bit equality
//! would be a test of the hardware. The bounds below are the f32 FFT
//! error floor with headroom: an FFT round trip at these sizes lands
//! within ~1e-6 relative, and the signal peaks near 1.0, so
//! [`MAX_PEAK_DELTA`] sits two decades above the floor and ~74 dB below
//! anything a real change to the engine would move. The test compares
//! bitwise first and reports the measured deltas, so a same-machine
//! refactor that moves a single bit is still visible in the log.
//!
//! # What this does not cover
//!
//! - `loader.rs`'s background thread and its mailbox handoff into
//!   `ResonanceIr::process`. Reaching it needs a real IR file on disk
//!   and a plugin instance that scans a directory; the engine-level
//!   swap path it feeds *is* covered here, via `begin_swap`.
//! - `ir_loader::load_ir` itself (the `std::fs::read` wrapper). Its
//!   entire body below the read — `load_ir_from_bytes` — is covered.
//!
//! To (re)generate the golden after an *intentional* audio change:
//!
//!     RESONANCE_BLESS=1 cargo test -p resonance-ir --test dsp_golden
//!
//! A pure refactor must never need this.

use std::path::PathBuf;

use resonance_ir::dsp::{block_size_for, IrEngine, LatencyMode, StereoConvolver};
use resonance_ir::ir_loader::load_ir_from_bytes;
use resonance_ir::params::{IrParams, IrSmoothers};

const SR: f32 = 48_000.0;
/// The IR fixture is authored at 44.1 kHz so the decode path has to
/// resample it to reach `SR` — the same conversion a real cab IR goes
/// through.
const IR_SR: f32 = 44_100.0;
/// Length of the synthesised IR at its authoring rate: ~23 ms, a
/// realistic guitar-cabinet capture.
const IR_FRAMES: usize = 1024;

const MAX_BLOCK: usize = 320;
/// Blocks per scenario — long enough that the whole (resampled) IR has
/// rung out several times over.
const BLOCKS: usize = 24;

/// Peak absolute deviation allowed between a render and the golden.
/// See the module header.
const MAX_PEAK_DELTA: f32 = 2.0e-4;
/// RMS deviation allowed across the render. FFT rounding is zero-mean,
/// so its RMS sits far below its peak.
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

/// Deterministic pseudo-noise from an index — no RNG, no seeding
/// question, identical on every machine.
fn noise(n: u64) -> f32 {
    let mut s = n.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1) as u32;
    s ^= s >> 16;
    s = s.wrapping_mul(2_246_822_519);
    s ^= s >> 13;
    (s >> 8) as f32 * (2.0 / (1 << 23) as f32) - 1.0
}

/// A guitar-cabinet-shaped impulse response, built from closed-form
/// parts so it is byte-identical everywhere:
///
/// - a direct spike at tap 0 (the driver's initial excursion),
/// - two damped resonant modes (`f1`/`f2`) — a cab's cone and cabinet
///   resonances, which is what gives the IR its recognisable colour,
/// - an exponentially decaying, one-pole low-passed pseudo-noise tail
///   (the room/mic reflections).
///
/// `seed` and the two mode frequencies differ between L and R, so the
/// two convolver channels carry genuinely different filters.
fn cab_impulse_response(seed: u64, f1: f32, f2: f32) -> Vec<f32> {
    let mut h = Vec::with_capacity(IR_FRAMES);
    let mut lp = 0.0f32;
    for i in 0..IR_FRAMES {
        let t = i as f32 / IR_SR;
        let direct = if i == 0 { 0.9 } else { 0.0 };
        let modes = 0.55 * (-90.0 * t).exp() * (f1 * t * TAU).sin()
            + 0.30 * (-150.0 * t).exp() * (f2 * t * TAU).sin();
        // One-pole low pass over the noise so the tail is dark, the way
        // a real cab's is; the coefficient is fixed, not tuned.
        lp += 0.25 * (noise(seed + i as u64) - lp);
        let tail = 0.22 * (-45.0 * t).exp() * lp;
        h.push(direct + modes + tail);
    }
    h
}

/// Encode two channels as a 16-bit PCM stereo WAV, so the golden covers
/// the decode path the plugin really uses rather than handing the
/// engine a `Vec<f32>` directly.
fn encode_wav(left: &[f32], right: &[f32], sample_rate: u32) -> Vec<u8> {
    let frames = left.len().min(right.len());
    let data_len = (frames * 2 * 2) as u32; // 2 channels, 2 bytes each
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&2u16.to_le_bytes()); // channels
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&(sample_rate * 4).to_le_bytes()); // byte rate
    w.extend_from_slice(&4u16.to_le_bytes()); // block align
    w.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        for v in [left[i], right[i]] {
            let s = (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            w.extend_from_slice(&s.to_le_bytes());
        }
    }
    w
}

/// Which of the two IRs a scenario loads. `A` is the darker, boxier
/// cab; `B` is brighter with its resonances moved, so the swap
/// crossfade has two audibly different filters to cross between.
#[derive(Clone, Copy, PartialEq)]
enum Ir {
    A,
    B,
}

/// Load an IR through the real WAV decode + resample path and build a
/// convolver for `block_size` from it.
fn convolver(ir: Ir, block_size: usize) -> StereoConvolver {
    let (l, r) = match ir {
        Ir::A => (
            cab_impulse_response(1, 95.0, 640.0),
            cab_impulse_response(1_000_003, 102.0, 710.0),
        ),
        Ir::B => (
            cab_impulse_response(7, 150.0, 1900.0),
            cab_impulse_response(2_000_003, 163.0, 2150.0),
        ),
    };
    let wav = encode_wav(&l, &r, IR_SR as u32);
    let data = load_ir_from_bytes(&wav, SR).expect("the synthesised WAV must decode");
    assert!(data.stereo, "the fixture is stereo; the decoder lost a channel");
    StereoConvolver::new(&data.left, Some(&data.right), block_size)
}

#[derive(Clone, Copy)]
enum Signal {
    /// Unit impulse on L at sample 0 and R at sample 29: the output is
    /// the loaded impulse response itself.
    Impulse,
    /// A DI-style 10 Hz pluck train — what a player actually sends a
    /// cab IR.
    Di,
}

impl Signal {
    fn sample(self, n: u64) -> (f32, f32) {
        let t = n as f32 / SR;
        match self {
            Signal::Impulse => (
                if n == 0 { 1.0 } else { 0.0 },
                if n == 29 { 1.0 } else { 0.0 },
            ),
            Signal::Di => {
                let x = t % 0.1;
                let env = (-30.0 * x).exp();
                let v = 0.8
                    * env
                    * ((196.0 * t * TAU).sin()
                        + 0.5 * (392.0 * t * TAU).sin()
                        + 0.25 * (1176.0 * t * TAU).sin());
                (v, v * 0.85)
            }
        }
    }
}

/// A parameter edit applied *between* blocks; the smoothers are
/// retargeted from the params at the top of every block, exactly as
/// `lib.rs` does.
type MidRunEdit = fn(&IrParams, usize);

/// `Copy` so the guard test below can build a "same scenario, no IR"
/// variant with struct-update syntax and render both.
#[derive(Clone, Copy)]
struct Scenario {
    name: &'static str,
    signal: Signal,
    blocks: &'static [usize],
    mode: LatencyMode,
    /// The IR installed before the run; `None` renders the no-IR path.
    ir: Option<Ir>,
    /// If set, `begin_swap` to this IR at the given block.
    swap_to: Option<(Ir, usize)>,
    setup: fn(&IrParams),
    edit: Option<MidRunEdit>,
}

fn scenarios() -> Vec<Scenario> {
    vec![
        // 1. Impulse in, fully wet: the golden for this scenario is the
        //    loaded impulse response as the convolver reproduces it —
        //    every partition, every overlap-add. If the FFT sizes, the
        //    partition count or the accumulation order changes, this is
        //    where it shows.
        Scenario {
            name: "impulse_is_the_ir",
            signal: Signal::Impulse,
            blocks: &[256],
            mode: LatencyMode::Normal,
            ir: Some(Ir::A),
            swap_to: None,
            setup: |p| {
                p.dry_wet.set_value(1.0);
                p.output_gain.set_value(1.0);
            },
            edit: None,
        },
        // 2. A DI signal through the cab at unity: the everyday path,
        //    with continuous excitation so the convolver is always
        //    straddling partition boundaries rather than decaying into
        //    zeros.
        Scenario {
            name: "di_through_cab",
            signal: Signal::Di,
            blocks: &[256, 128, 97],
            mode: LatencyMode::Normal,
            ir: Some(Ir::A),
            swap_to: None,
            setup: |p| {
                p.dry_wet.set_value(1.0);
                p.output_gain.set_value(1.0);
            },
            edit: None,
        },
        // 3. The blend and the trim swept across the whole run. The dry
        //    side is the input delayed by exactly one convolution block
        //    so it lines up with the wet; an off-by-one there is
        //    inaudible at 0% or 100% and comb-filters everywhere in
        //    between, which is precisely what this scenario renders.
        Scenario {
            name: "blend_and_gain_ramps",
            signal: Signal::Di,
            blocks: &[192, 320],
            mode: LatencyMode::Normal,
            ir: Some(Ir::A),
            swap_to: None,
            setup: |p| {
                p.dry_wet.set_value(0.0);
                p.output_gain.set_value(0.1);
            },
            edit: Some(|p, block| {
                let t = block as f32 / BLOCKS as f32;
                p.dry_wet.set_value(t);
                // The full trim range, so the smoother is always
                // mid-ramp at a block boundary.
                p.output_gain.set_value(0.1 + 9.9 * t);
            }),
        },
        // 4. Swapping to a different IR mid-run: fade the old one out
        //    over 64 samples, hand over, fade the new one in. Both IRs
        //    are audibly different, so a broken handover is a step in
        //    the waveform rather than a subtle level change.
        Scenario {
            name: "swap_crossfade_mid_run",
            signal: Signal::Di,
            blocks: &[256, 128],
            mode: LatencyMode::Normal,
            ir: Some(Ir::A),
            swap_to: Some((Ir::B, BLOCKS / 2)),
            setup: |p| {
                p.dry_wet.set_value(1.0);
                p.output_gain.set_value(1.0);
            },
            edit: None,
        },
        // 5. No IR loaded at all. This is not silence and it is not the
        //    input either: it is the input delayed by one convolution
        //    block and scaled by the trim, which is what keeps an
        //    un-loaded plugin time-aligned with the rest of the mix.
        Scenario {
            name: "no_ir_delayed_dry",
            signal: Signal::Di,
            blocks: &[256],
            mode: LatencyMode::Normal,
            ir: None,
            swap_to: None,
            setup: |p| {
                p.dry_wet.set_value(1.0);
                p.output_gain.set_value(0.8);
            },
            edit: None,
        },
        // 6+7. The other two latency modes. A partitioned convolver is
        //    supposed to compute the *same* convolution at any hop size
        //    — only the latency and the CPU cost change — so these two
        //    goldens are what would catch a partitioning bug that only
        //    bites at one block size.
        Scenario {
            name: "tracking_mode_short_blocks",
            signal: Signal::Impulse,
            blocks: &[64, 32],
            mode: LatencyMode::Tracking,
            ir: Some(Ir::A),
            swap_to: None,
            setup: |p| {
                p.dry_wet.set_value(1.0);
                p.output_gain.set_value(1.0);
            },
            edit: None,
        },
        Scenario {
            name: "efficient_mode_long_blocks",
            signal: Signal::Impulse,
            blocks: &[320, 256],
            mode: LatencyMode::Efficient,
            ir: Some(Ir::A),
            swap_to: None,
            setup: |p| {
                p.dry_wet.set_value(1.0);
                p.output_gain.set_value(1.0);
            },
            edit: None,
        },
    ]
}

/// Deterministic render of one scenario into the golden sample stream.
fn render_scenario(s: &Scenario) -> Vec<f32> {
    let params = IrParams::default();
    (s.setup)(&params);

    let block_size = block_size_for(SR, s.mode);
    let mut engine = IrEngine::new(block_size);
    let mut smoothers = IrSmoothers::new();
    smoothers.prepare(SR, &params);
    engine.reset();
    if let Some(ir) = s.ir {
        engine.install(convolver(ir, block_size));
    }

    let mut out = Vec::new();
    let mut left = vec![0.0f32; MAX_BLOCK];
    let mut right = vec![0.0f32; MAX_BLOCK];
    let mut n: u64 = 0;

    for block in 0..BLOCKS {
        if let Some(edit) = s.edit {
            edit(&params, block);
        }
        if let Some((ir, at)) = s.swap_to {
            if block == at {
                engine.begin_swap(convolver(ir, block_size));
            }
        }
        smoothers.retarget_from(&params);

        let frames = s.blocks[block % s.blocks.len()];
        for i in 0..frames {
            let (l, r) = s.signal.sample(n + i as u64);
            left[i] = l;
            right[i] = r;
        }
        engine.process_block(
            &mut left[..frames],
            &mut right[..frames],
            &mut smoothers.dry_wet,
            &mut smoothers.output_gain,
        );
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
fn ir_output_matches_golden() {
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
        "IR golden: {diff_count}/{} samples differ | peak delta {max_abs:.3e} \
         (limit {MAX_PEAK_DELTA:.1e}) | rms delta {rms_err:.3e} (limit \
         {MAX_RMS_DELTA:.1e})",
        rendered.len()
    );

    if let Some((i, got, want)) = first_diff {
        assert!(
            max_abs <= MAX_PEAK_DELTA && rms_err <= MAX_RMS_DELTA,
            "IR engine output moved beyond the FFT-rounding budget: \
             {diff_count}/{} samples differ, peak delta {max_abs:.3e} (limit \
             {MAX_PEAK_DELTA:.1e}), rms delta {rms_err:.3e} (limit \
             {MAX_RMS_DELTA:.1e}); first at sample {i}: got {got:?}, want \
             {want:?}.\nThis is a real change to the convolution path, not \
             CPU-dependent FFT rounding. If it was intended, re-bless with \
             RESONANCE_BLESS=1.",
            rendered.len()
        );
    }
}

/// Guards the fixture itself: the synthesised IR must survive the WAV
/// round trip as a real, stereo, non-trivial filter. If the encoder or
/// the decoder ever degraded it to silence or to a single spike, every
/// scenario would still render *something* and the golden would pin the
/// degraded version forever.
#[test]
fn the_synthesised_ir_round_trips_as_a_real_filter() {
    let block = block_size_for(SR, LatencyMode::Normal);
    for ir in [Ir::A, Ir::B] {
        // Rebuild the same bytes the scenarios use and inspect the
        // decoded result rather than the pre-encode floats.
        let (l, r) = match ir {
            Ir::A => (
                cab_impulse_response(1, 95.0, 640.0),
                cab_impulse_response(1_000_003, 102.0, 710.0),
            ),
            Ir::B => (
                cab_impulse_response(7, 150.0, 1900.0),
                cab_impulse_response(2_000_003, 163.0, 2150.0),
            ),
        };
        let wav = encode_wav(&l, &r, IR_SR as u32);
        let data = load_ir_from_bytes(&wav, SR).expect("fixture WAV must decode");

        assert!(data.stereo, "fixture lost its second channel in the decode");
        // 1024 frames at 44.1 kHz become ~1114 at 48 kHz.
        assert!(
            data.left.len() > IR_FRAMES,
            "decode did not resample 44.1 kHz -> 48 kHz (got {} frames)",
            data.left.len()
        );
        let energy: f32 = data.left.iter().skip(64).map(|x| x * x).sum();
        assert!(
            energy > 1e-3,
            "the IR has no body past its first 64 taps — it is a spike, not a \
             cabinet, and convolving with it would prove nothing"
        );
        assert!(
            data.left != data.right,
            "the two IR channels are identical — a channel swap would be \
             invisible to this golden"
        );
        // And it must actually build a convolver at the plugin's hop.
        let _ = StereoConvolver::new(&data.left, Some(&data.right), block);
    }
}

/// Guards the scenario table: every scenario must render audio, and the
/// ones with an IR loaded must depart from a plain delayed copy of
/// their input — otherwise the convolver is not convolving and the
/// golden pins a wire.
#[test]
fn every_scenario_convolves() {
    for s in scenarios() {
        let out = render_scenario(&s);
        assert!(
            out.iter().all(|x| x.is_finite()),
            "scenario `{}` rendered non-finite samples",
            s.name
        );
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-4, "scenario `{}` rendered silence", s.name);

        if s.ir.is_none() {
            continue;
        }
        match s.signal {
            Signal::Impulse => {
                // An unloaded convolver would put everything within a
                // couple of samples of the block-size delay. Require a
                // real response spread over hundreds of taps.
                let above: usize = out.iter().filter(|x| x.abs() > 1e-3).count();
                assert!(
                    above > 100,
                    "scenario `{}` produced only {above} significant samples — \
                     that is a delayed spike, not a convolution",
                    s.name
                );
            }
            Signal::Di => {
                // Compare with the same input rendered through the
                // no-IR path (delayed dry): a convolved signal cannot
                // be a scalar multiple of it.
                let mut dry_scenario = Scenario {
                    ir: None,
                    swap_to: None,
                    edit: None,
                    ..s
                };
                dry_scenario.setup = |p| {
                    p.dry_wet.set_value(1.0);
                    p.output_gain.set_value(1.0);
                };
                let dry = render_scenario(&dry_scenario);
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
                    resid > 0.1 * energy,
                    "scenario `{}` is within 10% of a scaled, delayed copy of its \
                     input — the IR is not being applied",
                    s.name
                );
            }
        }
    }
}
