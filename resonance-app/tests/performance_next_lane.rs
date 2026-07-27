//! Pure, headless tests for the Performance next-chords look-ahead lane
//! helpers (todo #309, design #151, arch doc #152).
//!
//! These lock the data contract the Canvas/Element draw path relies on —
//! without going through `wgpu`, so they are deterministic and
//! environment-independent. The on-screen rendering is exercised by the
//! `iced_test` golden suite (owned by the e2e-tester) and the render
//! smoke-tests in `performance_next_lane_render.rs`; here we cover the
//! emphasis tiers, the bars-until math + labels, the mini-diagram sizing,
//! and the lazy-cache fingerprint.

use resonance_app::view::performance::next_lane::{
    bars_until, bars_until_label, emphasis_for, fingerprint, mini_size, Emphasis, NextCard,
};
use resonance_music_theory::{
    Chord, ChordQuality, PitchClass, BASS_4, GUITAR_6, GUITAR_8,
};

fn pc(semitone: u8) -> PitchClass {
    PitchClass::from_semitone(semitone)
}

fn major(root: u8) -> Chord {
    Chord::new(pc(root), ChordQuality::Maj)
}

fn card(chord: Chord, bars: u32, emphasis: Emphasis) -> NextCard {
    NextCard {
        chord,
        tuning: &GUITAR_6,
        capo: 0,
        bars_until: bars,
        emphasis,
    }
}

#[test]
fn emphasis_tiers_by_position() {
    // The immediate next chord is emphasised; the second is normal; any
    // further preview dims.
    assert_eq!(emphasis_for(0), Emphasis::First);
    assert_eq!(emphasis_for(1), Emphasis::Mid);
    assert_eq!(emphasis_for(2), Emphasis::Later);
    assert_eq!(emphasis_for(7), Emphasis::Later);
}

#[test]
fn bars_until_counts_bar_lines_and_clamps_at_zero() {
    // Forward look-ahead: bar-lines from the current bar to the chord's bar.
    assert_eq!(bars_until(4, 5), 1, "next bar");
    assert_eq!(bars_until(4, 7), 3);
    assert_eq!(bars_until(0, 0), 0, "a sub-bar change reads as 'this bar'");
    // A loop wrap-around (slot before the current bar) clamps to 0 rather
    // than underflowing.
    assert_eq!(bars_until(9, 2), 0, "loop wrap clamps, never underflows");
}

#[test]
fn bars_until_label_matches_the_design_copy() {
    assert_eq!(bars_until_label(0), "this bar");
    assert_eq!(bars_until_label(1), "in 1 bar");
    assert_eq!(bars_until_label(2), "in 2 bars");
    assert_eq!(bars_until_label(3), "in 3 bars");
    assert_eq!(bars_until_label(12), "in 12 bars");
}

#[test]
fn mini_size_is_positive_and_widens_with_more_strings() {
    let (w6, h6) = mini_size(&GUITAR_6);
    let (w8, h8) = mini_size(&GUITAR_8);
    let (w4, _h4) = mini_size(&BASS_4);

    assert!(w6 > 0.0 && h6 > 0.0, "the mini box has a real size");
    assert!(w8 > w6, "an 8-string box is wider than a 6-string one");
    assert!(w6 > w4, "a 6-string box is wider than a 4-string bass");
    assert_eq!(h6, h8, "height depends on the fret window, not string count");
}

#[test]
fn fingerprint_is_stable_for_identical_cards() {
    let a = vec![
        card(major(0), 1, Emphasis::First),
        card(major(7), 2, Emphasis::Mid),
    ];
    let b = vec![
        card(major(0), 1, Emphasis::First),
        card(major(7), 2, Emphasis::Mid),
    ];
    assert_eq!(fingerprint(&a), fingerprint(&b));
}

#[test]
fn fingerprint_tracks_every_field_that_changes_the_lane() {
    let base = vec![card(major(0), 1, Emphasis::First)];

    // Different chord.
    let chord = vec![card(major(7), 1, Emphasis::First)];
    assert_ne!(
        fingerprint(&base),
        fingerprint(&chord),
        "a different upcoming chord must invalidate the cache"
    );

    // Different bars-until (a bar passed).
    let bars = vec![card(major(0), 2, Emphasis::First)];
    assert_ne!(
        fingerprint(&base),
        fingerprint(&bars),
        "a bars-until change (a bar passing) must invalidate the cache"
    );

    // Different emphasis.
    let emph = vec![card(major(0), 1, Emphasis::Mid)];
    assert_ne!(
        fingerprint(&base),
        fingerprint(&emph),
        "a different emphasis must invalidate the cache"
    );

    // Different tuning.
    let mut tuned = base.clone();
    tuned[0].tuning = &BASS_4;
    assert_ne!(
        fingerprint(&base),
        fingerprint(&tuned),
        "a different tuning must invalidate the cache"
    );

    // Different capo.
    let mut capoed = base.clone();
    capoed[0].capo = 3;
    assert_ne!(
        fingerprint(&base),
        fingerprint(&capoed),
        "a different capo must invalidate the cache"
    );

    // Different card count (end of progression drops a card).
    let shorter: Vec<NextCard> = Vec::new();
    assert_ne!(
        fingerprint(&base),
        fingerprint(&shorter),
        "the empty/end state must not share a key with a populated lane"
    );
}
