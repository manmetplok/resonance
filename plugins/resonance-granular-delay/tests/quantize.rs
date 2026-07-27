//! Direct unit tests of the plugin-side pitch quantizer helper
//! (ba todo #1078, doc #252 §3): arbitrary fractional-semitone
//! transposes snap to the nearest scale degree for C major and
//! A minor, to integer semitones in semitone mode, and pass through
//! untouched when quantization is off.

use resonance_granular_delay::quantize::{
    mode_from_index, quantize_transpose, root_from_index, PitchQuantize,
};
use resonance_music_theory::{Mode, PitchClass, Scale};

fn c_major() -> Scale {
    Scale::new(PitchClass::C, Mode::Major)
}

fn a_minor() -> Scale {
    Scale::new(PitchClass::A, Mode::Minor)
}

#[test]
fn off_leaves_values_untouched() {
    for &v in &[0.0f32, 0.37, -13.42, 24.0, -24.0, 100.0 / 300.0] {
        let q = quantize_transpose(v, PitchQuantize::Off, c_major());
        assert!(
            q.to_bits() == v.to_bits(),
            "quantize off altered {v} -> {q}"
        );
    }
}

#[test]
fn semitone_mode_rounds_to_integer_semitones() {
    let q = |v: f32| quantize_transpose(v, PitchQuantize::Semitones, c_major());
    assert_eq!(q(0.0), 0.0);
    assert_eq!(q(1.4), 1.0);
    assert_eq!(q(2.6), 3.0);
    assert_eq!(q(-0.6), -1.0);
    assert_eq!(q(-3.5), -4.0); // f32::round ties away from zero
    assert_eq!(q(11.9), 12.0);
    assert_eq!(q(-23.9), -24.0);
}

#[test]
fn scale_mode_snaps_to_c_major_degrees() {
    // C major admits transposes {0, 2, 4, 5, 7, 9, 11} + 12k above the
    // root.
    let q = |v: f32| quantize_transpose(v, PitchQuantize::Scale, c_major());
    assert_eq!(q(0.3), 0.0);
    assert_eq!(q(1.4), 2.0);
    assert_eq!(q(3.4), 4.0); // major third
    assert_eq!(q(5.6), 5.0);
    assert_eq!(q(6.0), 7.0); // equidistant: documented upward tie-break
    assert_eq!(q(-1.4), -1.0); // leading tone below the root
    assert_eq!(q(12.9), 12.0); // octave lattice repeats
    assert_eq!(q(-12.0), -12.0);
}

#[test]
fn scale_mode_snaps_to_a_minor_degrees() {
    // A minor admits transposes {0, 2, 3, 5, 7, 8, 10} + 12k above the
    // root — a different degree lattice than C major even though the
    // two share pitch classes.
    let q = |v: f32| quantize_transpose(v, PitchQuantize::Scale, a_minor());
    assert_eq!(q(1.4), 2.0);
    assert_eq!(q(3.4), 3.0); // minor third — C major snaps this to 4
    assert_eq!(q(4.0), 5.0); // equidistant 3/5: upward tie-break
    assert_eq!(q(8.6), 8.0); // minor sixth
    assert_eq!(q(-2.4), -2.0);
    assert_eq!(q(12.4), 12.0);
}

#[test]
fn major_and_minor_disagree_on_the_third() {
    let major = quantize_transpose(3.4, PitchQuantize::Scale, c_major());
    let minor = quantize_transpose(3.4, PitchQuantize::Scale, a_minor());
    assert_eq!(major, 4.0);
    assert_eq!(minor, 3.0);
}

#[test]
fn chromatic_scale_rounds_like_semitone_mode() {
    let chromatic = Scale::new(PitchClass::C, Mode::Chromatic);
    assert_eq!(
        quantize_transpose(1.4, PitchQuantize::Scale, chromatic),
        1.0
    );
    assert_eq!(
        quantize_transpose(-4.6, PitchQuantize::Scale, chromatic),
        -5.0
    );
}

#[test]
fn param_index_mappings() {
    assert_eq!(PitchQuantize::from_index(0), PitchQuantize::Off);
    assert_eq!(PitchQuantize::from_index(1), PitchQuantize::Semitones);
    assert_eq!(PitchQuantize::from_index(2), PitchQuantize::Scale);
    assert_eq!(root_from_index(0), PitchClass::C);
    assert_eq!(root_from_index(9), PitchClass::A);
    assert_eq!(mode_from_index(1), Mode::Major);
    assert_eq!(mode_from_index(2), Mode::Minor);
    // Out-of-range values clamp instead of panicking.
    assert_eq!(mode_from_index(99), *Mode::ALL.last().unwrap());
    assert_eq!(root_from_index(12), PitchClass::C);
}
