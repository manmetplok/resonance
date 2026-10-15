//! `SyllableMode` (spec `vocal-mcp-control-and-quality.md` §6, D6):
//! Syllabic is the syllabic melody unchanged, Mixed and Melismatic split
//! some syllables into 2–3 notes and report those notes as generator
//! holds, deterministically per seed.

use resonance_music_theory::{
    count_syllables, derive_vocal_with_holds, derive_vocal_with_motif, generate_lyrics,
    SyllableMode, VocalParams, VocalStyle, VoiceType,
};

use super::{chords, STYLES};

fn params(style: VocalStyle, mode: SyllableMode, seed: u64) -> VocalParams {
    let mut p = VocalParams {
        style,
        syllable_mode: mode,
        voice: VoiceType::Soprano,
        ..VocalParams::default()
    };
    p.range = p.voice.default_range();
    p.draft = generate_lyrics(&p, seed);
    p
}

#[test]
fn syllabic_is_the_syllabic_melody_with_no_holds() {
    for &style in &STYLES {
        for seed in [7u64, 42, 0xABCD] {
            let p = params(style, SyllableMode::Syllabic, seed);
            let plain = derive_vocal_with_motif(&chords(), &p, 480, 4, None, None, seed);
            let derived = derive_vocal_with_holds(&chords(), &p, 480, 4, None, None, seed);
            assert_eq!(derived.notes, plain, "{style:?}/{seed}");
            assert!(derived.auto_holds.iter().all(|h| !h));
        }
    }
}

/// Generator holds per run, and the invariants every split keeps.
fn holds_for(mode: SyllableMode) -> usize {
    let mut total = 0;
    for &style in &STYLES {
        for seed in [7u64, 42, 0xABCD, 99, 1234] {
            let p = params(style, mode, seed);
            let plain = derive_vocal_with_motif(&chords(), &p, 480, 4, None, None, seed);
            let derived = derive_vocal_with_holds(&chords(), &p, 480, 4, None, None, seed);
            let again = derive_vocal_with_holds(&chords(), &p, 480, 4, None, None, seed);
            assert_eq!(derived, again, "deterministic per seed");
            assert_eq!(derived.notes.len(), derived.auto_holds.len());
            let holds = derived.auto_holds.iter().filter(|h| **h).count();
            // Every hold is an extra note: the lyric units are unchanged.
            assert_eq!(derived.notes.len() - holds, plain.len(), "{style:?}/{seed}");
            let units: u32 = p.draft.iter().map(|l| count_syllables(&l.text)).sum();
            assert_eq!(plain.len(), units as usize);
            // The song's first note has nothing to hold.
            assert!(!derived.auto_holds.first().copied().unwrap_or(false));
            // A hold starts after the note it continues has ended, and
            // every note stays inside the range.
            let (lo, hi) = p.range;
            for (w, hold) in derived.notes.windows(2).zip(&derived.auto_holds[1..]) {
                if *hold {
                    assert!(
                        w[0].start_tick + w[0].duration_ticks <= w[1].start_tick,
                        "{style:?}/{seed} overlap inside a melisma"
                    );
                }
            }
            for n in &derived.notes {
                assert!((lo..=hi).contains(&n.note), "{style:?}/{seed} out of range");
            }
            total += holds;
        }
    }
    total
}

#[test]
fn mixed_and_melismatic_make_holds_and_melismatic_makes_more() {
    let mixed = holds_for(SyllableMode::Mixed);
    let melismatic = holds_for(SyllableMode::Melismatic);
    assert!(mixed > 0, "Mixed makes some melismas");
    assert!(melismatic > mixed, "Melismatic ({melismatic}) makes more than Mixed ({mixed})");
}

#[test]
fn a_split_keeps_the_syllables_time_span() {
    let seed = 42;
    let p = params(VocalStyle::PopBallad, SyllableMode::Melismatic, seed);
    let plain = derive_vocal_with_motif(&chords(), &p, 480, 4, None, None, seed);
    let derived = derive_vocal_with_holds(&chords(), &p, 480, 4, None, None, seed);
    // Fold each hold back into the note it continues: the start of each
    // group and the end of its last note are the syllabic note's.
    let mut groups: Vec<(u64, u64)> = Vec::new();
    for (n, hold) in derived.notes.iter().zip(&derived.auto_holds) {
        let end = n.start_tick + n.duration_ticks;
        match groups.last_mut() {
            Some(g) if *hold => g.1 = end,
            _ => groups.push((n.start_tick, end)),
        }
    }
    let spans: Vec<(u64, u64)> =
        plain.iter().map(|n| (n.start_tick, n.start_tick + n.duration_ticks)).collect();
    assert_eq!(groups, spans);
}
