use resonance_mastering::assistant::analyze::NUM_SPECTRUM_BINS;
use resonance_mastering::assistant::targets::{band_center_hz, target_curve, Genre};

#[test]
fn curve_has_correct_length() {
    for &g in Genre::ALL {
        let c = target_curve(g);
        assert_eq!(c.len(), NUM_SPECTRUM_BINS);
        for v in c {
            assert!(v.is_finite());
        }
    }
}

#[test]
fn rock_has_midrange_dip() {
    let c = target_curve(Genre::Rock);
    // Find 400 Hz bin and a neighbor one octave below.
    let mut idx_400 = 0;
    let mut idx_100 = 0;
    for i in 0..NUM_SPECTRUM_BINS {
        if band_center_hz(i) >= 400.0 && idx_400 == 0 {
            idx_400 = i;
        }
        if band_center_hz(i) >= 100.0 && idx_100 == 0 {
            idx_100 = i;
        }
    }
    // 400 Hz should be below 100 Hz by at least the gaussian dip
    // amount *above* the natural pink tilt (which favors 100 Hz).
    let dip = c[idx_100] - c[idx_400];
    assert!(dip > 0.5, "expected 400 Hz dip under 100 Hz, got {dip}");
}

#[test]
fn target_lufs_is_genre_dependent() {
    assert!(Genre::Rock.target_lufs() > Genre::Acoustic.target_lufs());
    assert!(Genre::Pop.target_lufs() > Genre::Jazz.target_lufs());
}
