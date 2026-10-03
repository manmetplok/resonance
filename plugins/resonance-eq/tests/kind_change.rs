//! Changing a band's kind must not click (FU-M6c).
//!
//! A kind change repurposes every biquad stage of the band, so their state
//! is restarted from zero — and a TDF-II section restarted mid-signal jumps
//! from the old filter's output to `b0 * x`, a step as large as the signal
//! itself on loud material. The band now crossfades from the old stages
//! (still running on their old coefficients) to the restarted new ones over
//! ~5 ms, so the output moves from one steady state to the other without a
//! discontinuity.

use resonance_eq::dsp::EqDsp;
use resonance_eq::params::EqParams;
use resonance_plugin::{Smoother, SmoothingStyle};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
const TONE_HZ: f32 = 110.0;
const AMP: f32 = 0.9;

fn unity() -> Smoother {
    let mut s = Smoother::new(SmoothingStyle::Logarithmic(20.0));
    s.set_sample_rate(SR);
    s.reset(1.0);
    s.set_target(1.0);
    s
}

/// Largest sample-to-sample step in `x`.
fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

/// Render `blocks` blocks of a loud low tone through band 2, switching its
/// kind from `from` to `to` at the start of block `switch_at`.
fn render(from: i32, to: i32, switch_at: usize, blocks: usize) -> Vec<f32> {
    let params = EqParams::default();
    let band = &params.bands[2];
    band.enabled.set_value(true);
    band.freq.set_value(150.0);
    band.gain.set_value(12.0);
    band.q.set_value(1.0);
    band.kind.set_value(from);

    let mut dsp = EqDsp::new(SR);
    let mut gain = unity();
    let mut out = Vec::with_capacity(blocks * BLOCK);
    let mut t = 0usize;
    for b in 0..blocks {
        if b == switch_at {
            band.kind.set_value(to);
        }
        dsp.update_from_params(&params);
        let mut l: Vec<f32> = (0..BLOCK)
            .map(|i| AMP * (std::f32::consts::TAU * TONE_HZ * (t + i) as f32 / SR).sin())
            .collect();
        let mut r = l.clone();
        dsp.process_stereo(&mut l, &mut r, &mut gain);
        out.extend_from_slice(&l);
        t += BLOCK;
    }
    out
}

#[test]
fn kind_change_crossfades_instead_of_clicking() {
    // Bell <-> low shelf, and into a cut: each pair restarts the stages.
    for (from, to) in [(0, 1), (1, 0), (0, 3), (3, 2)] {
        let switch_at = 40;
        let out = render(from, to, switch_at, 60);
        let (before, after) = out.split_at(switch_at * BLOCK);
        let peak = before.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.1, "{from}->{to}: rendered silence ({peak})");

        // The largest step a steady tone makes, on either side of the switch
        // (the new kind's steady state may be louder or quieter).
        let steady = max_step(&before[before.len() - BLOCK..])
            .max(max_step(&after[after.len() - BLOCK..]));
        // Across the switch: the last pre-switch sample through the first
        // 20 ms after it.
        let seam = &out[switch_at * BLOCK - 1..switch_at * BLOCK + 960];
        let seam_step = max_step(seam);
        assert!(
            seam_step < steady * 1.5,
            "{from}->{to}: kind change clicked — largest step across the switch \
             {seam_step:.4} vs {steady:.4} for the steady tone"
        );
    }
}

/// The fade ends: once it and the new stages' start-up transient are over,
/// the band renders what a band that had been the new kind all along
/// renders, so no lingering old filter colours the result. (Not bit-exact:
/// the f32 biquad's rounding noise, boosted by the +12 dB low shelf, leaves
/// a ~1e-4 floor between two differently-started runs on a ~3.6 peak.)
#[test]
fn kind_change_settles_on_the_new_kind() {
    let switched = render(0, 1, 10, 40);
    let native = render(1, 1, 10, 40);
    let tail = 30 * BLOCK..;
    let err = switched[tail.clone()]
        .iter()
        .zip(&native[tail])
        .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(err < 1e-3, "after the fade the band is not the new kind: max diff {err}");
}

/// Render a loud low tone through band 2 configured as `kind`, toggling
/// `enabled` at each block in `toggles` (starting enabled).
fn render_toggled(kind: i32, slope: i32, toggles: &[usize], blocks: usize) -> Vec<f32> {
    let params = EqParams::default();
    let band = &params.bands[2];
    band.enabled.set_value(true);
    band.freq.set_value(150.0);
    band.gain.set_value(12.0);
    band.q.set_value(1.0);
    band.kind.set_value(kind);
    band.slope.set_value(slope);

    let mut dsp = EqDsp::new(SR);
    let mut gain = unity();
    let mut out = Vec::with_capacity(blocks * BLOCK);
    let mut on = true;
    for b in 0..blocks {
        if toggles.contains(&b) {
            on = !on;
            band.enabled.set_value(on);
        }
        dsp.update_from_params(&params);
        let t = b * BLOCK;
        let mut l: Vec<f32> = (0..BLOCK)
            .map(|i| AMP * (std::f32::consts::TAU * TONE_HZ * (t + i) as f32 / SR).sin())
            .collect();
        let mut r = l.clone();
        dsp.process_stereo(&mut l, &mut r, &mut gain);
        out.extend_from_slice(&l);
    }
    out
}

/// DSP2-11: switching a band off or on crossfades like a kind change.
/// Bypassing a +12 dB bell or a 48 dB/oct cut used to cut over in one
/// sample, a step of most of the signal.
#[test]
fn band_enable_toggle_crossfades_instead_of_clicking() {
    // (kind, slope): +12 dB bell, and a 48 dB/oct low cut above the tone.
    for (kind, slope) in [(0, 0), (3, 2)] {
        let (off_at, on_at) = (40, 80);
        let out = render_toggled(kind, slope, &[off_at, on_at], 120);
        let steady = max_step(&out[(off_at - 1) * BLOCK..off_at * BLOCK])
            .max(max_step(&out[(on_at - 1) * BLOCK..on_at * BLOCK]));
        assert!(steady > 0.005, "kind {kind}: rendered (near) silence");
        for (name, at) in [("off", off_at), ("on", on_at)] {
            let seam = &out[at * BLOCK - 1..at * BLOCK + 960];
            let seam_step = max_step(seam);
            assert!(
                seam_step < steady * 1.5,
                "kind {kind}: switching {name} clicked — step {seam_step:.4} vs \
                 {steady:.4} for the steady tone"
            );
        }
    }
}
