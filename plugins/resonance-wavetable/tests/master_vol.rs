//! Master volume must be de-zippered: a host automation jump lands on the
//! param instantly (per the `Param::set_plain` contract), and the engine's
//! smoother turns it into a ~5 ms linear ramp instead of a step in the
//! output. The engine is fully deterministic (fixed RNG seed, bundled
//! wavetables), so rendering the same note through two engines — one with a
//! constant master volume, one with a mid-stream jump — yields a per-sample
//! gain trajectory `b[n] / a[n]` that we can pin directly.

use resonance_plugin::{EventIterator, NoteEvent};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::params::WavetableParams;

const SR: f32 = 48_000.0;
const BLOCK: usize = 512;
const SMOOTH_MS: f32 = 5.0;

fn render_blocks(
    engine: &mut SynthEngine,
    params: &WavetableParams,
    n_blocks: usize,
    note_on_first: bool,
    out: &mut Vec<f32>,
) {
    for b in 0..n_blocks {
        let mut left = vec![0.0f32; BLOCK];
        let mut right = vec![0.0f32; BLOCK];
        let events = if b == 0 && note_on_first {
            vec![NoteEvent::NoteOn {
                note: 60,
                velocity: 1.0,
                timing: 0,
            }]
        } else {
            Vec::new()
        };
        let mut iter = EventIterator::new(&events);
        engine.render_block(&mut left, &mut right, BLOCK, params, &mut iter);
        out.extend_from_slice(&left);
    }
}

#[test]
fn master_vol_jump_ramps_without_step_discontinuity() {
    const PRE_BLOCKS: usize = 10;
    const POST_BLOCKS: usize = 10;
    let jump_at = PRE_BLOCKS * BLOCK;

    // Reference: constant master volume 1.0 for the whole render.
    let params_a = WavetableParams::new();
    params_a.master_volume.set_value(1.0);
    let mut engine_a = SynthEngine::new();
    engine_a.initialize(SR);
    let mut a = Vec::new();
    render_blocks(&mut engine_a, &params_a, PRE_BLOCKS + POST_BLOCKS, true, &mut a);

    // Same engine/note, but master volume jumps 1.0 -> 0.25 at a block
    // boundary (exactly how block-quantized host automation lands).
    let params_b = WavetableParams::new();
    params_b.master_volume.set_value(1.0);
    let mut engine_b = SynthEngine::new();
    engine_b.initialize(SR);
    let mut b = Vec::new();
    render_blocks(&mut engine_b, &params_b, PRE_BLOCKS, true, &mut b);
    params_b.master_volume.set_value(0.25);
    render_blocks(&mut engine_b, &params_b, POST_BLOCKS, false, &mut b);

    // Identical before the jump.
    for n in 0..jump_at {
        assert!(
            (a[n] - b[n]).abs() < 1e-6,
            "pre-jump divergence at sample {n}: {} vs {}",
            a[n],
            b[n]
        );
    }

    // After the jump, b[n] = g[n] * a[n] where g is the effective gain
    // trajectory. A linear 5 ms ramp moves at most `step` per sample; an
    // unsmoothed gain would step 0.75 in one sample at the boundary.
    let ramp_samples = (SR * SMOOTH_MS / 1000.0).ceil();
    let step = 0.75 / ramp_samples;
    let window = (jump_at - 256)..(jump_at + BLOCK);
    let mut prev: Option<(usize, f32)> = None;
    let mut checked = 0usize;
    for n in window {
        // Skip near-zero crossings where the ratio is ill-conditioned.
        if a[n].abs() < 0.01 {
            continue;
        }
        let g = b[n] / a[n];
        if let Some((pn, pg)) = prev {
            let bound = step * (n - pn) as f32 + 1e-3;
            assert!(
                (g - pg).abs() <= bound,
                "gain step discontinuity at sample {n} (jump at {jump_at}): \
                 {pg} -> {g} over {} samples (allowed {bound})",
                n - pn
            );
        }
        prev = Some((n, g));
        checked += 1;
    }
    assert!(checked > 100, "too few usable samples ({checked})");

    // The ramp converges: well after the jump the gain sits at 0.25.
    let settled = jump_at + ramp_samples as usize + 100;
    for n in settled..settled + BLOCK {
        if a[n].abs() < 0.01 {
            continue;
        }
        let g = b[n] / a[n];
        assert!(
            (g - 0.25).abs() < 1e-3,
            "gain not settled at sample {n}: {g}"
        );
    }
}
