//! The loudness gap is split between the input trim and the limiter's
//! input gain (`lim_gain`): the trim stops 3 dB short of the target and
//! the limiter is pushed the rest, so the loudness is made after the
//! clipper rather than in every stage before it.

use resonance_mastering_assist::decide::{build, Target};
use resonance_mastering_assist::targets::target_curve;
use resonance_mastering_assist::{AnalysisResult, Genre};

fn at_lufs(lufs: f32) -> AnalysisResult {
    AnalysisResult {
        sample_rate: 48_000.0,
        duration_s: 10.0,
        integrated_lufs: lufs,
        short_term_lufs: lufs,
        true_peak_dbtp: -1.0,
        crest_db: 12.0,
        correlation: 0.6,
        spectrum_db: target_curve(Genre::Rock).to_vec(),
    }
}

#[test]
fn trim_and_limiter_gain_add_up_to_the_loudness_gap() {
    // Rock targets −11 LUFS.
    for (lufs, trim, gain) in [(-20.0, 6.0, 3.0), (-8.0, -6.0, 3.0), (-50.0, 24.0, 15.0)] {
        let s = build(&at_lufs(lufs), &Target::Genre(Genre::Rock));
        assert!((s.input_trim_db - trim).abs() < 1e-4, "{lufs} LUFS: trim {}", s.input_trim_db);
        assert!((s.limiter_gain_db - gain).abs() < 1e-4, "{lufs} LUFS: gain {}", s.limiter_gain_db);
        let limiter = s.stages().into_iter().find(|st| st.stage == "limiter").unwrap();
        let written = limiter.params.iter().find(|c| c.key == "lim_gain").map(|c| c.value);
        assert_eq!(written, Some(gain), "{lufs} LUFS: {:?}", limiter.params);
    }
}
