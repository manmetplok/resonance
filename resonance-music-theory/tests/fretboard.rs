//! Fretboard voicing search, including upper-register positions
//! reached via `voicing_from`.

use resonance_music_theory::chord::ChordQuality;
use resonance_music_theory::{
    fretboard_voicing, fretboard_voicing_from, Chord, PitchClass, ALL_TUNINGS, GUITAR_6,
    MAX_START_FRET, WINDOW_FRETS,
};

fn a_major() -> Chord {
    Chord::new(PitchClass::A, ChordQuality::Maj)
}

#[test]
fn open_position_c_major_unchanged() {
    // Textbook open C shape: x-3-2-0-1-0 (low E muted below the root).
    let v = fretboard_voicing(&Chord::new(PitchClass::C, ChordQuality::Maj), &GUITAR_6);
    assert_eq!(
        v.frets,
        vec![None, Some(3), Some(2), Some(0), Some(1), Some(0)]
    );
    assert_eq!(v.start_fret, 0);
}

#[test]
fn min_start_yields_fifth_fret_barre() {
    // E-shape A-major barre at fret 5: 5-7-7-6-5-5.
    let v = fretboard_voicing_from(&a_major(), &GUITAR_6, 5);
    assert_eq!(
        v.frets,
        vec![Some(5), Some(7), Some(7), Some(6), Some(5), Some(5)]
    );
    assert_eq!(v.start_fret, 5);
}

#[test]
fn upper_register_voicing_above_fret_11_is_reachable() {
    // Second-octave A-major shape at fret 12: x-12-14-14-14-12 — every
    // sounding fret is above the old `start..=7` window cap (max fret
    // 11), which made positions like this unreachable.
    let v = fretboard_voicing_from(&a_major(), &GUITAR_6, 12);
    assert_eq!(
        v.frets,
        vec![None, Some(12), Some(14), Some(14), Some(14), Some(12)]
    );
    assert_eq!(v.start_fret, 12);
    let highest = v.frets.iter().filter_map(|f| *f).max().unwrap();
    assert!(highest > 11, "expected an upper-register fret, got {highest}");
}

#[test]
fn upper_register_voicings_are_fully_fretted() {
    // E major has three open-string chord tones on a guitar; a
    // `min_start > 0` request must not fall back to them.
    let v = fretboard_voicing_from(&Chord::new(PitchClass::E, ChordQuality::Maj), &GUITAR_6, 8);
    for fret in v.frets.iter().flatten() {
        assert!(
            (8..=12).contains(fret),
            "fret {fret} outside the requested 8..=12 window"
        );
    }
    assert!(v.frets.iter().any(|f| f.is_some()));
}

#[test]
fn min_start_clamps_to_max_start_fret() {
    let v = fretboard_voicing_from(&a_major(), &GUITAR_6, u8::MAX);
    for fret in v.frets.iter().flatten() {
        assert!(
            (MAX_START_FRET..MAX_START_FRET + WINDOW_FRETS).contains(fret),
            "fret {fret} outside the clamped {MAX_START_FRET}..={} window",
            MAX_START_FRET + WINDOW_FRETS - 1
        );
    }
    assert!(v.frets.iter().any(|f| f.is_some()));
}

#[test]
fn default_search_prefers_the_lowest_position_unless_it_drops_strings() {
    // `voicing()` stays in the nut-anchored window (every fretted note
    // within frets 1..=WINDOW_FRETS) for every quality whose chord
    // tones it can sound on all strings there. Only dim triads — whose
    // 3-semitone tone gaps can leave a string without a reachable tone
    // in a 4-fret nut window — may box up the neck, and only when the
    // boxed window sounds strictly more strings than the best
    // nut-anchored shape.
    for root in 0..12u8 {
        let root_pc = PitchClass::from_semitone(root);
        for q in ChordQuality::ALL {
            let chord = Chord::new(root_pc, q);
            let v = fretboard_voicing(&chord, &GUITAR_6);
            let highest = v.frets.iter().filter_map(|f| *f).max().unwrap_or(0);
            if highest <= WINDOW_FRETS {
                continue;
            }
            assert_eq!(
                q,
                ChordQuality::Dim,
                "{chord}: only dim triads may escape the nut window, \
                 got frets {:?}",
                v.frets
            );
            // The escape must pay for itself in sounding strings:
            // compare against the per-string reachability of the nut
            // window (open string or frets 1..=WINDOW_FRETS).
            let pcs: Vec<u8> = chord.pitch_classes().map(|pc| pc.to_semitone()).collect();
            let nut_reachable = GUITAR_6
                .open
                .iter()
                .filter(|&&open| (0..=WINDOW_FRETS).any(|f| pcs.contains(&((open + f) % 12))))
                .count();
            // The search compares pre-mute scores (strictly greater on
            // escape); the final voicing may then mute strings below
            // the root, so externally we can only require parity.
            let sounding = v.frets.iter().flatten().count();
            assert!(
                sounding >= nut_reachable,
                "{chord}: escaped to fret {highest} while sounding fewer \
                 strings ({sounding} vs nut-window {nut_reachable})"
            );
        }
    }
}

#[test]
fn fret_one_voicings_anchor_at_the_nut() {
    // Open C fingers fret 1 on the B string yet renders from the nut:
    // `start_fret == 0` is the display anchor, and open vs fret-1 is
    // carried by `frets` (`Some(0)` vs `Some(1)`), not by `start_fret`.
    let v = fretboard_voicing(&Chord::new(PitchClass::C, ChordQuality::Maj), &GUITAR_6);
    assert_eq!(v.start_fret, 0);
    assert!(v.frets.contains(&Some(1)));
    assert!(v.frets.contains(&Some(0)));

    // A fully fretted fret-1 barre also anchors at the nut (standard
    // chord-chart convention for the F barre), without any open string.
    let v = fretboard_voicing_from(&Chord::new(PitchClass::F, ChordQuality::Maj), &GUITAR_6, 1);
    assert_eq!(v.start_fret, 0);
    assert!(!v.frets.contains(&Some(0)));
    assert!(v.frets.contains(&Some(1)));
}

#[test]
fn every_voicing_fits_a_window_frets_tall_diagram() {
    // `start_fret` is a display anchor: a chord-diagram renderer with
    // `WINDOW_FRETS` rows must be able to draw every fretted note.
    // Nut-anchored voicings (`start_fret == 0`, which includes the
    // deliberate fret-1 collapse) fit frets `1..=WINDOW_FRETS`; boxed
    // voicings fit `start_fret..=start_fret + WINDOW_FRETS - 1`.
    // Regression: with the old 5-wide search window, Cdim on Bass 4
    // produced frets [x, 3, 1, 5] with start_fret 0, silently clipping
    // the fret-5 dot off a 4-row nut-anchored diagram.
    for root in 0..12u8 {
        let root_pc = PitchClass::from_semitone(root);
        for q in ChordQuality::ALL {
            let chord = Chord::new(root_pc, q);
            for tuning in ALL_TUNINGS {
                for min_start in 0..=MAX_START_FRET {
                    let v = fretboard_voicing_from(&chord, tuning, min_start);
                    let lo = v.start_fret.max(1);
                    let hi = v.start_fret.max(1) + WINDOW_FRETS - 1;
                    for fret in v.frets.iter().flatten().copied().filter(|&f| f > 0) {
                        assert!(
                            (lo..=hi).contains(&fret),
                            "{chord} {} min_start={min_start}: fret {fret} outside \
                             display window {lo}..={hi} (start_fret {})",
                            tuning.short,
                            v.start_fret
                        );
                    }
                }
            }
        }
    }
}
