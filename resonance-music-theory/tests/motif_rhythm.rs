//! Tests for [`derive_motif_rhythm`]: the rhythm extraction the drum
//! motif mode uses to lock a drum voice to the section's shared motif.

use resonance_music_theory::{
    derive_motif_rhythm, Chord, ChordQuality, MotifParams, MotifSource, PitchClass, TimedChord,
};

const TPB: u32 = 480;

fn gen(p: MotifParams) -> MotifSource {
    MotifSource::Generated(p)
}

fn tc(chord: Chord, start_beat: u32, duration_beats: u32) -> TimedChord {
    TimedChord {
        chord,
        start_beat,
        duration_beats,
    }
}

#[test]
fn empty_chord_list_returns_empty_rhythm() {
    let hits = derive_motif_rhythm(&[], &gen(MotifParams::default()), TPB);
    assert!(hits.is_empty());
}

#[test]
fn fixed_seed_produces_deterministic_rhythm() {
    let chords = vec![tc(
        Chord::new(PitchClass::C, ChordQuality::Maj),
        0,
        4,
    )];
    let params = MotifParams {
        seed: 42,
        ..MotifParams::default()
    };
    let a = derive_motif_rhythm(&chords, &gen(params), TPB);
    let b = derive_motif_rhythm(&chords, &gen(params), TPB);
    assert!(!a.is_empty());
    assert_eq!(a, b);
}

#[test]
fn rhythm_first_hit_lands_at_chord_start() {
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let params = MotifParams {
        seed: 7,
        ..MotifParams::default()
    };
    let hits = derive_motif_rhythm(&chords, &gen(params), TPB);
    let first = hits.first().expect("at least one hit");
    assert_eq!(first.start_tick, 0);
}

#[test]
fn second_chord_hits_start_at_chord_boundary() {
    let chords = vec![
        tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(PitchClass::F, ChordQuality::Maj), 4, 4),
    ];
    let params = MotifParams {
        seed: 11,
        ..MotifParams::default()
    };
    let hits = derive_motif_rhythm(&chords, &gen(params), TPB);
    // The first onset of the second chord must land exactly on its
    // start tick — the rhythm restarts per chord rather than running
    // through the bar line.
    let second_chord_start = 4 * TPB as u64;
    assert!(
        hits.iter().any(|h| h.start_tick == second_chord_start),
        "expected a hit at {} but got {:?}",
        second_chord_start,
        hits.iter().map(|h| h.start_tick).collect::<Vec<_>>()
    );
}

#[test]
fn rhythm_total_duration_does_not_exceed_chord_span() {
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let params = MotifParams::default();
    let hits = derive_motif_rhythm(&chords, &gen(params), TPB);
    let chord_end = 4 * TPB as u64;
    for h in &hits {
        assert!(h.start_tick + h.duration_ticks <= chord_end);
    }
}

#[test]
fn at_least_one_hit_is_accented() {
    // The motif builder always marks the first note of each motif as
    // accented (and any note with a duration_ratio >= 2). Across a
    // 4-beat chord we should get one accented hit.
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let params = MotifParams {
        seed: 1,
        ..MotifParams::default()
    };
    let hits = derive_motif_rhythm(&chords, &gen(params), TPB);
    assert!(hits.iter().any(|h| h.accent));
}

#[test]
fn low_complexity_only_uses_simple_rhythm_patterns() {
    // complexity = 0.2 over 8 patterns caps the pattern pool at
    // floor(0.2 * 7) = 1, i.e. only the two simplest patterns:
    // [1,1,1,1] (steady) and [2,1,1] (long-short-short). A 3-note
    // motif on a 4-beat chord therefore tiles to exactly 3 hits with
    // ratios (1,1,1) or (2,1,1) — never a pattern whose second and
    // third slots differ. The old `ceil` admitted pattern [1,1,2] as
    // well, which put the long note last.
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    for seed in 0..256u64 {
        let params = MotifParams {
            seed,
            complexity: 0.2,
            motif_len: 3,
            ..MotifParams::default()
        };
        let hits = derive_motif_rhythm(&chords, &gen(params), TPB);
        assert_eq!(hits.len(), 3, "seed {seed}: expected exactly 3 hits");
        assert!(
            hits[0].duration_ticks >= hits[1].duration_ticks,
            "seed {seed}: long note must lead, got {:?}",
            hits.iter().map(|h| h.duration_ticks).collect::<Vec<_>>()
        );
        assert_eq!(
            hits[1].duration_ticks, hits[2].duration_ticks,
            "seed {seed}: trailing slots must be equal at low complexity, got {:?}",
            hits.iter().map(|h| h.duration_ticks).collect::<Vec<_>>()
        );
    }
}

#[test]
fn changing_motif_seed_changes_rhythm() {
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let a = derive_motif_rhythm(
        &chords,
        &gen(MotifParams {
            seed: 1,
            ..MotifParams::default()
        }),
        TPB,
    );
    let b = derive_motif_rhythm(
        &chords,
        &gen(MotifParams {
            seed: 2,
            ..MotifParams::default()
        }),
        TPB,
    );
    // Different seeds should at least sometimes produce different
    // hit sequences. If they happen to collide we need a different
    // seed pair; pinning [1, 2] is fine because the RNG is fixed.
    assert_ne!(a, b);
}
