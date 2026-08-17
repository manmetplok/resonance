//! Headless tests for `PerformanceState` — the Performance-mode footer's
//! instrument/tuning + capo selection and the capo-applied voicing the live
//! fingering diagrams consume (epic #11, todo #311, design #151 / arch #152).

use resonance_app::state::performance::MAX_CAPO;
use resonance_app::state::PerformanceState;
use resonance_music_theory::{
    fretboard_voicing, fretboard_voicing_from, parse_chord, ALL_TUNINGS, BASS_4, GUITAR_6,
};

fn chord(sym: &str) -> resonance_music_theory::Chord {
    parse_chord(sym).expect("test chord parses")
}

#[test]
fn default_is_guitar_6_no_capo() {
    let p = PerformanceState::default();
    assert_eq!(p.tuning_index, 0);
    assert_eq!(p.capo, 0);
    // ALL_TUNINGS[0] is the 6-string guitar.
    assert_eq!(p.tuning().name, GUITAR_6.name);
    assert_eq!(p.tuning().string_count(), 6);
}

#[test]
fn set_tuning_index_selects_and_rejects_out_of_range() {
    let mut p = PerformanceState::default();

    // Selecting Bass 4 (index 2) yields a 4-string bass tuning — the
    // diagrams then render bass-string diagrams.
    p.set_tuning_index(2);
    assert_eq!(p.tuning_index, 2);
    assert_eq!(p.tuning().name, BASS_4.name);
    assert_eq!(p.tuning().string_count(), 4);

    // Out-of-range indices are ignored: the selection is left untouched.
    p.set_tuning_index(ALL_TUNINGS.len());
    assert_eq!(p.tuning_index, 2);
    p.set_tuning_index(999);
    assert_eq!(p.tuning_index, 2);
}

#[test]
fn switching_tuning_keeps_capo() {
    let mut p = PerformanceState::default();
    p.set_capo(3);
    p.set_tuning_index(1);
    assert_eq!(p.capo, 3, "switching instrument must not reset the capo");
}

#[test]
fn set_capo_clamps_to_max() {
    let mut p = PerformanceState::default();
    p.set_capo(5);
    assert_eq!(p.capo, 5);
    p.set_capo(MAX_CAPO + 7);
    assert_eq!(p.capo, MAX_CAPO);
    p.set_capo(0);
    assert_eq!(p.capo, 0);
}

#[test]
fn voicing_without_capo_matches_open_voicing() {
    let p = PerformanceState::default();
    let c = chord("C");
    let got = p.voicing(&c);
    let want = fretboard_voicing(&c, &GUITAR_6);
    assert_eq!(got.frets, want.frets);
    assert_eq!(got.start_fret, want.start_fret);
}

#[test]
fn voicing_with_capo_matches_pinned_window_and_shifts_up() {
    let mut p = PerformanceState::default();
    p.set_capo(3);
    let c = chord("C");
    let got = p.voicing(&c);

    // The capo voicing is exactly the window-pinned search at the capo.
    let want = fretboard_voicing_from(&c, &GUITAR_6, 3);
    assert_eq!(got.frets, want.frets);
    assert_eq!(got.start_fret, want.start_fret);

    // Every sounded string is fretted at or above the capo (you cannot
    // fret below a capo), and no open strings remain.
    for f in got.frets.iter().flatten() {
        assert!(
            *f >= 3,
            "capo voicing must not use frets below the capo, got {f}"
        );
    }
    assert!(
        got.frets.iter().flatten().any(|f| *f > 0),
        "capo voicing should fret at least one string"
    );
}

#[test]
fn capo_changes_the_voicing() {
    // A non-zero capo should generally produce a different (transposed-up)
    // shape than the open position for the same chord.
    let mut p = PerformanceState::default();
    let c = chord("C");
    let open = p.voicing(&c);
    p.set_capo(5);
    let capoed = p.voicing(&c);
    assert_ne!(
        open.frets, capoed.frets,
        "capo at fret 5 should change the C voicing"
    );
}

#[test]
fn bass_tuning_yields_bass_string_diagram() {
    let mut p = PerformanceState::default();
    p.set_tuning_index(2); // Bass 4
    let v = p.voicing(&chord("E"));
    assert_eq!(
        v.frets.len(),
        4,
        "bass voicing should have one fret slot per bass string"
    );
}
