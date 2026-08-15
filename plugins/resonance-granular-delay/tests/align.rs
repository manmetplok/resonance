//! WSOLA onset alignment through the full plugin stack (ba todo #1320,
//! doc #252 §4-5).
//!
//! The engine-level behaviour — that alignment reduces splice roughness,
//! stays inside its window and is bit-identical when off — is pinned by
//! `resonance-dsp/tests/granular_align.rs`. What this file pins is the
//! part the audit found missing: the capability is *reachable*. The
//! `align` parameter must travel param → `BlockParams` → `GrainParams`
//! and actually move onsets, and switching it off must leave the render
//! path exactly where it was.

use resonance_granular_delay::dsp::ALIGN_WINDOW_SECONDS;
use resonance_granular_delay::params::PARAM_COUNT;
use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;
const BLOCKS: usize = 200;

/// Deterministic, strongly pitched material: a 220 Hz sawtooth, the
/// case correlation alignment exists for (a steady periodic splice is
/// what audibly combs when onsets land at arbitrary phase).
fn saw(n: u64) -> f32 {
    let t = n as f32 / SR;
    let mut v = 0.0;
    for h in 1..=10 {
        v += (1.0 / h as f32) * (220.0 * h as f32 * t * std::f32::consts::TAU).sin();
    }
    0.3 * v
}

/// Wet-forward granulation with position spray on.
///
/// Spray is what alignment is *for*: without it a fixed-offset grain
/// tap already continues itself sample for sample (the template and the
/// candidate coincide, so the correlator's best lag is 0 and alignment
/// is a no-op by construction). Sprayed onsets land at an arbitrary
/// phase of the sounding material, which is the splice the correlator
/// repairs. Everything else is held still so the aligner is the only
/// thing that can move a read position.
fn plugin(align: bool) -> ResonanceGranularDelay {
    let mut p = ResonanceGranularDelay::new();
    p.params.sync.set_plain(0.0);
    p.params.time_ms.set_value(60.0);
    p.params.grain_size_ms.set_value(40.0);
    p.params.density_hz.set_value(30.0);
    p.params.scheduler.set_value(0); // Sync: deterministic onsets
    p.params.mix.set_value(1.0);
    p.params.feedback.set_value(0.0);
    p.params.spray_ms.set_value(10.0);
    p.params.size_jitter.set_value(0.0);
    p.params.level_jitter.set_value(0.0);
    p.params.reverse_prob.set_value(0.0);
    p.params.pan_spread.set_value(0.0);
    p.params.pitch.set_value(0.0);
    p.params.spread_cents.set_value(0.0);
    p.params.align.set_value(align);
    p.initialize(SR, BLOCK as u32);
    p
}

/// Render `BLOCKS` blocks of sawtooth, returning the wet output.
fn render(p: &mut ResonanceGranularDelay) -> Vec<f32> {
    let mut out = Vec::with_capacity(BLOCKS * BLOCK);
    let mut left = [0.0f32; BLOCK];
    let mut right = [0.0f32; BLOCK];
    let mut n = 0u64;
    for _ in 0..BLOCKS {
        for i in 0..BLOCK {
            let v = saw(n + i as u64);
            left[i] = v;
            right[i] = v;
        }
        {
            let mut outs = [OutputBuffer {
                left: &mut left[..],
                right: &mut right[..],
            }];
            let mut ev = EventIterator::empty();
            p.process(&mut outs, BLOCK, &mut ev, None);
        }
        n += BLOCK as u64;
        out.extend_from_slice(&left);
    }
    out
}

/// The parameter exists on the public surface, is a boolean, and is off
/// by default — so no existing project or preset changes character.
#[test]
fn align_is_a_declared_boolean_parameter_defaulting_off() {
    let p = ResonanceGranularDelay::new();
    let param = p.params.param_at(29);
    assert_eq!(param.id(), "align");
    assert_eq!((param.min_plain(), param.max_plain()), (0.0, 1.0));
    assert_eq!(param.default_plain(), 0.0);
    assert!(!p.params.align.value());
    assert_eq!(PARAM_COUNT, 30, "align must be part of the param surface");
}

/// The whole point of the todo: turning the parameter on reaches
/// `GrainParams::align` and the engine actually moves onsets. Turning it
/// off leaves every onset on its nominal position.
#[test]
fn the_align_param_reaches_the_grain_engine() {
    let mut off = plugin(false);
    render(&mut off);
    assert_eq!(
        off.aligned_spawns(),
        0,
        "with Align off no onset may be moved"
    );
    assert_eq!(off.max_align_lag_samples(), 0.0);

    let mut on = plugin(true);
    render(&mut on);
    assert!(
        on.aligned_spawns() > 0,
        "with Align on the engine must correlate and snap onsets \
         (the param never reached GrainParams)"
    );
}

/// Aligned onsets stay inside the configured search window, so the
/// switch can never move a grain audibly far from its delay time.
#[test]
fn aligned_onsets_stay_inside_the_search_window() {
    let mut on = plugin(true);
    render(&mut on);
    let bound = f64::from(ALIGN_WINDOW_SECONDS * SR);
    let lag = on.max_align_lag_samples();
    assert!(lag > 0.0, "no alignment happened, nothing was bounded");
    assert!(
        lag <= bound + 1e-6,
        "alignment moved an onset {lag} samples, outside the {bound}-sample window"
    );
}

/// Align off must be the pre-#1320 engine, bit for bit: the parameter
/// may not perturb the render path it is not switched into. (The DSP
/// golden pins the same property across the whole scenario set; this
/// asserts it directly against the aligned run, which must differ.)
#[test]
fn align_off_renders_identically_and_on_renders_differently() {
    let mut a = plugin(false);
    let mut b = plugin(false);
    assert_eq!(
        render(&mut a).iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        render(&mut b).iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        "the unaligned path must be deterministic"
    );

    let mut on = plugin(true);
    let aligned = render(&mut on);
    let unaligned = render(&mut plugin(false));
    let departure = aligned
        .iter()
        .zip(&unaligned)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    assert!(
        departure > 1e-4,
        "Align on rendered the same audio as Align off ({departure})"
    );
}
