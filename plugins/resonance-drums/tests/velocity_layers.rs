//! Velocity that follows the recording (drums-plugin-rework.md §7 E7).
//!
//! Each take's loudness is measured once, where it is built (the RMS of
//! its strike, `kit::LOUDNESS_FRAMES`); a hit picks the layer whose
//! measured level is nearest the level its velocity asks for — a straight
//! dB line from the softest layer to the loudest — and makes up the rest
//! in gain. So a velocity sweep rises monotonically in output level, with
//! no step where one layer hands over to the next, and a bank with fewer
//! layers than the reference still sounds (by relative position).
//! Velocity humanize moves each hit by up to ± N MIDI steps, from a
//! fixed seed: the same hits render the same.

use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::dsp::voice_pick::MIN_VELOCITY_SPAN_DB;
use resonance_drums::dsp::{pick_layer_by_level, DrumSampler, PortBuffers};
use resonance_drums::kit::{
    LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer, LOUDNESS_FRAMES, NUM_OUTPUT_PORTS,
    OVERHEAD_PORT_INDEX,
};
use resonance_drums::params::{humanize_from_label, humanize_label, DrumParams, OUTPUT_MODE_MULTI};
use resonance_plugin::Param;

const SR: f32 = 48_000.0;
/// The kick's close-mic port in Multi.
const KICK_PORT: usize = 1;

/// A strike: a 200 Hz sine at amplitude `amp`, `frames` long (its RMS
/// over the loudness window is `amp / √2` to within 0.02 dB: 17.07
/// periods of it).
fn strike(amp: f32, frames: usize) -> LoadedSample {
    LoadedSample::mono(
        (0..frames)
            .map(|i| (i as f32 * 200.0 * std::f32::consts::TAU / SR).sin() * amp)
            .collect(),
    )
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

fn bank(position: &str, levels_db: &[f32]) -> LoadedMicBank {
    LoadedMicBank {
        position: position.to_string(),
        setup_key: String::new(),
        layers: levels_db
            .iter()
            .map(|&l| VelocityLayer::new(vec![strike(10f32.powf(l / 20.0), LOUDNESS_FRAMES * 2)]))
            .collect(),
    }
}

/// The recorded layer levels of the test kick (dB of the sine's peak):
/// unevenly spaced, as a real recording is — 30 dB in all.
const KICK_LAYERS: [f32; 6] = [-36.0, -30.0, -21.0, -16.0, -12.0, -6.0];

/// A kit whose kick has six close-mic layers and an overhead bank with
/// only three (a mismatched bank); every other pad is empty.
fn kit() -> Vec<LoadedPad> {
    PAD_MAPPINGS
        .iter()
        .enumerate()
        .map(|(i, m)| LoadedPad {
            name: m.name.to_string(),
            choke_group: m.choke_group,
            output_group: m.output_group,
            close_mics: if i == 0 {
                vec![bank("KickIn", &KICK_LAYERS)]
            } else {
                Vec::new()
            },
            overhead: (i == 0).then(|| bank("OHsAB", &[-40.0, -30.0, -20.0])),
        })
        .collect()
}

/// One kick at `velocity`: the RMS (dB) of its close mic over the strike
/// window, and the overhead's peak.
fn kick_level(sampler: &mut DrumSampler, params: &DrumParams, velocity: f32) -> (f32, f32) {
    sampler.reset();
    sampler.update_global_settings(params);
    sampler.note_on(drum_map::KICK, velocity);
    let frames = LOUDNESS_FRAMES;
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0; frames], vec![0.0; frames]))
        .collect();
    {
        let mut ports: Vec<PortBuffers<'_>> = bufs
            .iter_mut()
            .map(|(l, r)| PortBuffers {
                left: l.as_mut_slice(),
                right: r.as_mut_slice(),
            })
            .collect();
        sampler.render_block(&mut ports, frames, params, &[]);
    }
    let close = &bufs[KICK_PORT].0;
    let rms = (close.iter().map(|s| s * s).sum::<f32>() / frames as f32).sqrt();
    let oh = bufs[OVERHEAD_PORT_INDEX]
        .0
        .iter()
        .fold(0.0f32, |m, s| m.max(s.abs()));
    (db(rms), oh)
}

fn multi() -> DrumParams {
    let p = DrumParams::default();
    p.output_mode.set_value(OUTPUT_MODE_MULTI);
    p
}

fn sampler() -> DrumSampler {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut s = DrumSampler::new(rx);
    s.set_sample_rate(SR);
    s.pads = kit();
    s
}

#[test]
fn a_take_measures_the_level_of_its_strike() {
    let layer = VelocityLayer::new(vec![strike(0.5, LOUDNESS_FRAMES), strike(0.25, LOUDNESS_FRAMES)]);
    // A sine's RMS is its peak / √2: -9.03 dB and -15.05 dB; the layer is
    // their mean *power* (-11.07 dB), not the mean of the dB (-12.04).
    let a = layer.round_robins[0].level_db();
    assert!((a - db(0.5 / 2f32.sqrt())).abs() < 0.03, "{a}");
    let power = (0.5f32.powi(2) / 2.0 + 0.25f32.powi(2) / 2.0) / 2.0;
    let want = 10.0 * power.log10();
    assert!((layer.level_db() - want).abs() < 0.03, "{}", layer.level_db());
}

/// A layer whose takes are all silent (or broken) measures as silent —
/// never picked by loudness; a silent take beside sounding ones does not
/// drag the layer down: it is left out.
#[test]
fn silent_takes_are_left_out_of_a_layers_level() {
    use resonance_drums::kit::SILENT_DB;
    let silent = || LoadedSample::mono(vec![0.0; LOUDNESS_FRAMES]);
    assert_eq!(VelocityLayer::new(vec![silent(), silent()]).level_db(), SILENT_DB);
    assert_eq!(VelocityLayer::new(Vec::new()).level_db(), SILENT_DB);
    let with = VelocityLayer::new(vec![strike(0.5, LOUDNESS_FRAMES), silent()]);
    let alone = VelocityLayer::new(vec![strike(0.5, LOUDNESS_FRAMES)]);
    assert_eq!(with.level_db(), alone.level_db());
}

/// Layers of equal loudness (a normalized library) still play softer at
/// a softer velocity, every layer gets its share of the velocity range,
/// and the loudest hit plays at unity.
#[test]
fn equal_loudness_layers_still_have_dynamics() {
    let level = |_: usize| -12.0;
    let (top, g) = pick_layer_by_level(1.0, 4, level);
    assert_eq!((top, g), (3, 1.0));
    let mut layers_seen = [false; 4];
    let mut last = f32::NEG_INFINITY;
    for step in 1..=100 {
        let v = step as f32 / 100.0;
        let (layer, gain) = pick_layer_by_level(v, 4, level);
        layers_seen[layer] = true;
        let out = -12.0 + db(gain);
        assert!(out >= last - 1e-3, "louder at {v}: {out} after {last}");
        last = out;
    }
    assert!(layers_seen.iter().all(|&s| s), "every layer plays: {layers_seen:?}");
    let soft = -12.0 + db(pick_layer_by_level(0.0, 4, level).1);
    assert!(
        (soft - (-12.0 - MIN_VELOCITY_SPAN_DB)).abs() < 0.01,
        "the softest hit is {MIN_VELOCITY_SPAN_DB} dB down: {soft}"
    );
}

/// Exactly one usable layer (the rest silent): it plays every hit, its
/// level following the velocity over the minimum span.
#[test]
fn one_usable_layer_plays_every_hit() {
    let levels = [-150.0, -9.0, -150.0];
    let level = |i: usize| levels[i];
    for step in 0..=20 {
        let v = step as f32 / 20.0;
        let (layer, gain) = pick_layer_by_level(v, 3, level);
        assert_eq!(layer, 1, "at {v}");
        let want = -MIN_VELOCITY_SPAN_DB * (1.0 - v);
        assert!((db(gain) - want).abs() < 0.01, "at {v}: {} dB", db(gain));
    }
}

#[test]
fn the_pick_puts_the_hit_on_a_straight_db_line() {
    let levels = [-30.0, -24.0, -15.0, -10.0, -6.0, 0.0];
    let level = |i: usize| levels[i];
    // The ends play their layers at unity.
    assert_eq!(pick_layer_by_level(0.0, 6, level), (0, 1.0));
    assert_eq!(pick_layer_by_level(1.0, 6, level), (5, 1.0));
    // In between, the layer nearest the target, made up to it in gain.
    for step in 0..=100 {
        let v = step as f32 / 100.0;
        let (layer, gain) = pick_layer_by_level(v, 6, level);
        let target = -30.0 + 30.0 * v;
        let out = levels[layer] + db(gain);
        assert!(
            (out - target).abs() < 1e-3,
            "v {v}: {out} dB, want {target}"
        );
        let nearest = levels
            .iter()
            .map(|l| (l - target).abs())
            .fold(f32::INFINITY, f32::min);
        assert!(((levels[layer] - target).abs() - nearest).abs() < 1e-4);
    }
    // Levels that say nothing (all within 1 dB): equal buckets — and the
    // loudness still on a MIN_VELOCITY_SPAN_DB line (item 8, below).
    assert_eq!(pick_layer_by_level(0.9, 4, |_| -12.0).0, 3);
    assert_eq!(pick_layer_by_level(0.1, 4, |_| -12.0).0, 0);
    let (_, gain) = pick_layer_by_level(0.5, 4, |_| -12.0);
    assert!((db(gain) + MIN_VELOCITY_SPAN_DB / 2.0).abs() < 1e-3);
    // A silent layer is never picked by loudness.
    let with_silent = [-150.0, -20.0, -10.0];
    assert_eq!(pick_layer_by_level(0.0, 3, |i| with_silent[i]).0, 1);
}

/// The velocity sweep: 127 hits, MIDI velocity 1..127. The output level
/// must rise monotonically along the straight line from the softest
/// layer to the loudest, and no step between neighbouring velocities may
/// be larger than that line's own step (30 dB / 126 ≈ 0.24 dB) plus
/// 0.05 dB of slack for f32 rounding.
///
/// That bound is what "no step at a layer boundary" means here: the old
/// equal-bucket pick at unity gain jumped by the gap between two
/// recordings (here up to 9 dB) wherever it changed layer.
#[test]
fn a_velocity_sweep_is_monotonic_with_no_step_at_layer_boundaries() {
    let params = multi();
    let mut s = sampler();
    let lo = KICK_LAYERS[0];
    let hi = KICK_LAYERS[KICK_LAYERS.len() - 1];
    let per_step = (hi - lo) / 126.0;
    let x = per_step + 0.05;
    let mut last: Option<f32> = None;
    for midi in 1..=127u32 {
        let v = midi as f32 / 127.0;
        let (level, oh) = kick_level(&mut s, &params, v);
        assert!(
            oh > 0.0,
            "velocity {midi}: the 3-layer overhead bank is silent"
        );
        if let Some(prev) = last {
            let step = level - prev;
            assert!(step >= -1e-3, "velocity {midi}: level fell by {step} dB");
            assert!(
                step <= x,
                "velocity {midi}: a {step:.3} dB step (bound {x:.3})"
            );
        }
        last = Some(level);
    }
    // The ends are the recordings themselves: the sine's peak level
    // minus 3.01 dB (RMS), at unity.
    let (top, _) = kick_level(&mut s, &params, 1.0);
    assert!((top - (hi - 3.0103)).abs() < 0.05, "{top}");
    let (bottom, _) = kick_level(&mut s, &params, 0.0);
    assert!((bottom - (lo - 3.0103)).abs() < 0.05, "{bottom}");
}

/// The curve shapes the velocity before the layer pick, so a soft curve
/// makes the same hit reach a louder level.
#[test]
fn the_velocity_curve_moves_the_level_too() {
    let linear = multi();
    let soft = multi();
    soft.velocity_curve.set_value(1.0);
    let mut s = sampler();
    let (a, _) = kick_level(&mut s, &linear, 0.4);
    let (b, _) = kick_level(&mut s, &soft, 0.4);
    assert!(b > a + 3.0, "soft curve: {b} dB vs linear {a} dB");
}

/// Humanize: the same hits render the same on every run (the generator
/// is seeded, and `reset` re-seeds it); the same velocity played again
/// lands at different levels; and at 0 nothing moves.
#[test]
fn humanize_is_deterministic_and_moves_the_velocity() {
    let run = |amount: f32| -> Vec<f32> {
        let params = multi();
        params.velocity_humanize.set_value(amount);
        let mut s = sampler();
        (0..16)
            .map(|_| {
                // kick_level resets first: every hit is the first after a
                // reset, so re-seed by hand to walk the sequence instead.
                s.update_global_settings(&params);
                kick_level_no_reset(&mut s, &params, 0.5)
            })
            .collect()
    };
    let a = run(12.0);
    let b = run(12.0);
    assert_eq!(a, b, "humanize is reproducible");
    let spread = a.iter().cloned().fold(f32::NEG_INFINITY, f32::max)
        - a.iter().cloned().fold(f32::INFINITY, f32::min);
    // ±12 steps of 30 dB / 126 is up to ±2.9 dB.
    assert!(
        spread > 1.0,
        "humanize moves the level ({spread:.2} dB spread)"
    );
    assert!(spread < 6.0, "but within its range ({spread:.2} dB)");
    let flat = run(0.0);
    assert!(
        flat.windows(2).all(|w| w[0] == w[1]),
        "off: every hit the same"
    );

    // After a reset, the sequence starts over.
    let params = multi();
    params.velocity_humanize.set_value(12.0);
    let mut s = sampler();
    let first = kick_level(&mut s, &params, 0.5).0;
    let _ = kick_level_no_reset(&mut s, &params, 0.5);
    let again = kick_level(&mut s, &params, 0.5).0;
    assert_eq!(first, again);

    // A note no pad plays draws nothing from the generator: the next
    // kick is humanized as if it had not come.
    let mut s = sampler();
    s.update_global_settings(&params);
    let _ = kick_level(&mut s, &params, 0.5);
    let want = kick_level_no_reset(&mut s, &params, 0.5);
    let mut s = sampler();
    s.update_global_settings(&params);
    let _ = kick_level(&mut s, &params, 0.5);
    s.note_on(0, 0.5); // unmapped
    let got = kick_level_no_reset(&mut s, &params, 0.5);
    assert_eq!(got, want, "an unmapped note moved the humanize sequence");
}

/// [`kick_level`] without the reset (voices of earlier hits are long
/// done: every hit renders a whole strike window first).
fn kick_level_no_reset(sampler: &mut DrumSampler, params: &DrumParams, velocity: f32) -> f32 {
    // Let any earlier voice end: the strikes are two windows long.
    let frames = LOUDNESS_FRAMES;
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0; frames], vec![0.0; frames]))
        .collect();
    let mut render = |s: &mut DrumSampler| {
        let mut ports: Vec<PortBuffers<'_>> = bufs
            .iter_mut()
            .map(|(l, r)| PortBuffers {
                left: l.as_mut_slice(),
                right: r.as_mut_slice(),
            })
            .collect();
        s.render_block(&mut ports, frames, params, &[]);
    };
    render(sampler);
    render(sampler);
    sampler.note_on(drum_map::KICK, velocity);
    render(sampler);
    let close = &bufs[KICK_PORT].0;
    db((close.iter().map(|s| s * s).sum::<f32>() / frames as f32).sqrt())
}

#[test]
fn the_humanize_param_reads_and_parses() {
    let p = DrumParams::default();
    let h = &p.velocity_humanize;
    assert_eq!(h.id(), "velocity_humanize");
    assert_eq!(
        (h.min_plain(), h.max_plain(), h.default_plain()),
        (0.0, 20.0, 0.0)
    );
    assert_eq!(h.display(0.0), "Off");
    assert_eq!(h.display(5.0), "±5");
    assert_eq!(h.display(2.5), "±2.5");
    assert_eq!(h.parse("±7"), Some(7.0));
    assert_eq!(h.parse("off"), Some(0.0));
    assert_eq!(humanize_from_label("+3"), Some(3.0));
    assert_eq!(humanize_label(20.0), "±20");
}
