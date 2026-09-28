//! `EMITTED_PARAM_KEYS` is exactly the set of keys the engine can emit:
//! the mastering plugin's lockstep test resolves that list against its
//! params, so a key emitted but missing from it would go unchecked.

use std::collections::BTreeSet;

use resonance_mastering_assist::decide::{
    bins_for_range, build, Target, EMITTED_PARAM_KEYS, HIGH_BAND_HZ, LOW_BAND_HZ,
};
use resonance_mastering_assist::targets::{target_band, target_curve};
use resonance_mastering_assist::{AnalysisResult, Genre};

fn analysis(crest_db: f32, correlation: f32, lufs: f32, spectrum: Vec<f32>) -> AnalysisResult {
    AnalysisResult {
        sample_rate: 48_000.0,
        duration_s: 10.0,
        integrated_lufs: lufs,
        short_term_lufs: lufs,
        true_peak_dbtp: -1.0,
        crest_db,
        correlation,
        spectrum_db: spectrum,
    }
}

/// Spectra and readings that walk every branch of every stage.
fn every_branch() -> Vec<AnalysisResult> {
    let (lo, hi) = target_band(Genre::Rock);
    let on = target_curve(Genre::Rock).to_vec();
    let mut off = on.clone();
    let (l0, l1) = bins_for_range(LOW_BAND_HZ);
    for i in l0..l1 {
        off[i] = hi[i] + 3.0;
    }
    let (h0, h1) = bins_for_range(HIGH_BAND_HZ);
    for i in h0..h1 {
        off[i] = lo[i] - 2.0;
    }
    let mut out = Vec::new();
    for spectrum in [on, off] {
        for crest in [7.0, 12.0, 18.0] {
            for correlation in [0.1, 0.5, 0.85, 0.95] {
                for lufs in [-11.0, -30.0] {
                    out.push(analysis(crest, correlation, lufs, spectrum.clone()));
                }
            }
        }
    }
    out
}

#[test]
fn the_emitted_key_list_is_exact() {
    let listed: BTreeSet<&str> = EMITTED_PARAM_KEYS.iter().copied().collect();
    assert_eq!(listed.len(), EMITTED_PARAM_KEYS.len(), "a key is listed twice");
    let mut emitted = BTreeSet::new();
    for a in every_branch() {
        for stage in build(&a, &Target::Genre(Genre::Rock)).stages() {
            for change in stage.params {
                assert!(listed.contains(change.key), "{} is emitted but not listed", change.key);
                emitted.insert(change.key);
            }
        }
    }
    let never: Vec<_> = listed.difference(&emitted).collect();
    assert!(never.is_empty(), "listed but never emitted: {never:?}");
}
