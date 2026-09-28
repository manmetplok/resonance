//! The mastering saturator's harmonic signature, read with the same
//! analysis `meter.probe` uses (warmth-width-depth.md §7.3, W3 exit
//! criterion): Tape character adds H2, Tube (fully symmetric) does not.

use resonance_mastering::stages::saturator::{Saturator, SaturatorConfig, Shaper};
use resonance_metering::probe::{
    analyze_harmonics, bin_exact_hz, probe_sine, HarmonicReport, PROBE_LEN,
};

const SR: f64 = 48_000.0;

fn probe(character: f32) -> HarmonicReport {
    let freq = bin_exact_hz(SR, 1_000.0);
    let warmup = SR as usize;
    let x = probe_sine(SR, freq, -12.0, warmup + PROBE_LEN);
    let mut left = x.clone();
    let mut right = x;
    let mut s = Saturator::new(SR as f32);
    let cfg = SaturatorConfig {
        enabled: true,
        drive_db: 12.0,
        character,
        mix: 1.0,
        shaper: Shaper::Smooth,
    };
    for (l, r) in left.chunks_mut(512).zip(right.chunks_mut(512)) {
        s.process_stereo(l, r, &cfg);
    }
    analyze_harmonics(SR, freq, &left[warmup..])
}

#[test]
fn tape_character_shows_second_harmonic() {
    let tape = probe(1.0);
    let h2 = tape.h[0].unwrap();
    assert!(h2 > -60.0, "Tape adds H2: {h2} dBc ({tape:?})");
    assert!(tape.thd_pct > 0.1, "12 dB of drive distorts audibly: {}", tape.thd_pct);
}

#[test]
fn symmetric_tube_character_is_odd_dominant() {
    let tube = probe(0.0);
    let tape = probe(1.0);
    assert!(tube.h2_h3_db.unwrap() < 0.0, "symmetric: H3 over H2 ({tube:?})");
    assert!(
        tape.h[0].unwrap() > tube.h[0].unwrap() + 20.0,
        "asymmetry is what brings H2: tape {:?} vs tube {:?}",
        tape.h[0],
        tube.h[0]
    );
}
