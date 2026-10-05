//! `reset()` must return the reverb to its constructed state (ba todo #1378).
//!
//! CLAP's `reset()` means "you are about to be handed unrelated audio,
//! forget everything". Hosts call it on transport stop/start and when the
//! playhead is relocated, so two bounces of the same passage from
//! different transport histories have to come out the same — an offline
//! render that depends on what played before it is not reproducible.
//!
//! It did not hold. `FdnBank::clear()` cleared the delay lines, the
//! damping filters and the feedback state, but never touched `lfos`,
//! while the FDN advances every LFO on every sample regardless of mod
//! depth. So the modal detuning resumed at whatever phase the previous
//! audio left it at, and a reset instance rendered a measurably different
//! tail from a fresh one.
//!
//! This is the assertion `tests/dsp_golden.rs` could not make: its header
//! records that only construction was a reproducible starting state, and
//! every scenario there builds a fresh plugin to sidestep the problem.
//! With this passing, a reused-and-reset instance is equivalent, and the
//! golden's fresh-per-scenario rule becomes a style choice rather than a
//! correctness requirement.

use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
use resonance_reverb::params::ReverbParams;
use resonance_reverb::ResonanceReverb;

const SR: f32 = 48_000.0;
const MAX_BLOCK: usize = 256;

/// Blocks in the reference render — 64 × 256 = 16 384 samples, ~341 ms.
///
/// This length is load-bearing, not a round number. At `size = 0.6` the
/// FDN delay lines are ~2 900 samples long, so a render shorter than
/// that contains only the dry path, the early reflections and the
/// diffusers — and the FDN, which is the ONLY stage the modulation LFOs
/// touch, contributes nothing at all. An earlier draft of this test ran
/// 8 blocks (2 048 samples) and passed against the unfixed code, because
/// the tail it claimed to compare had not started yet.
/// [`the_reference_render_reaches_the_modulated_tail`] pins the property
/// that makes the comparison meaningful, so this cannot silently rot
/// back into a vacuous test if the delay lengths change.
const BLOCKS: usize = 64;

/// Deterministic pseudo-noise from the absolute sample index — content
/// with enough spectral spread to drive the whole network.
fn noise(n: u64) -> f32 {
    let mut s = n.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1) as u32;
    s ^= s >> 16;
    s = s.wrapping_mul(2_246_822_519);
    s ^= s >> 13;
    (s >> 8) as f32 * (2.0 / (1 << 23) as f32) - 1.0
}

/// A 15 ms noise burst then silence: excites the tank, then lets the
/// tail — which is where the modulation lives — run on its own.
fn burst(n: u64) -> (f32, f32) {
    if n < 720 {
        let w = 0.5 - 0.5 * (std::f32::consts::TAU * n as f32 / 720.0).cos();
        (0.8 * w * noise(n), 0.8 * w * noise(n + 9_973))
    } else {
        (0.0, 0.0)
    }
}

/// Modulation wide open and fast, so the LFO phase is the loudest thing
/// in the render. With `mod_depth` at 0 the bug is inaudible; this is the
/// setting that exposes it.
fn modulated(p: &ReverbParams) {
    p.mod_rate.set_value(4.5);
    p.mod_depth.set_value(1.0);
    p.size.set_value(0.6);
    p.decay.set_value(6.0);
    p.diffusion.set_value(0.9);
    p.mix.set_value(1.0);
}

fn render(plugin: &mut ResonanceReverb, blocks: usize) -> Vec<f32> {
    let mut out = Vec::new();
    let mut left = vec![0.0f32; MAX_BLOCK];
    let mut right = vec![0.0f32; MAX_BLOCK];
    let mut n: u64 = 0;

    for _ in 0..blocks {
        for i in 0..MAX_BLOCK {
            let (l, r) = burst(n + i as u64);
            left[i] = l;
            right[i] = r;
        }
        {
            let mut outs = [OutputBuffer {
                left: &mut left[..],
                right: &mut right[..],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, MAX_BLOCK, &mut ev, None);
        }
        n += MAX_BLOCK as u64;
        out.extend_from_slice(&left[..]);
        out.extend_from_slice(&right[..]);
    }
    out
}

fn fresh() -> ResonanceReverb {
    let mut plugin = ResonanceReverb::new();
    // Classic, pinned: a fresh instance runs Room (reverb-algorithms.md D2).
    plugin.params.algorithm.set_value(resonance_reverb::dsp::Algorithm::Classic as i32);
    modulated(&plugin.params);
    plugin.initialize(SR, MAX_BLOCK as u32);
    plugin
}

fn assert_bit_identical(reset: &[f32], fresh: &[f32], what: &str) {
    assert_eq!(reset.len(), fresh.len(), "{what}: render lengths differ");
    let mut diffs = 0usize;
    let mut max_abs = 0.0f32;
    let mut first = None;
    for (i, (a, b)) in reset.iter().zip(fresh).enumerate() {
        if a.to_bits() != b.to_bits() {
            diffs += 1;
            max_abs = max_abs.max((a - b).abs());
            if first.is_none() {
                first = Some((i, *a, *b));
            }
        }
    }
    if let Some((i, got, want)) = first {
        panic!(
            "{what}: a reset instance rendered differently from a fresh one — \
             {diffs}/{} samples differ, peak delta {max_abs:.3e}; first at sample \
             {i} (reset {got:?}, fresh {want:?}).\nSomething in the signal path \
             survives clear(). The FDN modulation LFOs were the original cause \
             (ba todo #1378); check for another free-running accumulator that \
             clear() skips.",
            reset.len()
        );
    }
}

/// Peak level over the last quarter of a render: the region that is
/// nothing but recirculated FDN tail, long after the 15 ms burst and the
/// early-reflection cluster are over.
fn late_tail_peak(render: &[f32]) -> f32 {
    let start = render.len() / 4 * 3;
    render[start..].iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

#[test]
fn the_reference_render_reaches_the_modulated_tail() {
    // The guard on every other test in this file. The LFOs modulate the
    // FDN read taps and NOTHING else, so a render that stops before the
    // FDN's ~2 900-sample delay lines have recirculated compares two
    // stretches of audio that the bug could not have touched — it would
    // pass whether or not `clear()` reset the LFO phase. Shorten BLOCKS
    // or lengthen the delays and this fails first, with the reason.
    let mut plugin = fresh();
    let out = render(&mut plugin, BLOCKS);
    assert!(
        late_tail_peak(&out) > 1e-3,
        "the last quarter of the reference render is effectively silent \
         (peak {:e}) — the FDN tail never arrives, so the reset comparison \
         would be vacuous. Raise BLOCKS or shorten the room size.",
        late_tail_peak(&out)
    );
}

#[test]
fn a_reset_instance_renders_identically_to_a_fresh_one() {
    // Run audio through it, reset, then render the reference passage.
    // The 6 priming blocks leave the LFO bank at an arbitrary phase —
    // roughly 0.44 of a cycle on channel 0, nowhere near a whole number.
    let mut reused = fresh();
    let _ = render(&mut reused, 6);
    reused.reset();
    let after_reset = render(&mut reused, BLOCKS);

    let mut clean = fresh();
    let from_fresh = render(&mut clean, BLOCKS);

    assert_bit_identical(&after_reset, &from_fresh, "modulated tail");
}

#[test]
fn the_prior_audio_does_not_change_what_reset_restores() {
    // Two instances given DIFFERENT histories — different lengths, so
    // they stop at different LFO phases — must still converge after
    // reset. A reset that restored some *fixed but wrong* state would
    // pass the test above and fail this one.
    let mut short_history = fresh();
    let _ = render(&mut short_history, 3);
    short_history.reset();
    let a = render(&mut short_history, BLOCKS);

    let mut long_history = fresh();
    let _ = render(&mut long_history, 11);
    long_history.reset();
    let b = render(&mut long_history, BLOCKS);

    assert_bit_identical(&a, &b, "differing transport histories");
}

#[test]
fn reset_is_idempotent() {
    let mut plugin = fresh();
    let _ = render(&mut plugin, 4);
    plugin.reset();
    plugin.reset();
    plugin.reset();
    let after = render(&mut plugin, BLOCKS);

    let mut clean = fresh();
    let from_fresh = render(&mut clean, BLOCKS);
    assert_bit_identical(&after, &from_fresh, "repeated reset");
}
