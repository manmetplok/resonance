use resonance_mastering::assistant::analyze::{AnalysisResult, NUM_SPECTRUM_BINS};
use resonance_mastering::assistant::decide::{
    bins_for_range, build, Target, HIGH_BAND_HZ, LOW_BAND_HZ,
};
use resonance_mastering::assistant::targets::{target_curve, Genre};

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

#[test]
fn high_crest_enables_gentle_glue() {
    let flat = vec![0.0_f32; NUM_SPECTRUM_BINS];
    let a = dummy_analysis(18.0, flat);
    let s = build(&a, &Target::Genre(Genre::Rock));
    assert!(s.glue_enabled);
    assert!((s.glue_ratio - 2.0).abs() < 1e-6);
    assert!(s.glue_makeup_db > 0.0, "expected positive makeup gain");
}

#[test]
fn low_crest_disables_glue() {
    let flat = vec![0.0_f32; NUM_SPECTRUM_BINS];
    let a = dummy_analysis(7.0, flat);
    let s = build(&a, &Target::Genre(Genre::Rock));
    assert!(!s.glue_enabled);
}

#[test]
fn bass_heavy_input_suggests_low_shelf_cut() {
    // Build an analyzed spectrum that is +6 dB louder than target
    // in the low band. Expect a negative low-shelf gain suggestion.
    let target = target_curve(Genre::Rock);
    let mut analyzed: Vec<f32> = target.to_vec();
    let (lo, hi) = bins_for_range(LOW_BAND_HZ);
    for v in &mut analyzed[lo..hi] {
        *v += 6.0;
    }
    let a = dummy_analysis(14.0, analyzed);
    let s = build(&a, &Target::Genre(Genre::Rock));
    assert!(
        s.tonal_low_shelf_gain_db < -2.0,
        "expected a low-shelf cut, got {}",
        s.tonal_low_shelf_gain_db
    );
}

#[test]
fn dim_top_suggests_high_shelf_boost() {
    let target = target_curve(Genre::Rock);
    let mut analyzed: Vec<f32> = target.to_vec();
    let (lo, hi) = bins_for_range(HIGH_BAND_HZ);
    for v in &mut analyzed[lo..hi] {
        *v -= 6.0;
    }
    let a = dummy_analysis(14.0, analyzed);
    let s = build(&a, &Target::Genre(Genre::Rock));
    assert!(
        s.tonal_high_shelf_gain_db > 2.0,
        "expected a high-shelf boost, got {}",
        s.tonal_high_shelf_gain_db
    );
}

#[test]
fn quiet_input_suggests_positive_trim() {
    let flat = vec![0.0_f32; NUM_SPECTRUM_BINS];
    let mut a = dummy_analysis(14.0, flat);
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
    let flat = vec![0.0_f32; NUM_SPECTRUM_BINS];
    let mut a = dummy_analysis(14.0, flat);
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
    let flat = vec![0.0_f32; NUM_SPECTRUM_BINS];
    let mut a = dummy_analysis(14.0, flat);
    a.correlation = 0.1;
    let s = build(&a, &Target::Genre(Genre::Rock));
    assert!(s.imager_enabled);
    assert!(
        s.imager_width < 1.0,
        "expected narrowing, got {}",
        s.imager_width
    );
}
