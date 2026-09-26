//! The audition scrub strip redraws when the auditioned file changes
//! (review VIEW-36).
//!
//! Its cache fingerprint mixed the peak count — always
//! `THUMBNAIL_BUCKETS` — but not the peaks, so selecting another file of
//! the same length with auto-play off kept showing the previous waveform.

use resonance_app::Resonance;

fn peaks(scale: f32) -> Vec<(f32, f32)> {
    (0..64)
        .map(|i| {
            let v = scale * (i as f32 / 64.0);
            (-v, v)
        })
        .collect()
}

#[test]
fn different_peaks_same_length_change_the_fingerprint() {
    let a = Resonance::test_audition_scrub_fingerprint(&peaks(0.5), 0, 48_000, false);
    let b = Resonance::test_audition_scrub_fingerprint(&peaks(0.9), 0, 48_000, false);
    assert_ne!(a, b, "loop B must not reuse loop A's cached strip");
}

#[test]
fn identical_inputs_keep_the_fingerprint() {
    let a = Resonance::test_audition_scrub_fingerprint(&peaks(0.5), 100, 48_000, true);
    let b = Resonance::test_audition_scrub_fingerprint(&peaks(0.5), 100, 48_000, true);
    assert_eq!(a, b);
}
