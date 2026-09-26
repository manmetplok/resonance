//! Per-band gain reduction: measured, published, drawn (ba todo #1319,
//! audit finding M5).
//!
//! Every band compressor already measured its own gain reduction and
//! nothing ever read it, so setting a per-band threshold was blind — the
//! user could not tell whether the band was compressing at all. These
//! tests cover the whole path: the stage exposes it, the audio thread
//! publishes it, and the meter turns it into something readable.

use resonance_mastering::stages::multiband::{BandConfig, Multiband, MultibandConfig, NUM_BANDS};
use resonance_mastering::viz::MasteringViz;
use resonance_mastering::ResonanceMastering;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;

fn sine_stereo(freq: f32, amp: f32, n: usize) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0f32; n];
    for (i, s) in l.iter_mut().enumerate() {
        *s = (i as f32 / SR * freq * std::f32::consts::TAU).sin() * amp;
    }
    let r = l.clone();
    (l, r)
}

// --- the stage measures it ----------------------------------------------

#[test]
fn an_idle_stage_reports_no_reduction_on_any_band() {
    let mb = Multiband::new(SR, 512);
    assert_eq!(mb.band_gr_db(), [0.0; NUM_BANDS]);
}

/// The point of the whole todo: a band that is compressing says so, and
/// the bands that are not stay silent — so the meter separates them.
#[test]
fn only_the_compressing_band_reports_reduction() {
    let mut cfg = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };
    cfg.bands[0] = BandConfig {
        enabled: true,
        threshold_db: -30.0,
        ratio: 8.0,
        attack_ms: 1.0,
        ..BandConfig::default()
    };

    let n = Multiband::latency_for(SR) + 12_000;
    let mut mb = Multiband::new(SR, n);
    let (mut l, mut r) = sine_stereo(50.0, 0.6, n);
    mb.process_stereo(&mut l, &mut r, &cfg);

    let gr = mb.band_gr_db();
    assert!(
        gr[0] > 1.0,
        "the compressing low band should report reduction, got {} dB",
        gr[0]
    );
    for (i, g) in gr.iter().enumerate().skip(1) {
        assert_eq!(*g, 0.0, "band {i} is not compressing but reports {g} dB");
    }
}

/// A lower threshold means more reduction — the reading tracks the
/// control it is there to give feedback about.
#[test]
fn a_lower_threshold_reports_more_reduction() {
    let measure = |threshold_db: f32| {
        let mut cfg = MultibandConfig {
            enabled: true,
            ..MultibandConfig::default()
        };
        cfg.bands[0] = BandConfig {
            enabled: true,
            threshold_db,
            ratio: 4.0,
            attack_ms: 1.0,
            ..BandConfig::default()
        };
        let n = Multiband::latency_for(SR) + 12_000;
        let mut mb = Multiband::new(SR, n);
        let (mut l, mut r) = sine_stereo(50.0, 0.6, n);
        mb.process_stereo(&mut l, &mut r, &cfg);
        mb.band_gr_db()[0]
    };

    let gentle = measure(-12.0);
    let hard = measure(-36.0);
    assert!(
        hard > gentle + 1.0,
        "threshold −36 dB reported {hard} dB, −12 dB reported {gentle} dB"
    );
}

// --- the audio thread publishes it --------------------------------------

#[test]
fn viz_round_trips_the_four_band_values() {
    let viz = MasteringViz::new();
    assert_eq!(viz.band_gr_db(), [0.0; NUM_BANDS]);
    viz.store_band_gr([1.5, 0.0, 7.25, 12.0]);
    assert_eq!(viz.band_gr_db(), [1.5, 0.0, 7.25, 12.0]);
}

/// End to end: running audio through the plugin with one band
/// compressing must leave the editor able to read that band's GR.
#[test]
fn processing_publishes_band_gr_to_the_editor() {
    let mut plugin = ResonanceMastering::new();
    plugin.initialize(SR, 4096);

    let p = plugin.params();
    p.multiband.on.set_value(true);
    p.multiband.bands[0].on.set_value(true);
    p.multiband.bands[0].threshold.set_value(-36.0);
    p.multiband.bands[0].ratio.set_value(8.0);
    p.multiband.bands[0].attack.set_value(1.0);

    let total = plugin.latency_samples() as usize + 24_000;
    stream_sine(&mut plugin, 50.0, 0.6, total, 512);

    let gr = plugin.viz().band_gr_db();
    assert!(
        gr[0] > 1.0,
        "band 0 GR never reached the editor (got {} dB)",
        gr[0]
    );
    assert_eq!(
        gr[1], 0.0,
        "band 1 is not compressing but published {} dB",
        gr[1]
    );
}

// --- the meter is readable ----------------------------------------------

mod meter {
    use resonance_mastering::editor::controls::gr_meter::{
        fill_fraction, is_active, readout, ACTIVE_THRESHOLD_DB, FULL_SCALE_DB,
    };

    #[test]
    fn an_idle_band_shows_an_empty_track_and_a_dash() {
        assert_eq!(fill_fraction(0.0), 0.0);
        assert!(!is_active(0.0));
        assert_eq!(readout(0.0), "—");
        // A hair of reduction is still "not compressing" — a one-pixel
        // sliver would read as activity that is not there.
        assert_eq!(fill_fraction(ACTIVE_THRESHOLD_DB), 0.0);
    }

    #[test]
    fn the_bar_scales_with_reduction_and_pins_at_full_scale() {
        let half = fill_fraction(FULL_SCALE_DB / 2.0);
        assert!((half - 0.5).abs() < 1e-6, "half scale filled {half}");
        assert_eq!(fill_fraction(FULL_SCALE_DB), 1.0);
        assert_eq!(fill_fraction(FULL_SCALE_DB * 4.0), 1.0);
        assert!(fill_fraction(2.0) < fill_fraction(6.0));
    }

    /// GR is published as a positive attenuation; the readout has to
    /// print it the way a user reads a gain-reduction meter.
    #[test]
    fn an_active_band_prints_signed_decibels() {
        assert_eq!(readout(3.4), "-3.4 dB");
        assert_eq!(readout(0.55), "-0.6 dB");
        assert_eq!(readout(12.0), "-12.0 dB");
        assert!(is_active(3.4));
    }

    /// A non-finite value must not blank the whole panel or draw a
    /// nonsense bar.
    #[test]
    fn garbage_reads_as_idle() {
        assert_eq!(fill_fraction(f32::NAN), 0.0);
        assert_eq!(fill_fraction(f32::INFINITY), 0.0);
        assert_eq!(fill_fraction(-5.0), 0.0);
    }
}

fn stream_sine(
    plugin: &mut ResonanceMastering,
    freq_hz: f32,
    amp: f32,
    total: usize,
    block: usize,
) {
    let step = freq_hz * std::f32::consts::TAU / SR;
    let mut phase = 0.0f32;
    let mut done = 0;
    while done < total {
        let n = block.min(total - done);
        let mut l = vec![0.0f32; n];
        let mut r = vec![0.0f32; n];
        for i in 0..n {
            let s = phase.sin() * amp;
            l[i] = s;
            r[i] = s;
            phase += step;
        }
        let mut outs = [OutputBuffer {
            left: &mut l,
            right: &mut r,
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        done += n;
    }
}
