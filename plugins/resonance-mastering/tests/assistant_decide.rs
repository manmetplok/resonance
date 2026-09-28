use resonance_mastering::assistant::analyze::AnalysisResult;
use resonance_mastering::assistant::decide::{
    bins_for_range, build, param_by_key, Target, HIGH_BAND_HZ, LOW_BAND_HZ, STAGE_DIAGNOSTIC,
};
use resonance_mastering::params::MasteringParams;
use resonance_mastering::PARAM_COUNT;
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

/// Analyses that make every stage speak: a low band over the band, a top
/// under it, loose dynamics and a narrow image.
fn busy_analyses() -> Vec<AnalysisResult> {
    let (lo, hi) = target_band(Genre::Rock);
    let mut analyzed = on_target();
    let (l0, l1) = bins_for_range(LOW_BAND_HZ);
    for i in l0..l1 {
        analyzed[i] = hi[i] + 3.0;
    }
    let (h0, h1) = bins_for_range(HIGH_BAND_HZ);
    for i in h0..h1 {
        analyzed[i] = lo[i] - 2.0;
    }
    let mut a = dummy_analysis(18.0, analyzed);
    a.correlation = 0.95;
    a.integrated_lufs = -24.0;
    vec![
        a,
        dummy_analysis(7.0, on_target()),
        dummy_analysis(12.0, on_target()),
    ]
}

/// The stage list names real param keys, and applying the suggestions
/// writes exactly those keys to exactly those values — so an agent that
/// sets the listed keys over the control API gets what Apply gets.
#[test]
fn stages_name_real_keys_and_apply_writes_exactly_them() {
    for analysis in busy_analyses() {
        let s = build(&analysis, &Target::Genre(Genre::Rock));
        let stages = s.stages();
        assert!(!stages.is_empty());
        let lines: usize = stages.iter().map(|st| st.rationale.len()).sum();
        assert_eq!(lines, s.rationale.len(), "every rationale line belongs to a stage");

        let params = MasteringParams::default();
        let defaults: Vec<f64> =
            (0..PARAM_COUNT).map(|i| params.param_at(i).get_plain()).collect();
        s.apply_to(&params);
        let mut written = std::collections::HashSet::new();
        for stage in &stages {
            if stage.stage == STAGE_DIAGNOSTIC {
                assert!(stage.params.is_empty());
            }
            for change in &stage.params {
                let p = param_by_key(&params, change.key)
                    .unwrap_or_else(|| panic!("{} is not a mastering param", change.key));
                assert!(
                    (p.get_plain() - f64::from(change.value)).abs() < 1e-4,
                    "{} = {} after apply, suggested {}",
                    change.key,
                    p.get_plain(),
                    change.value
                );
                written.insert(change.key);
            }
        }
        for i in 0..PARAM_COUNT {
            let p = params.param_at(i);
            if !written.contains(p.id()) {
                assert_eq!(p.get_plain(), defaults[i], "apply touched unlisted {}", p.id());
            }
        }
    }
}

#[test]
fn shelves_are_listed_only_when_they_move() {
    let s = build(&dummy_analysis(12.0, on_target()), &Target::Genre(Genre::Rock));
    for stage in s.stages() {
        if stage.stage.starts_with("tonal_") {
            assert!(stage.params.is_empty(), "{:?}", stage);
        }
    }
    let s = build(&busy_analyses()[0], &Target::Genre(Genre::Rock));
    let keys: Vec<&str> = s
        .stages()
        .iter()
        .flat_map(|st| st.params.iter().map(|c| c.key))
        .collect();
    for key in ["tone_b0_gain", "tone_b3_gain", "img_width", "lim_ceiling", "glue_ratio"] {
        assert!(keys.contains(&key), "{key} missing from {keys:?}");
    }
}
