//! The two plots and the meter feed: the transfer curve is the mode's
//! static curve (identity where the DSP is transparent, odd or even
//! where the harmonics say so), the harmonic bars come off the same
//! probe the tests pin, and the audio thread publishes its levels.

use std::time::{Duration, Instant};

use resonance_color::dsp::voicing::transfer;
use resonance_color::dsp::Settings;
use resonance_color::editor::curve::{curve_points, CurveCache, NUM_POINTS};
use resonance_color::editor::harmonics::{bar_fraction, ProbeCache, FLOOR_DBC};
use resonance_color::params::Mode;
use resonance_color::probe::{probe, PROBE_LEVEL_DBFS};

#[test]
fn the_curve_is_the_identity_where_the_dsp_is_transparent() {
    for i in 0..=20 {
        let x = -1.0 + i as f32 / 10.0;
        assert_eq!(transfer(Mode::Console, 0.0, 0.5, 1.0, x), x, "Console drive 0");
        for mode in Mode::ALL {
            let y = transfer(mode, 1.0, 1.0, 0.0, x);
            assert!((y - x).abs() < 1e-6, "{} at mix 0", mode.label());
        }
    }
}

#[test]
fn the_curve_has_the_symmetry_its_harmonics_say() {
    for i in 1..=10 {
        let x = i as f32 / 10.0;
        // Console: odd, f(−x) = −f(x).
        let c = |x| transfer(Mode::Console, 0.8, 0.5, 1.0, x);
        assert!((c(-x) + c(x)).abs() < 1e-6);
        // Warm: x plus an even term, so f(x) + f(−x) = 2·even(x) > 0.
        let w = |x| transfer(Mode::Warm, 0.8, 0.8, 1.0, x);
        assert!(w(x) + w(-x) > 0.0);
        assert!(((w(x) - w(-x)) / 2.0 - x).abs() < 1e-6, "Warm's odd part is the identity");
    }
}

#[test]
fn the_curve_is_monotonic_with_unity_slope_at_the_origin() {
    for mode in Mode::ALL {
        let s = Settings {
            mode,
            drive: 0.7,
            bias: 0.5,
            ..Settings::default()
        };
        let pts = curve_points(&s);
        assert_eq!(pts.len(), NUM_POINTS);
        // Non-decreasing: Console's `sin` curve is flat past its peak.
        assert!(pts.windows(2).all(|w| w[1].1 >= w[0].1), "{} is not monotonic", mode.label());
        let h = 1e-3f32;
        let slope = (transfer(mode, 0.7, 0.5, 1.0, h) - transfer(mode, 0.7, 0.5, 1.0, -h)) / (2.0 * h);
        assert!((slope - 1.0).abs() < 0.01, "{} small-signal slope {slope}", mode.label());
    }
}

#[test]
fn the_curve_cache_follows_the_settings_it_depends_on() {
    let mut cache = CurveCache::default();
    let a = Settings::default();
    let first = cache.points(&a).to_vec();
    // Tone does not move the curve.
    assert_eq!(cache.points(&Settings { tone_db: 3.0, ..a }), &first[..]);
    // Drive does.
    assert_ne!(cache.points(&Settings { drive: 0.9, ..a }), &first[..]);
}

/// Poll the cache the way `ui()` does, once a millisecond, until it shows
/// the probe for `s`.
fn wait_for(cache: &mut ProbeCache, s: &Settings) -> resonance_color::probe::HarmonicSignature {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let sig = cache.signature(s);
        if cache.is_current(s) {
            return sig.expect("a current probe has a signature");
        }
        assert!(Instant::now() < deadline, "the probe worker never answered");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn the_bars_read_the_probe() {
    let s = Settings {
        mode: Mode::Tube,
        drive: 0.5,
        ..Settings::default()
    };
    let mut cache = ProbeCache::default();
    let shown = wait_for(&mut cache, &s);
    assert_eq!(shown, probe(&s, PROBE_LEVEL_DBFS));
    // Auto-gain and flutter are not part of what the probe measures, so
    // toggling them re-uses the result.
    let same = Settings { auto_gain: false, flutter: 0.3, ..s };
    assert!(cache.is_current(&same));
    assert_eq!(cache.signature(&same), Some(shown));
    assert_eq!(bar_fraction(0.0), 1.0);
    assert_eq!(bar_fraction(FLOOR_DBC), 0.0);
    assert_eq!(bar_fraction(-300.0), 0.0);
}

/// `ui()` never waits for a probe: the call that requests one returns at
/// once, with the previous result (none yet, the first time), and the
/// bars catch up on a later frame.
#[test]
fn asking_for_a_probe_never_blocks_the_caller() {
    // Tape HQ at 4x is the slowest probe there is.
    let s = Settings {
        mode: Mode::Tape,
        drive: 0.8,
        oversample: resonance_dsp::OversampleFactor::X4,
        tape_quality: resonance_color::params::TapeQuality::Hq,
        ..Settings::default()
    };
    let mut cache = ProbeCache::default();
    assert_eq!(cache.signature(&s), None, "the first call waited for the probe");
    let first = wait_for(&mut cache, &s);
    // A change keeps showing the old bars until the new probe lands.
    let moved = Settings { drive: 0.2, ..s };
    assert_eq!(cache.signature(&moved), Some(first), "the retarget waited for the probe");
    let second = wait_for(&mut cache, &moved);
    assert_ne!(first, second);
    assert_eq!(second, probe(&moved, PROBE_LEVEL_DBFS));
}

/// Closing the editor drops the cache with a probe in flight: the worker
/// must stop and join promptly, not hang the GUI thread's teardown.
#[test]
fn dropping_the_cache_mid_probe_joins_the_worker() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut cache = ProbeCache::default();
        let hq = Settings {
            mode: Mode::Tape,
            drive: 0.8,
            oversample: resonance_dsp::OversampleFactor::X4,
            tape_quality: resonance_color::params::TapeQuality::Hq,
            ..Settings::default()
        };
        let _ = cache.signature(&hq);
        // Queue a second request behind the one running.
        let _ = cache.signature(&Settings { drive: 0.1, ..hq });
        drop(cache);
        let _ = tx.send(());
    });
    rx.recv_timeout(Duration::from_secs(20))
        .expect("dropping the probe cache hung (the worker never joined)");
}

#[test]
fn the_audio_thread_publishes_levels_to_the_editor() {
    use resonance_color::dsp::ColorDsp;
    use resonance_color::viz::ColorViz;
    let viz = ColorViz::new();
    assert_eq!(viz.input_db(), f32::NEG_INFINITY, "silent before the first block");
    let s = Settings {
        mode: Mode::Tube,
        drive: 1.0,
        ..Settings::default()
    };
    let mut dsp = ColorDsp::new(48_000.0, &s);
    for block in 0..40 {
        let mut l: Vec<f32> = (0..256)
            .map(|i| {
                let t = (block * 256 + i) as f32 / 48_000.0;
                0.5 * (std::f32::consts::TAU * 440.0 * t).sin()
            })
            .collect();
        let mut r = l.clone();
        dsp.process(&mut l, &mut r, &s, Some(&viz));
    }
    // A 0.5 peak sine: the input meter reads its peak (−6 dBFS).
    assert!((viz.input_db() + 6.02).abs() < 0.1, "input meter {}", viz.input_db());
    assert!(viz.output_db().is_finite());
    // Full drive squashes the peaks, so auto-gain is lifting the wet path.
    assert!(viz.auto_gain_db() > 0.5, "auto-gain readout {}", viz.auto_gain_db());
    assert_eq!(viz.auto_gain_db(), dsp.auto_gain_db());
}
