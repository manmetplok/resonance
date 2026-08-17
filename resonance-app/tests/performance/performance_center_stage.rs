//! Pure, headless tests for the Performance centre-stage hero helpers
//! (todo #308, design #151, arch doc #152).
//!
//! These cover the voicing selection + fingerprint logic the Canvas draw
//! path relies on — without going through `wgpu`, so they are deterministic
//! and environment-independent. The on-screen rendering is exercised by the
//! `iced_test` snapshot suite; here we lock the data contract: slash chords
//! voice bass-consistently, the cache fingerprint changes exactly when the
//! hero's appearance does, and the chord-box layout flags the bass dot as the
//! accent root.

use resonance_app::chord_box::{self, Dims};
use resonance_app::view::performance::center_stage::{
    diagram_size, hero_fingerprint, lowest_sounding_pc, voicing_for,
};
use resonance_music_theory::{
    Chord, ChordQuality, PitchClass, BASS_4, GUITAR_6, GUITAR_8,
};

fn pc(semitone: u8) -> PitchClass {
    PitchClass::from_semitone(semitone)
}

/// C, E, F, G, A as pitch classes.
const C: u8 = 0;
const E: u8 = 4;
const F: u8 = 5;
const G: u8 = 7;
const A: u8 = 9;

fn major(root: u8) -> Chord {
    Chord::new(pc(root), ChordQuality::Maj)
}

#[test]
fn root_position_major_voices_with_root_in_the_bass() {
    let voicing = voicing_for(major(C), &GUITAR_6, 0);
    assert_eq!(
        lowest_sounding_pc(&voicing, &GUITAR_6),
        Some(C),
        "open C major sounds its root (C) on the lowest played string"
    );
    assert!(
        voicing.frets.iter().any(|f| f.is_some()),
        "a real voicing has at least one played string"
    );
}

#[test]
fn slash_chord_voices_bass_consistently() {
    // F/A — first inversion. The lowest sounding note must be the slash
    // bass (A), not the chord root (F).
    let f_over_a = major(F).with_bass(pc(A));
    let voicing = voicing_for(f_over_a, &GUITAR_6, 0);
    assert_eq!(
        lowest_sounding_pc(&voicing, &GUITAR_6),
        Some(A),
        "F/A must sound A in the bass"
    );

    // C/E — another inversion, sanity check on a different shape.
    let c_over_e = major(C).with_bass(pc(E));
    let voicing = voicing_for(c_over_e, &GUITAR_6, 0);
    assert_eq!(
        lowest_sounding_pc(&voicing, &GUITAR_6),
        Some(E),
        "C/E must sound E in the bass"
    );
}

#[test]
fn slash_chord_layout_flags_the_bass_dot_as_the_accent_root() {
    let f_over_a = major(F).with_bass(pc(A));
    let voicing = voicing_for(f_over_a, &GUITAR_6, 0);

    let dims = Dims {
        cell_w: 240.0,
        chord_name_h: 0.0,
        header_h: 30.0,
        fret_spacing: 52.0,
        fret_count: 4,
        string_spacing: 30.0,
        dot_r: 12.0,
    };
    let layout = chord_box::layout(
        &dims,
        (0.0, 0.0),
        &GUITAR_6,
        &voicing.frets,
        voicing.start_fret,
        &f_over_a,
    );

    assert_eq!(layout.name, "F/A", "the diagram labels the slash chord");
    // Every dot flagged as the accent root must actually be the bass note,
    // and at least one such dot exists.
    let root_dots: Vec<_> = layout.dots.iter().filter(|d| d.is_root).collect();
    assert!(
        !root_dots.is_empty(),
        "the bass note is drawn as the accent root dot"
    );
    for dot in root_dots {
        assert_eq!(dot.note, pc(A).as_str(), "accent dot is the bass (A)");
    }
}

#[test]
fn capo_pushes_the_voicing_up_the_neck_without_open_strings() {
    let voicing = voicing_for(major(C), &GUITAR_6, 3);
    assert!(
        voicing.frets.iter().filter_map(|f| *f).all(|fret| fret >= 1),
        "with a capo no string is played open (below the capo)"
    );
    assert!(
        voicing.frets.iter().any(|f| f.is_some()),
        "a capo'd voicing still plays strings"
    );
    assert_eq!(
        lowest_sounding_pc(&voicing, &GUITAR_6),
        Some(C),
        "capo'd C major still sounds its root in the bass"
    );
}

#[test]
fn fingerprint_tracks_chord_tuning_and_capo() {
    let c = major(C);
    let g = major(G);

    // Stable for identical inputs.
    assert_eq!(
        hero_fingerprint(c, &GUITAR_6, 0),
        hero_fingerprint(c, &GUITAR_6, 0)
    );
    // Changes with the chord, the tuning, and the capo.
    assert_ne!(
        hero_fingerprint(c, &GUITAR_6, 0),
        hero_fingerprint(g, &GUITAR_6, 0),
        "a different chord must invalidate the cache"
    );
    assert_ne!(
        hero_fingerprint(c, &GUITAR_6, 0),
        hero_fingerprint(c, &BASS_4, 0),
        "a different tuning must invalidate the cache"
    );
    assert_ne!(
        hero_fingerprint(c, &GUITAR_6, 0),
        hero_fingerprint(c, &GUITAR_6, 2),
        "a different capo must invalidate the cache"
    );
    // Slash vs plain triad are distinct.
    assert_ne!(
        hero_fingerprint(c, &GUITAR_6, 0),
        hero_fingerprint(c.with_bass(pc(E)), &GUITAR_6, 0),
        "a slash bass must invalidate the cache"
    );
}

#[test]
fn diagram_size_is_positive_and_widens_with_more_strings() {
    let (w6, h6) = diagram_size(&GUITAR_6);
    let (w8, h8) = diagram_size(&GUITAR_8);
    let (w4, _h4) = diagram_size(&BASS_4);

    assert!(w6 > 0.0 && h6 > 0.0, "the canvas has a real size");
    assert!(w8 > w6, "an 8-string diagram is wider than a 6-string one");
    assert!(w6 > w4, "a 6-string diagram is wider than a 4-string bass");
    assert_eq!(h6, h8, "height depends on the fret window, not string count");
}
