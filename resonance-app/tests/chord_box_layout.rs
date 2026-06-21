//! Geometry tests for the backend-agnostic `chord_box` layout module.
//!
//! These pin down the shared chord-diagram *convention* (nut vs start-fret
//! label, board/string/fret geometry, dot placement + root accent, open/mute
//! markers) independently of any rendering backend, covering both a
//! nut-anchored voicing and a boxed (`start_fret > 0`) voicing.

use resonance_app::chord_box::{self, Dims, Marker, Nut};
use resonance_music_theory::{
    fretboard_voicing, Chord, ChordQuality, PitchClass, BASS_4, GUITAR_6,
};

/// Round-number dims so expected coordinates are exact and obvious.
fn test_dims() -> Dims {
    Dims {
        cell_w: 100.0,
        chord_name_h: 5.0,
        header_h: 5.0,
        fret_spacing: 4.0,
        fret_count: 4,
        string_spacing: 10.0,
        dot_r: 1.0,
    }
}

fn approx(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
}

#[test]
fn nut_anchored_c_major() {
    let dims = test_dims();
    // Open C major on a 6-string guitar: x32010.
    let frets = [None, Some(3), Some(2), Some(0), Some(1), Some(0)];
    let chord = Chord::new(PitchClass::C, ChordQuality::Maj);
    let lo = chord_box::layout(&dims, (0.0, 0.0), &GUITAR_6, &frets, 0, &chord);

    assert_eq!(lo.name, "C");
    approx(lo.cell_center_x, 50.0);
    approx(lo.name_y, 0.0);
    approx(lo.header_y, 5.0);
    approx(lo.nut_y, 10.0);
    approx(lo.board_w, 50.0); // (6-1) * 10
    approx(lo.board_x, 25.0); // (100 - 50) / 2
    approx(lo.board_h, 16.0); // 4 * 4

    // Nut-anchored.
    assert_eq!(lo.nut, Nut::Open);

    // Fret lines 0..=4.
    assert_eq!(lo.fret_ys.len(), 5);
    approx(lo.fret_ys[0], 10.0);
    approx(lo.fret_ys[4], 26.0);

    // Strings: positions, labels, open/mute markers.
    assert_eq!(lo.strings.len(), 6);
    let xs: Vec<f32> = lo.strings.iter().map(|s| s.x).collect();
    for (got, want) in xs.iter().zip([25.0, 35.0, 45.0, 55.0, 65.0, 75.0]) {
        approx(*got, want);
    }
    let labels: Vec<&str> = lo.strings.iter().map(|s| s.label).collect();
    assert_eq!(labels, ["E", "A", "D", "G", "B", "e"]);
    let markers: Vec<Option<Marker>> = lo.strings.iter().map(|s| s.marker).collect();
    assert_eq!(
        markers,
        [
            Some(Marker::Mute), // string muted
            None,               // fret 3
            None,               // fret 2
            Some(Marker::Open), // open
            None,               // fret 1
            Some(Marker::Open), // open
        ]
    );

    // Dots: only the three fretted (non-open) strings, with root accent.
    assert_eq!(lo.dots.len(), 3);

    // String 1 (A), fret 3 -> C (root).
    let d0 = &lo.dots[0];
    approx(d0.x, 35.0);
    approx(d0.y, 20.0); // nut_y + (3 - 0.5) * 4
    approx(d0.r, 1.0);
    assert_eq!(d0.note, "C");
    assert!(d0.is_root);

    // String 2 (D), fret 2 -> E (not root).
    let d1 = &lo.dots[1];
    approx(d1.x, 45.0);
    approx(d1.y, 16.0); // nut_y + (2 - 0.5) * 4
    assert_eq!(d1.note, "E");
    assert!(!d1.is_root);

    // String 4 (B), fret 1 -> C (root).
    let d2 = &lo.dots[2];
    approx(d2.x, 65.0);
    approx(d2.y, 12.0); // nut_y + (1 - 0.5) * 4
    assert_eq!(d2.note, "C");
    assert!(d2.is_root);
}

#[test]
fn boxed_start_fret_maps_into_window() {
    let dims = test_dims();
    // A-form C major barre at the 3rd fret: x35553, start_fret = 3.
    let frets = [None, Some(3), Some(5), Some(5), Some(5), Some(3)];
    let chord = Chord::new(PitchClass::C, ChordQuality::Maj);
    let lo = chord_box::layout(&dims, (0.0, 0.0), &GUITAR_6, &frets, 3, &chord);

    // Boxed: start-fret label instead of a nut bar.
    assert_eq!(lo.nut, Nut::StartFret(3));

    // Five fretted strings, all inside the window.
    assert_eq!(lo.dots.len(), 5);

    // String 1 (A), fret 3 -> display fret 1 -> C (root).
    let d = &lo.dots[0];
    approx(d.x, 35.0);
    approx(d.y, 12.0); // nut_y + (1 - 0.5) * 4
    assert_eq!(d.note, "C");
    assert!(d.is_root);

    // String 2 (D), fret 5 -> display fret 3 -> G (not root).
    let d = &lo.dots[1];
    approx(d.y, 20.0); // nut_y + (3 - 0.5) * 4
    assert_eq!(d.note, "G");
    assert!(!d.is_root);
}

#[test]
fn dots_outside_window_are_dropped() {
    let dims = test_dims(); // fret_count = 4
                            // start_fret 3 -> visible frets 3..=6; fret 9 maps to display 7 (dropped).
    let frets = [None, Some(3), Some(9), None, None, None];
    let chord = Chord::new(PitchClass::C, ChordQuality::Maj);
    let lo = chord_box::layout(&dims, (0.0, 0.0), &GUITAR_6, &frets, 3, &chord);

    // Only the in-window fret-3 dot survives; the fret-9 dot is clipped.
    assert_eq!(lo.dots.len(), 1);
    approx(lo.dots[0].x, 35.0);
}

#[test]
fn origin_offsets_all_coordinates() {
    let dims = test_dims();
    let frets = [None, Some(3), Some(2), Some(0), Some(1), Some(0)];
    let chord = Chord::new(PitchClass::C, ChordQuality::Maj);
    let base = chord_box::layout(&dims, (0.0, 0.0), &GUITAR_6, &frets, 0, &chord);
    let moved = chord_box::layout(&dims, (10.0, 20.0), &GUITAR_6, &frets, 0, &chord);

    approx(moved.board_x, base.board_x + 10.0);
    approx(moved.nut_y, base.nut_y + 20.0);
    approx(moved.cell_center_x, base.cell_center_x + 10.0);
    for (m, b) in moved.dots.iter().zip(&base.dots) {
        approx(m.x, b.x + 10.0);
        approx(m.y, b.y + 20.0);
        assert_eq!(m.note, b.note);
        assert_eq!(m.is_root, b.is_root);
    }
}

#[test]
fn integrates_with_real_voicing() {
    let dims = test_dims();
    // A real voicing from the music-theory engine drives the layout end to end.
    let chord = Chord::new(PitchClass::E, ChordQuality::Min7);
    let v = fretboard_voicing(&chord, &BASS_4);
    let lo = chord_box::layout(&dims, (0.0, 0.0), &BASS_4, &v.frets, v.start_fret, &chord);

    // Nut variant tracks the voicing's start fret.
    match lo.nut {
        Nut::Open => assert_eq!(v.start_fret, 0),
        Nut::StartFret(sf) => {
            assert_eq!(sf, v.start_fret);
            assert!(sf > 0);
        }
    }

    // One dot per fretted (non-open) string that lands inside the window.
    let fretted_in_window = v
        .frets
        .iter()
        .filter(|f| matches!(f, Some(n) if *n > 0))
        .filter(|f| {
            let n = f.unwrap();
            let display = if v.start_fret == 0 {
                n
            } else {
                n - v.start_fret + 1
            };
            display <= dims.fret_count
        })
        .count();
    assert_eq!(lo.dots.len(), fretted_in_window);
    assert_eq!(lo.strings.len(), BASS_4.string_count());
}
