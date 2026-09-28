use resonance_mastering::assistant::analyze::AnalysisResult;
use resonance_mastering::assistant::decide::{
    bins_for_range, build, Target, HIGH_BAND_HZ, LOW_BAND_HZ,
};
use resonance_mastering::assistant::targets::{target_band, target_curve, Genre};

fn dummy_analysis(crest_db: f32, spectrum: Vec<f32>) -> AnalysisResult {
    AnalysisResult {
        sample_rate: 48_000.0,
        duration_s: 10.0,
        integrated_lufs: -14.0,
        short_term_lufs: -14.0,
        true_peak_dbtp: -1.0,
        crest_db,
        correlation: 0.8,
        spectrum_db: spectrum,
    }
}

fn on_target() -> Vec<f32> {
    target_curve(Genre::Rock).to_vec()
}

#[test]
fn high_crest_enables_gentle_glue() {
    let a = dummy_analysis(18.0, on_target());
    let s = build(&a, &Target::Genre(Genre::Rock));
    assert!(s.glue_enabled);
    assert!((s.glue_ratio - 2.0).abs() < 1e-6);
    assert!(s.glue_makeup_db > 0.0, "expected positive makeup gain");
}

#[test]
fn low_crest_disables_glue() {
    let a = dummy_analysis(7.0, on_target());
    let s = build(&a, &Target::Genre(Genre::Rock));
    assert!(!s.glue_enabled);
}

/// A spectrum on the band's midline is on target everywhere: no shelf,
/// and every 1/3-octave deviation is 0.
#[test]
fn a_spectrum_on_the_midline_moves_no_shelf() {
    let a = dummy_analysis(14.0, on_target());
    let s = build(&a, &Target::Genre(Genre::Rock));
    assert_eq!(s.tonal_low_shelf_gain_db, 0.0);
    assert_eq!(s.tonal_high_shelf_gain_db, 0.0);
    assert_eq!(s.deviations.len(), 31);
    for d in &s.deviations {
        assert_eq!(d.deviation_db, 0.0, "{d:?}");
        assert!(d.lo_db < d.measured_db && d.measured_db < d.hi_db, "{d:?}");
    }
}

/// Anywhere inside the band is on target: pushing the low band to just
/// under the band's top edge still suggests nothing.
#[test]
fn a_spectrum_inside_the_band_moves_no_shelf() {
    let (_, hi) = target_band(Genre::Rock);
    let mut analyzed = on_target();
    let (lo_bin, hi_bin) = bins_for_range(LOW_BAND_HZ);
    for i in lo_bin..hi_bin {
        analyzed[i] = hi[i] - 0.1;
    }
    let a = dummy_analysis(14.0, analyzed);
    let s = build(&a, &Target::Genre(Genre::Rock));
    assert_eq!(s.tonal_low_shelf_gain_db, 0.0);
}

/// Only the part outside the band drives the shelf: a low band 2 dB over
/// the band's top edge asks for a 2 dB cut, not for the whole distance to
/// the midline.
#[test]
fn bass_heavy_input_cuts_only_the_excess_over_the_band() {
    let (_, hi) = target_band(Genre::Rock);
    let mut analyzed = on_target();
    let (lo_bin, hi_bin) = bins_for_range(LOW_BAND_HZ);
    for i in lo_bin..hi_bin {
        analyzed[i] = hi[i] + 2.0;
    }
    let a = dummy_analysis(14.0, analyzed);
    let s = build(&a, &Target::Genre(Genre::Rock));
    assert!(
        (s.tonal_low_shelf_gain_db - -2.0).abs() < 1e-3,
        "expected a 2 dB low-shelf cut, got {}",
        s.tonal_low_shelf_gain_db
    );
    let sub = s
        .deviations
        .iter()
        .find(|d| (d.center_hz - 50.0).abs() < 2.0)
        .unwrap();
    assert!((sub.deviation_db - 2.0).abs() < 0.01, "{sub:?}");
}

#[test]
fn dim_top_boosts_only_the_shortfall_under_the_band() {
    let (lo, _) = target_band(Genre::Rock);
    let mut analyzed = on_target();
    let (lo_bin, hi_bin) = bins_for_range(HIGH_BAND_HZ);
    for i in lo_bin..hi_bin {
        analyzed[i] = lo[i] - 3.0;
    }
    let a = dummy_analysis(14.0, analyzed);
    let s = build(&a, &Target::Genre(Genre::Rock));
    assert!(
        (s.tonal_high_shelf_gain_db - 3.0).abs() < 1e-3,
        "expected a 3 dB high-shelf boost, got {}",
        s.tonal_high_shelf_gain_db
    );
}

/// The comparison is of shape, not level: the whole spectrum 20 dB down
/// still reads on target.
#[test]
fn absolute_level_does_not_matter() {
    let analyzed: Vec<f32> = on_target().iter().map(|v| v - 20.0).collect();
    let a = dummy_analysis(14.0, analyzed);
    let s = build(&a, &Target::Genre(Genre::Rock));
    assert_eq!(s.tonal_low_shelf_gain_db, 0.0);
    assert_eq!(s.tonal_high_shelf_gain_db, 0.0);
}

#[test]
fn quiet_input_suggests_positive_trim() {
    let mut a = dummy_analysis(14.0, on_target());
    a.integrated_lufs = -34.0;
    let s = build(&a, &Target::Genre(Genre::Rock));
    // Target is -11 LUFS, input is -34 → gap of 23 LU → trim ~20 dB
    assert!(
        s.input_trim_db > 15.0,
        "expected large positive trim, got {}",
        s.input_trim_db
    );
}

#[test]
fn narrow_stereo_suggests_widening() {
    let mut a = dummy_analysis(14.0, on_target());
    a.correlation = 0.95;
    let s = build(&a, &Target::Genre(Genre::Rock));
    assert!(s.imager_enabled);
    assert!(
        s.imager_width > 1.0,
        "expected widening, got {}",
        s.imager_width
    );
}

#[test]
fn very_wide_stereo_suggests_narrowing() {
    let mut a = dummy_analysis(14.0, on_target());
    a.correlation = 0.1;
    let s = build(&a, &Target::Genre(Genre::Rock));
    assert!(s.imager_enabled);
    assert!(
        s.imager_width < 1.0,
        "expected narrowing, got {}",
        s.imager_width
    );
}
