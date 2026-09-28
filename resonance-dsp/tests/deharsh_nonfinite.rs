//! The de-harsh suppressor recovers from non-finite input (review finding
//! M5). One NaN used to latch a full-depth cut in every band bin forever:
//! the detector's smoothed power went NaN and never came back, and a NaN
//! level compared as "over the reference" and clamped to the depth.
//!
//! Its own binary: `tests/deharsh.rs` carries a counting global
//! allocator.

use resonance_dsp::{ResonanceSuppressor, SimpleRng, SuppressorConfig, SuppressorMode};

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;

fn cfg(mode: SuppressorMode) -> SuppressorConfig {
    SuppressorConfig {
        enabled: true,
        depth_db: 12.0,
        mode,
        ..SuppressorConfig::default()
    }
}

/// White noise at about −15 dBFS RMS: broadband, so a working detector
/// cuts next to nothing.
fn noise(n: usize) -> Vec<f32> {
    let mut rng = SimpleRng::new(7);
    (0..n)
        .map(|_| 0.3 * (rng.next_u32() as f32 / u32::MAX as f32 * 2.0 - 1.0))
        .collect()
}

#[test]
fn a_single_nan_does_not_latch_a_cut() {
    for bad in [f32::NAN, f32::INFINITY] {
        for mode in [
            SuppressorMode::Stereo,
            SuppressorMode::Mid,
            SuppressorMode::Side,
            SuppressorMode::MidSide,
        ] {
            let cfg = cfg(mode);
            let mut sup = ResonanceSuppressor::new(SR);
            let len = 4 * 48_000;
            let mut l = noise(len);
            let mut r: Vec<f32> = l.iter().rev().copied().collect();
            let at = 24_000;
            l[at] = bad;
            r[at] = bad;
            let mut max_cut_after = 0.0f32;
            let settled = at + sup.latency() + 48_000;
            let mut start = 0;
            while start < len {
                let end = (start + BLOCK).min(len);
                sup.process_stereo(&mut l[start..end], &mut r[start..end], &cfg);
                if end > settled {
                    max_cut_after = max_cut_after.max(sup.max_cut_db());
                }
                start = end;
            }
            // A second of clean audio after the bad sample left the
            // history: the cut is back to what broadband noise gets.
            assert!(
                max_cut_after < 3.0,
                "{bad} in {mode:?}: cut still {max_cut_after:.1} dB a second later"
            );
            let tail = &l[settled..];
            assert!(tail.iter().all(|x| x.is_finite()), "{bad} in {mode:?}: output stays non-finite");
            assert!(tail.iter().any(|&x| x.abs() > 0.05), "{bad} in {mode:?}: output went silent");
        }
    }
}
