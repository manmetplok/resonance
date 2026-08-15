//! Tempo-locked grain density (ba todo #1322): with PER-BEAT on, one
//! grain is spawned per selected division of the host tempo, the cloud
//! re-locks on every tempo change, and with it off nothing about the
//! free-running path moves.

use resonance_granular_delay::params::{DENSITY_MAX_HZ, DENSITY_MIN_HZ};
use resonance_granular_delay::presets::{load_preset, PRESETS};
use resonance_granular_delay::sync;
use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin, TempoInfo};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

/// Divisions of the shared table (see `crate::sync`).
const D_1_4: usize = 4;
const D_1_8: usize = 7;
const D_1_8T: usize = 9;
const D_1_16: usize = 10;

fn tempo(bpm: f32) -> TempoInfo {
    TempoInfo {
        bpm,
        time_sig_num: 4,
        time_sig_den: 4,
        playing: true,
        song_pos_beats: 0.0,
    }
}

/// Deterministic, wet-forward plugin: Sync scheduler so onsets are
/// exactly periodic and the spawn count is a direct measurement of the
/// grain rate.
fn plugin() -> ResonanceGranularDelay {
    let mut p = ResonanceGranularDelay::new();
    p.params.sync.set_plain(0.0);
    p.params.time_ms.set_value(60.0);
    p.params.grain_size_ms.set_value(40.0);
    p.params.density_hz.set_value(22.0);
    p.params.scheduler.set_value(0); // Sync
    p.params.mix.set_value(1.0);
    p.params.feedback.set_value(0.0);
    p.params.spray_ms.set_value(0.0);
    p.params.size_jitter.set_value(0.0);
    p.params.level_jitter.set_value(0.0);
    p.params.reverse_prob.set_value(0.0);
    p.params.pan_spread.set_value(0.0);
    p.initialize(SR, BLOCK as u32);
    p
}

/// Render `seconds` of a steady tone at `at`, returning the wet output
/// and how many grains were spawned during it.
fn render(p: &mut ResonanceGranularDelay, at: Option<TempoInfo>, seconds: f32) -> (Vec<f32>, u64) {
    let before = p.grains_spawned();
    let blocks = (seconds * SR / BLOCK as f32).round() as usize;
    let mut out = Vec::with_capacity(blocks * BLOCK);
    let mut left = [0.0f32; BLOCK];
    let mut right = [0.0f32; BLOCK];
    let mut n = 0u64;
    for _ in 0..blocks {
        for i in 0..BLOCK {
            let t = (n + i as u64) as f32 / SR;
            let v = 0.4 * (330.0 * t * std::f32::consts::TAU).sin();
            left[i] = v;
            right[i] = v;
        }
        {
            let mut outs = [OutputBuffer {
                left: &mut left[..],
                right: &mut right[..],
            }];
            let mut ev = EventIterator::empty();
            p.process(&mut outs, BLOCK, &mut ev, at);
        }
        n += BLOCK as u64;
        out.extend_from_slice(&left);
    }
    (out, p.grains_spawned() - before)
}

/// Measured grain rate over `seconds`, grains per second.
fn measure_hz(p: &mut ResonanceGranularDelay, at: Option<TempoInfo>, seconds: f32) -> f32 {
    let (_, spawned) = render(p, at, seconds);
    spawned as f32 / seconds
}

// --- The rate table itself -------------------------------------------

/// One grain per division: the arithmetic the DSP locks to, pinned
/// against hand-computed values at a known tempo.
#[test]
fn tempo_locked_rate_matches_the_division_table() {
    // 120 BPM: a beat is 500 ms.
    for (division, want) in [
        (D_1_4, 2.0),    // quarter note  -> 2 grains/s
        (D_1_8, 4.0),    // eighth        -> 4
        (D_1_8T, 6.0),   // eighth triplet-> 6
        (D_1_16, 8.0),   // sixteenth     -> 8
    ] {
        let got = sync::density_hz(120.0, division);
        assert!(
            (got - want).abs() < 1e-3,
            "division {division} at 120 BPM: {got} /s, expected {want}"
        );
    }
    // Doubling the tempo doubles the rate.
    assert!((sync::density_hz(240.0, D_1_8) - 8.0).abs() < 1e-3);
}

/// A crawling or runaway host tempo can never push the cloud outside
/// the range the Density knob itself declares.
#[test]
fn the_locked_rate_stays_inside_the_declared_density_range() {
    assert_eq!(sync::density_hz(1.0, 0), DENSITY_MIN_HZ);
    assert_eq!(sync::density_hz(100_000.0, 11), DENSITY_MAX_HZ);
}

/// Without a host tempo there is nothing to lock to, so the free
/// running knob still wins.
#[test]
fn without_a_host_tempo_the_density_knob_wins() {
    assert_eq!(sync::grain_density_hz(true, D_1_8, 22.0, None), 22.0);
    assert_eq!(sync::grain_density_hz(false, D_1_8, 22.0, Some(tempo(120.0))), 22.0);
    assert_eq!(sync::grain_density_hz(true, D_1_8, 22.0, Some(tempo(120.0))), 4.0);
}

// --- End to end through the plugin ------------------------------------

/// The measurement the todo asked for: with density_sync on, the grain
/// interval the engine actually produces is the division's, at a known
/// tempo — not the Density knob's 22 /s.
#[test]
fn the_engine_spawns_one_grain_per_division() {
    let mut p = plugin();
    p.params.density_sync.set_value(true);
    p.params.density_division.set_value(D_1_16 as i32);
    // 150 BPM: a beat is 400 ms, a sixteenth 100 ms -> 10 grains/s.
    let hz = measure_hz(&mut p, Some(tempo(150.0)), 4.0);
    assert!(
        (hz - 10.0).abs() <= 0.5,
        "expected ~10 grains/s at 150 BPM 1/16, measured {hz}"
    );
}

/// "Stays locked through tempo changes": the rate follows the host
/// without any parameter moving.
#[test]
fn the_grain_rate_relocks_when_the_tempo_changes() {
    let mut p = plugin();
    p.params.density_sync.set_value(true);
    p.params.density_division.set_value(D_1_8 as i32);

    let slow = measure_hz(&mut p, Some(tempo(90.0)), 4.0); // 3 /s
    let fast = measure_hz(&mut p, Some(tempo(180.0)), 4.0); // 6 /s
    assert!(
        (slow - 3.0).abs() <= 0.5,
        "expected ~3 grains/s at 90 BPM 1/8, measured {slow}"
    );
    assert!(
        (fast - 6.0).abs() <= 0.5,
        "expected ~6 grains/s at 180 BPM 1/8, measured {fast}"
    );
    assert!(
        fast > slow * 1.5,
        "the cloud did not re-lock: {slow} /s -> {fast} /s"
    );
}

/// With PER-BEAT off nothing changes — not the rate, and not a single
/// sample — however the density division is set. This is the guarantee
/// existing projects and presets rely on.
#[test]
fn density_sync_off_is_unchanged() {
    let mut off = plugin();
    let (a, spawned_a) = render(&mut off, Some(tempo(120.0)), 2.0);

    let mut other_division = plugin();
    other_division.params.density_division.set_value(D_1_4 as i32);
    let (b, spawned_b) = render(&mut other_division, Some(tempo(120.0)), 2.0);

    assert_eq!(spawned_a, spawned_b);
    assert_eq!(
        a.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        b.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        "the density division must not touch the free-running path"
    );
    // And the free-running rate is still the knob's.
    let hz = spawned_a as f32 / 2.0;
    assert!(
        (hz - 22.0).abs() <= 1.0,
        "free-running density should be the knob's 22 /s, measured {hz}"
    );
}

/// The shipped preset lives up to its name: loading "Eighth-Triplet
/// Echo" and running it at 120 BPM granulates in eighth triplets (6
/// grains/s), on a delay tap that is also 1/8T.
#[test]
fn the_eighth_triplet_preset_produces_eighth_triplets() {
    let entry = PRESETS
        .iter()
        .find(|e| e.name == "Eighth-Triplet Echo")
        .expect("the eighth-triplet preset is missing");

    let mut p = ResonanceGranularDelay::new();
    assert!(load_preset(&p.params, entry.json));
    assert!(p.params.density_sync.value(), "the preset must lock density");
    assert_eq!(p.params.density_division.value(), D_1_8T as i32);
    assert_eq!(p.params.division.value(), D_1_8T as i32);

    p.params.scheduler.set_value(0); // Sync, so the count is exact
    p.params.mix.set_value(1.0);
    p.initialize(SR, BLOCK as u32);
    let hz = measure_hz(&mut p, Some(tempo(120.0)), 6.0);
    assert!(
        (hz - 6.0).abs() <= 0.4,
        "'Eighth-Triplet Echo' granulates at {hz} /s at 120 BPM, expected ~6 (1/8T)"
    );
}
