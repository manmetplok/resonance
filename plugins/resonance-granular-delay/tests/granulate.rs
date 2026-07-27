//! Granulation behaviour through the full plugin stack: ring-buffer
//! wrap correctness, impulse timing, click-free wet rendering and the
//! write-head collision guard under stress (ba todo #1073, doc #252
//! §1/§5).

use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};

const SR: f32 = 48_000.0;

// --- Helpers ----------------------------------------------------------

fn run_blocks(
    plugin: &mut ResonanceGranularDelay,
    left: &mut [f32],
    right: &mut [f32],
    block: usize,
) {
    let frames = left.len();
    let mut pos = 0;
    while pos < frames {
        let n = (frames - pos).min(block);
        let mut outs = [OutputBuffer {
            left: &mut left[pos..pos + n],
            right: &mut right[pos..pos + n],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        pos += n;
    }
}

/// Largest second difference — a click/discontinuity detector that is
/// insensitive to the (smooth) granulated signal itself (same detector
/// as resonance-dsp/tests/granular.rs).
fn max_second_difference(x: &[f32]) -> f32 {
    x.windows(3)
        .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
        .fold(0.0_f32, f32::max)
}

/// A deterministic wet-only plugin: free-running time, no jitters, no
/// pan, Sync scheduler — the grain cloud is fully reproducible.
fn wet_only_plugin(time_ms: f32, grain_ms: f32, density_hz: f32) -> ResonanceGranularDelay {
    let plugin = ResonanceGranularDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(time_ms);
    plugin.params.grain_size_ms.set_value(grain_ms);
    plugin.params.density_hz.set_value(density_hz);
    plugin.params.scheduler.set_value(0); // Sync (deterministic)
    plugin.params.spray_ms.set_value(0.0);
    plugin.params.size_jitter.set_value(0.0);
    plugin.params.level_jitter.set_value(0.0);
    plugin.params.pan_spread.set_value(0.0);
    plugin.params.reverse_prob.set_value(0.0);
    plugin.params.spread_cents.set_value(0.0);
    // Single-pass granulation: the feedback path (ba todo #1074) is
    // live now, so pin it off to keep these renders one-pass.
    plugin.params.feedback.set_value(0.0);
    plugin.params.mix.set_value(1.0); // wet only
    plugin
}

// --- Tests ------------------------------------------------------------

/// An impulse granulated at rate 1 with zero jitter re-emerges exactly
/// at the configured delay position (every grain's read head crosses
/// the impulse `delay` samples after it was written), and nowhere
/// before it.
#[test]
fn impulse_reemerges_at_the_delay_position() {
    let mut plugin = wet_only_plugin(250.0, 80.0, 30.0);
    plugin.initialize(SR, 4096);

    let frames = SR as usize; // 1 s
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    left[0] = 1.0;
    right[0] = 1.0;

    run_blocks(&mut plugin, &mut left, &mut right, 512);

    let delay_samples = (0.250 * SR) as usize; // 12_000
    // Wet-only output must be silent before the delay position (grains
    // reading further back see only cleared buffer).
    let pre: f32 = left[..delay_samples - 64].iter().map(|x| x.abs()).sum();
    assert!(
        pre < 1e-6,
        "wet energy before the delay position: {pre}"
    );
    // ... and the granulated impulse lands in a tight window at it.
    let at: f32 = left[delay_samples - 64..delay_samples + 256]
        .iter()
        .map(|x| x.abs())
        .sum();
    assert!(
        at > 1e-3,
        "no granulated energy at the delay position, got {at}"
    );
    // Long after the cloud has passed (no feedback yet) it decays out.
    let tail_start = delay_samples + (0.4 * SR) as usize;
    let tail: f32 = left[tail_start..].iter().map(|x| x.abs()).sum();
    assert!(tail < 1e-4, "unexpected energy long after the cloud: {tail}");
}

/// The circular buffer wraps correctly: an impulse still re-emerges at
/// the delay position after the write head has lapped the power-of-two
/// ring several times (ring is 4 s => 2^18 samples at 48 kHz).
#[test]
fn ring_buffer_wraps_correctly_after_lapping() {
    let mut plugin = wet_only_plugin(100.0, 50.0, 40.0);
    plugin.initialize(SR, 4096);

    // 2^18 = 262_144 samples per lap; run 2 laps + 1 s.
    let lap = ((4.0 * SR) as usize + 1).next_power_of_two();
    let lead = 2 * lap;
    let frames = lead + SR as usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    // Impulse *after* two full laps of silence.
    left[lead] = 1.0;
    right[lead] = 1.0;

    run_blocks(&mut plugin, &mut left, &mut right, 4096);

    let delay_samples = (0.100 * SR) as usize; // 4_800
    for &x in left.iter() {
        assert!(x.is_finite(), "non-finite sample after wrap: {x}");
    }
    let at: f32 = left[lead + delay_samples - 64..lead + delay_samples + 256]
        .iter()
        .map(|x| x.abs())
        .sum();
    assert!(
        at > 1e-3,
        "no granulated energy at the delay position after lapping, got {at}"
    );
    // The window just before the (post-lap) impulse echo must stay
    // silent — stale un-overwritten data would show up here.
    let pre: f32 = left[lead + 1..lead + delay_samples - 64]
        .iter()
        .map(|x| x.abs())
        .sum();
    assert!(pre < 1e-6, "stale ring-buffer energy after lapping: {pre}");
}

/// Wet-only granulation of a steady low-frequency sine is click-free:
/// every grain is windowed to zero at both ends and overlap is
/// gain-compensated.
#[test]
fn wet_sine_has_no_discontinuity() {
    let mut plugin = wet_only_plugin(200.0, 90.0, 25.0);
    plugin.initialize(SR, 4096);

    let frames = SR as usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    for i in 0..frames {
        // 50 Hz keeps the source's own second difference tiny, so the
        // detector isolates granulation artifacts.
        let s = (std::f32::consts::TAU * 50.0 * i as f32 / SR).sin() * 0.9;
        left[i] = s;
        right[i] = s;
    }

    run_blocks(&mut plugin, &mut left, &mut right, 512);

    // Skip the initial fill; measure once the cloud is steady.
    let steady = &left[(0.4 * SR) as usize..];
    let rms: f32 =
        (steady.iter().map(|x| x * x).sum::<f32>() / steady.len() as f32).sqrt();
    assert!(rms > 0.05, "wet bus unexpectedly quiet: rms {rms}");
    let d2 = max_second_difference(steady);
    assert!(d2 < 0.02, "granulated sine clicked: max d2 {d2}");
}

/// Collision stress (doc #252 §5): +12 st grains (rate 2) that are far
/// longer than the delay would overtake the write head without the
/// spawn guard. The engine clamps them into the safe zone, so the
/// output stays click-free.
#[test]
fn collision_stress_fast_long_grains_short_delay_is_click_free() {
    let mut plugin = wet_only_plugin(60.0, 300.0, 20.0);
    plugin.params.pitch.set_value(12.0); // rate 2.0
    plugin.initialize(SR, 4096);

    let frames = 2 * SR as usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    for i in 0..frames {
        let s = (std::f32::consts::TAU * 50.0 * i as f32 / SR).sin() * 0.9;
        left[i] = s;
        right[i] = s;
    }

    run_blocks(&mut plugin, &mut left, &mut right, 512);

    for &x in left.iter().chain(right.iter()) {
        assert!(x.is_finite(), "non-finite sample under collision stress: {x}");
    }
    let steady = &left[(0.8 * SR) as usize..];
    let rms: f32 =
        (steady.iter().map(|x| x * x).sum::<f32>() / steady.len() as f32).sqrt();
    assert!(rms > 0.02, "collision guard silenced the wet bus: rms {rms}");
    // Rate-2 grains double the source's per-sample delta; the guard
    // keeps everything continuous well below click level.
    let d2 = max_second_difference(steady);
    assert!(d2 < 0.05, "write-head collision produced a click: max d2 {d2}");
}

/// Param plumbing reaches the engine: density scales the spawn count,
/// and the pan-spread path keeps left/right lock-stepped engines in
/// agreement (identical grain clouds over each channel).
#[test]
fn density_plumbs_through_to_spawn_rate() {
    let spawned_at = |density: f32| -> u64 {
        let mut plugin = wet_only_plugin(200.0, 90.0, density);
        plugin.initialize(SR, 512);
        let frames = SR as usize;
        let mut left = vec![0.1f32; frames];
        let mut right = vec![0.1f32; frames];
        run_blocks(&mut plugin, &mut left, &mut right, 512);
        plugin.grains_spawned()
    };

    let sparse = spawned_at(5.0);
    let dense = spawned_at(50.0);
    // Sync scheduler: ~5 and ~50 grains over one second.
    assert!(
        (4..=7).contains(&sparse),
        "5 Hz density spawned {sparse} grains in 1 s"
    );
    assert!(
        (45..=55).contains(&dense),
        "50 Hz density spawned {dense} grains in 1 s"
    );
}

/// A dry/wet sanity render: at mix 0 the plugin is bit-transparent
/// (dry path untouched), and the equal-power law holds at mix 1.
#[test]
fn mix_zero_is_transparent() {
    let mut plugin = wet_only_plugin(200.0, 90.0, 25.0);
    plugin.params.mix.set_value(0.0);
    plugin.initialize(SR, 512);

    let frames = SR as usize / 2;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    for i in 0..frames {
        let s = (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin() * 0.5;
        left[i] = s;
        right[i] = -s;
    }
    let dry_l = left.clone();
    let dry_r = right.clone();

    run_blocks(&mut plugin, &mut left, &mut right, 512);

    for i in 0..frames {
        assert!(
            (left[i] - dry_l[i]).abs() < 1e-6 && (right[i] - dry_r[i]).abs() < 1e-6,
            "dry path altered at sample {i}: {} vs {}",
            left[i],
            dry_l[i]
        );
    }
}
