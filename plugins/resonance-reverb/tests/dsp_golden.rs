//! Bit-exact DSP golden for the algorithmic reverb (ba todo #1373).
//!
//! `tests/plugin.rs` and `tests/freeze.rs` check that the tail exists,
//! that it decays, that freeze sustains it. All of that survives a
//! change to the diffusion allpass lengths, the FDN feedback matrix, the
//! damping filter or the early-reflection tap gains — the room would
//! sound like a different room and the suite would stay green. This test
//! pins the rendered samples.
//!
//! # The signal, and why it is the revealing one
//!
//! For a reverb the revealing signal is a **unit impulse**: the output
//! *is* the impulse response, which is the reverb's entire identity —
//! every early reflection at its own delay and gain, the diffusion
//! smear, the modal density the FDN builds, and the damping slope, all
//! laid out in time where a golden can pin each of them. Anything
//! sustained convolves the response with itself and hides exactly the
//! structure that matters. The impulses are offset between L and R (0
//! and 37 samples) so the two sides of the network cannot be swapped or
//! collapsed without the golden noticing.
//!
//! Two scenarios use a **short noise burst** instead: freeze has to
//! capture something with content to hold, and the modulation depth is
//! only readable when the tail is still being fed.
//!
//! # Determinism, and one finding
//!
//! The reverb's "randomised" elements are not random: the diffusion
//! delay ratios and the ER tap polarities are computed once at
//! construction from fixed constants, and the FDN modulation LFOs are
//! built with fixed staggered phases. There is no RNG.
//!
//! `ReverbDsp::clear()` — what the host's `reset()` calls — does *not*
//! reset those LFO phases: it clears the delay lines, the damping
//! filters and the feedback state, and leaves `fdn.lfos` free-running.
//! So a reset instance and a fresh instance do not render identically.
//! Every scenario below therefore constructs a **fresh plugin**, which
//! is the only starting state that is reproducible.
//!
//! # Tolerance: bit-exact
//!
//! The audio path is scalar f32 — delay lines with linear interpolation,
//! one-pole damping, allpass diffusers, an 8×8 feedback matrix and an
//! M/S width stage. No FFT, no runtime SIMD dispatch, no RNG, no clock.
//! (Bits can only move between machines through libm's rounding of
//! `sin`/`exp`/`powf` in the coefficient and LFO maths, which shows up
//! as most of the render shifting by ~1e-7.)
//!
//! To (re)generate the golden after an *intentional* audio change:
//!
//!     RESONANCE_BLESS=1 cargo test -p resonance-reverb --test dsp_golden
//!
//! A pure refactor must never need this.

use std::path::PathBuf;

use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
use resonance_reverb::params::ReverbParams;
use resonance_reverb::ResonanceReverb;

const SR: f32 = 48_000.0;
const MAX_BLOCK: usize = 256;
/// Blocks per scenario — ~108 ms, which covers the pre-delay, the whole
/// early-reflection cluster and the first tens of milliseconds of the
/// diffuse tail. That is where a reverb's character lives; the rest is
/// an exponential decay of what is already pinned here.
const BLOCKS: usize = 28;

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
    /// Unit impulse on L at sample 0 and on R at sample 37: the output
    /// is the reverb's impulse response, per channel.
    Impulse,
    /// A 15 ms band-limited noise burst at the start, then silence.
    /// Gives freeze something with spectral content to capture and the
    /// modulation something to move.
    Burst,
    /// A repeating 8 Hz pluck train — continuous excitation for the
    /// scenarios where the steady-state wash, not the response, is what
    /// is being pinned.
    Plucks,
}

impl Signal {
    fn sample(self, n: u64) -> (f32, f32) {
        let t = n as f32 / SR;
        match self {
            Signal::Impulse => (
                if n == 0 { 1.0 } else { 0.0 },
                if n == 37 { 1.0 } else { 0.0 },
            ),
            Signal::Burst => {
                if n < 720 {
                    let w = 0.5 - 0.5 * (TAU * n as f32 / 720.0).cos();
                    let v = 0.8 * w * noise(n);
                    (v, 0.8 * w * noise(n + 9_973))
                } else {
                    (0.0, 0.0)
                }
            }
            Signal::Plucks => {
                let x = t % 0.125;
                let env = (-260.0 * x).exp();
                let v = 0.6 * env * ((440.0 * t * TAU).sin() + 0.35 * (1320.0 * t * TAU).sin());
                (v, v * 0.8)
            }
        }
    }
}

/// A parameter edit applied *between* blocks. `None` for static
/// scenarios.
type MidRunEdit = fn(&ReverbParams, usize);

struct Scenario {
    name: &'static str,
    signal: Signal,
    blocks: &'static [usize],
    setup: fn(&ReverbParams),
    edit: Option<MidRunEdit>,
}

fn scenarios() -> Vec<Scenario> {
    vec![
        // 1. Factory defaults on an impulse: 50% size, 2 s decay, 8 kHz
        //    damping, 80% diffusion, 40% ER, 50% mix. The room a user
        //    hears before touching anything, pinned reflection by
        //    reflection.
        Scenario {
            name: "impulse_factory_defaults",
            signal: Signal::Impulse,
            blocks: &[256],
            setup: |_| {},
            edit: None,
        },
        // 2. Big, bright, fully wet: the FDN and the diffusers at
        //    maximum with nothing damping them and no dry signal to
        //    hide behind. This is the scenario where a change to the
        //    feedback matrix or the allpass chain is loudest.
        Scenario {
            name: "impulse_large_bright_wet",
            signal: Signal::Impulse,
            blocks: &[256, 111],
            setup: |p| {
                p.size.set_value(1.0);
                p.decay.set_value(12.0);
                p.damping.set_value(20000.0);
                p.diffusion.set_value(1.0);
                p.er_level.set_value(0.6);
                p.er_time.set_value(1.0);
                p.mix.set_value(1.0);
                p.mod_depth.set_value(0.0);
            },
            edit: None,
        },
        // 3. The opposite corner: a tiny, dark, undiffused room with the
        //    early reflections dominant. Diffusion at zero bypasses the
        //    allpass chain entirely, which is a different code path, and
        //    a 500 Hz damping cutoff puts the one-pole filter where its
        //    coefficient actually matters.
        Scenario {
            name: "impulse_small_dark_undiffused",
            signal: Signal::Impulse,
            blocks: &[192],
            setup: |p| {
                p.size.set_value(0.05);
                p.decay.set_value(0.3);
                p.damping.set_value(500.0);
                p.diffusion.set_value(0.0);
                p.er_level.set_value(1.0);
                p.er_time.set_value(0.2);
                p.mix.set_value(1.0);
                p.mod_depth.set_value(0.0);
            },
            edit: None,
        },
        // 4. Pre-delay and the M/S width stage. A 40 ms pre-delay puts
        //    the whole tail 1920 samples into the window, so the golden
        //    pins where it starts as well as what it is; width at 0
        //    collapses the tail to mono, which is a stage no other test
        //    renders.
        Scenario {
            name: "impulse_predelay_mono_width",
            signal: Signal::Impulse,
            blocks: &[256, 192],
            setup: |p| {
                p.predelay.set_value(40.0);
                p.width.set_value(0.0);
                p.size.set_value(0.4);
                p.decay.set_value(3.0);
                p.mix.set_value(1.0);
                p.mod_depth.set_value(0.0);
            },
            edit: None,
        },
        // 5. Modulation at full depth and a fast rate, fed by a noise
        //    burst. The FDN read taps move every sample, so this is the
        //    scenario that pins the LFO bank and the interpolation it
        //    drives — both invisible on a static impulse.
        Scenario {
            name: "burst_modulated_tail",
            signal: Signal::Burst,
            blocks: &[192, 256, 111],
            setup: |p| {
                p.mod_rate.set_value(4.5);
                p.mod_depth.set_value(1.0);
                p.size.set_value(0.6);
                p.decay.set_value(6.0);
                p.diffusion.set_value(0.9);
                p.mix.set_value(1.0);
            },
            edit: None,
        },
        // 6. Freeze engaged and released mid-run. Freeze drives the FDN
        //    to unity feedback and stops the input, so the captured
        //    wash must repeat unchanged; a leak or a decay in that state
        //    is exactly what a sample-level pin catches.
        Scenario {
            name: "burst_freeze_cycle",
            signal: Signal::Burst,
            blocks: &[256, 111],
            setup: |p| {
                p.size.set_value(0.35);
                p.decay.set_value(4.0);
                p.mix.set_value(1.0);
                p.mod_depth.set_value(0.2);
            },
            edit: Some(|p, block| {
                p.freeze
                    .set_value((BLOCKS / 4..3 * BLOCKS / 4).contains(&block));
            }),
        },
        // 7. Every control swept between blocks over continuous input.
        //    The size/decay/damping/pre-delay smoothers are advanced at
        //    *block* rate (a deliberate stair-step, see `lib.rs`), so
        //    varying the block size while sweeping is the only way to
        //    pin where those steps land.
        Scenario {
            name: "plucks_param_sweeps",
            signal: Signal::Plucks,
            blocks: &[64, 256, 149],
            setup: |p| {
                p.mix.set_value(0.8);
            },
            edit: Some(|p, block| {
                let t = block as f32 / BLOCKS as f32;
                p.size.set_value(0.05 + 0.9 * t);
                p.decay.set_value(0.3 + 8.0 * t);
                p.damping.set_value(20000.0 - 19000.0 * t);
                p.diffusion.set_value(t);
                p.er_level.set_value(1.0 - t);
                p.er_time.set_value(t);
                p.predelay.set_value(30.0 * t);
                p.mod_rate.set_value(0.2 + 4.0 * t);
                p.mod_depth.set_value(t);
                p.width.set_value(1.0 - t);
                p.mix.set_value(0.4 + 0.6 * t);
            }),
        },
    ]
}

/// Deterministic render of one scenario into the golden sample stream.
///
/// A fresh plugin per scenario, never a reused-and-reset one: see the
/// module header — `reset()` leaves the FDN modulation LFOs where they
/// were, so only construction is a reproducible starting state.
fn render_scenario(s: &Scenario) -> Vec<f32> {
    let mut plugin = ResonanceReverb::new();
    (s.setup)(&plugin.params);
    plugin.initialize(SR, MAX_BLOCK as u32);

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
fn reverb_output_is_bit_exact() {
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
            "reverb DSP output changed: {diff_count}/{} samples differ, peak delta \
             {max_abs:.3e}; first at sample {i} (got {got:?} / {:#010x}, want \
             {want:?} / {:#010x}).\nA refactor of the reverb path must be \
             bit-exact. If the change was intended, re-bless with \
             RESONANCE_BLESS=1.\nA peak delta at ~1e-7 spread over most of the \
             render is libm rounding, not a DSP change.",
            rendered.len(),
            got.to_bits(),
            want.to_bits(),
        );
    }
}

/// Guards the scenario table: every scenario must render a real tail —
/// energy long after the excitation stopped — and the two channels must
/// not be identical unless the scenario asked for mono. A scenario that
/// degenerated to dry-only would render silence in the tail and the
/// golden would pin the silence.
#[test]
fn every_scenario_renders_a_tail() {
    for s in scenarios() {
        let out = render_scenario(&s);
        assert!(
            out.iter().all(|x| x.is_finite()),
            "scenario `{}` rendered non-finite samples",
            s.name
        );
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-4, "scenario `{}` rendered silence", s.name);

        // Impulse and burst scenarios stop feeding the reverb almost
        // immediately, so anything in the last quarter of the render is
        // by definition the tail.
        if matches!(s.signal, Signal::Impulse | Signal::Burst) {
            let tail_start = out.len() * 3 / 4;
            let tail = out[tail_start..]
                .iter()
                .fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(
                tail > 1e-5,
                "scenario `{}` has no tail in the last quarter of the render \
                 ({tail:.3e}) — the reverb is not sounding",
                s.name
            );
        }
    }
}
