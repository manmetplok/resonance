//! Rhythm transforms (Open Music Theory v2, rhythm and meter in pop
//! music): tresillo cells in the motif rhythm pool, the straight
//! syncopation transform, and rhythmic acceleration when an idea
//! fragments.
//!
//! Everything is asserted through the public generator entry points:
//!   - tresillo cells (3+3+2, rotations, double tresillo) surface at
//!     high complexity and never below the complexity floor;
//!   - the varied repeat of a sentence's basic idea is sometimes a
//!     straight syncopation — same pitch material, first duration
//!     halved and every later onset shifted earlier by that half;
//!   - fragmentation outside a continuation keeps the fragment's notes
//!     at their original surface values (rhythmic acceleration)
//!     instead of stretching them to fill the chord;
//!   - the strong-beat contract still classifies beats correctly when
//!     syncopated onsets are in play;
//!   - output stays deterministic.

use resonance_music_theory::{
    derive_motif_melody_with_section, phrase_grammar_roles, Chord, ChordQuality,
    ContourPreference, GeneratedNote, MelodyParams, MelodyStyle, Mode, MotifParams, MotifSource,
    PhraseGrammarRole, PitchClass, Scale, TimedChord,
};

const TPB: u64 = 480;

fn tc(root: PitchClass, quality: ChordQuality, start_beat: u32, duration_beats: u32) -> TimedChord {
    TimedChord {
        chord: Chord::new(root, quality),
        start_beat,
        duration_beats,
    }
}

fn generate(
    chords: &[TimedChord],
    phrase_len: u8,
    motif_len: u8,
    complexity: f32,
    seed: u64,
) -> Vec<GeneratedNote> {
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let params = MelodyParams {
        style: MelodyStyle::Motif,
        phrase_len,
        rest_density: 0.0,
        complexity,
        leap_chance: 0.3,
        contour: ContourPreference::Auto,
        ..MelodyParams::default()
    };
    let source = MotifSource::Generated(MotifParams {
        seed,
        complexity,
        motif_len,
        leap_chance: 0.3,
    });
    derive_motif_melody_with_section(chords, scale, &params, &source, seed, TPB as u32)
}

/// Onsets within `[start, start + span)`, relative to `start`.
fn onsets_in(notes: &[GeneratedNote], start: u64, span: u64) -> Vec<u64> {
    notes
        .iter()
        .filter(|n| (start..start + span).contains(&n.start_tick))
        .map(|n| n.start_tick - start)
        .collect()
}

// ---------------------------------------------------------------------------
// Tresillo cells
// ---------------------------------------------------------------------------

/// One 4-beat tonic chord = one phrase whose lone antecedent keeps the
/// Identity transform, so the rendered onsets are the raw rhythm
/// pattern scaled over 1920 ticks.
fn single_chord() -> Vec<TimedChord> {
    vec![tc(PitchClass::C, ChordQuality::Maj, 0, 4)]
}

#[test]
fn tresillo_cells_surface_at_high_complexity() {
    // 3-note motif over 1920 ticks: tresillo [3,3,2] lands on
    // {0, 720, 1440}, the rotations on {0, 720, 1200} and
    // {0, 480, 1200}. No base pattern produces these onset sets.
    let chords = single_chord();
    let signatures: [&[u64]; 3] = [&[0, 720, 1440], &[0, 720, 1200], &[0, 480, 1200]];
    let mut hits = [0usize; 3];
    for seed in 0..600u64 {
        let notes = generate(&chords, 1, 3, 1.0, seed);
        let onsets = onsets_in(&notes, 0, 4 * TPB);
        for (i, sig) in signatures.iter().enumerate() {
            if onsets.as_slice() == *sig {
                hits[i] += 1;
            }
        }
    }
    for (i, h) in hits.iter().enumerate() {
        assert!(
            *h >= 10,
            "tresillo signature {i} appeared only {h} times in 600 seeds"
        );
    }
}

#[test]
fn double_tresillo_surfaces_at_high_complexity() {
    // 6-note motif: double tresillo 3+3+3+3+2+2 over 1920 ticks lands
    // on {0, 360, 720, 1080, 1440, 1680} — unique among the pool.
    let chords = single_chord();
    let signature: &[u64] = &[0, 360, 720, 1080, 1440, 1680];
    let mut hits = 0usize;
    for seed in 0..600u64 {
        let notes = generate(&chords, 1, 6, 1.0, seed);
        if onsets_in(&notes, 0, 4 * TPB).as_slice() == signature {
            hits += 1;
        }
    }
    assert!(hits >= 10, "double tresillo appeared only {hits} times in 600 seeds");
}

#[test]
fn tresillo_cells_stay_out_of_low_complexity_motifs() {
    // Below the complexity floor the pattern pool is the simple end of
    // the base list — a 3-note motif at complexity 0.2 only renders
    // [1,1,1] or [2,1,1], never a tresillo onset set.
    let chords = single_chord();
    let tresillos: [&[u64]; 3] = [&[0, 720, 1440], &[0, 720, 1200], &[0, 480, 1200]];
    for seed in 0..300u64 {
        let notes = generate(&chords, 1, 3, 0.2, seed);
        let onsets = onsets_in(&notes, 0, 4 * TPB);
        for sig in &tresillos {
            assert_ne!(
                onsets.as_slice(),
                *sig,
                "seed {seed}: tresillo leaked into a low-complexity motif"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Straight syncopation
// ---------------------------------------------------------------------------

/// 8 chords / phrase_len 2 = 4 phrases. In sentence groups the second
/// phrase is the varied repeat of the basic idea; chord 2 (beat 8) is
/// its first chord.
fn eight_chords() -> Vec<TimedChord> {
    vec![
        tc(PitchClass::C, ChordQuality::Maj, 0, 4),
        tc(PitchClass::A, ChordQuality::Min, 4, 4),
        tc(PitchClass::F, ChordQuality::Maj, 8, 4),
        tc(PitchClass::G, ChordQuality::Maj, 12, 4),
        tc(PitchClass::C, ChordQuality::Maj, 16, 4),
        tc(PitchClass::A, ChordQuality::Min, 20, 4),
        tc(PitchClass::G, ChordQuality::Maj, 24, 4),
        tc(PitchClass::C, ChordQuality::Maj, 28, 4),
    ]
}

#[test]
fn varied_repeats_sometimes_syncopate_the_basic_idea() {
    // Complexity 0.5 keeps tresillo cells out of the pool, so any
    // onset difference between the basic idea (chord 0) and the varied
    // repeat (chord 2) can only come from Transform::Syncopate —
    // identity and transposition leave onsets untouched.
    let chords = eight_chords();
    let chord_span = 4 * TPB;
    let mut sentence_seeds = 0usize;
    let mut syncopated = 0usize;
    for seed in 0..400u64 {
        let roles = phrase_grammar_roles(4, seed);
        if roles[0] != PhraseGrammarRole::BasicIdea {
            continue;
        }
        sentence_seeds += 1;
        let notes = generate(&chords, 2, 4, 0.5, seed);
        let idea = onsets_in(&notes, 0, chord_span);
        let repeat = onsets_in(&notes, 8 * TPB, chord_span);
        if idea.len() < 2 || repeat.len() != idea.len() || repeat[1] == idea[1] {
            continue;
        }
        syncopated += 1;
        // Straight syncopation halves the first duration: the second
        // onset of the syncopated repeat sits at half the basic idea's
        // second onset (give or take integer-division rounding), and
        // every later onset is earlier than its counterpart.
        assert!(
            (repeat[1] as i64 * 2 - idea[1] as i64).abs() <= 2,
            "seed {seed}: repeat onset {} is not half of {}",
            repeat[1],
            idea[1]
        );
        for (r, i) in repeat.iter().zip(idea.iter()).skip(1) {
            assert!(
                r < i,
                "seed {seed}: syncopated onset {r} not earlier than {i}"
            );
        }
    }
    assert!(sentence_seeds >= 100, "too few sentence seeds ({sentence_seeds})");
    assert!(
        syncopated >= 15,
        "only {syncopated}/{sentence_seeds} varied repeats syncopated"
    );
}

// ---------------------------------------------------------------------------
// Rhythmic acceleration when fragmenting
// ---------------------------------------------------------------------------

#[test]
fn fragments_accelerate_instead_of_stretching() {
    // Period chains draw a fresh transform for the second antecedent
    // (phrase 2). When that draw is Fragment, the head motive used to
    // stretch to fill each chord at half the presentation's note
    // count; with acceleration the fragment keeps its original note
    // values, so every phrase now carries roughly the presentation's
    // density (continuations, which double it, don't occur in period
    // chains).
    let chords = eight_chords();
    let mut period_seeds = 0usize;
    for seed in 0..300u64 {
        let roles = phrase_grammar_roles(4, seed);
        if roles[0] != PhraseGrammarRole::Antecedent {
            continue;
        }
        period_seeds += 1;
        let notes = generate(&chords, 2, 6, 0.9, seed);
        let opening = onsets_in(&notes, 0, 8 * TPB).len();
        let second_antecedent = onsets_in(&notes, 16 * TPB, 8 * TPB).len();
        assert!(opening > 0, "seed {seed}: empty opening phrase");
        assert!(
            second_antecedent as f32 >= opening as f32 * 0.7,
            "seed {seed}: phrase 2 carries {second_antecedent} notes vs {opening} in the \
             opening — fragment stretched instead of accelerating"
        );
    }
    assert!(period_seeds >= 100, "too few period seeds ({period_seeds})");
}

// ---------------------------------------------------------------------------
// Strong-beat contract under syncopated onsets
// ---------------------------------------------------------------------------

/// Index of the chord sounding at `tick`.
fn chord_index(chords: &[TimedChord], tick: u64) -> usize {
    chords
        .iter()
        .rposition(|c| c.start_beat as u64 * TPB <= tick)
        .unwrap_or(0)
}

fn is_chord_tone(chords: &[TimedChord], tick: u64, note: u8) -> bool {
    chords[chord_index(chords, tick)]
        .chord
        .pitch_classes()
        .any(|pc| pc.to_semitone() == note % 12)
}

fn is_strong(chords: &[TimedChord], tick: u64) -> bool {
    let start = chords[chord_index(chords, tick)].start_beat as u64 * TPB;
    (tick - start).is_multiple_of(2 * TPB)
}

#[test]
fn strong_beat_contract_holds_with_syncopated_rhythms() {
    // Full-complexity motifs draw tresillo cells and syncopation
    // transforms; onsets displaced off the strong-beat grid must be
    // classified as weak, and any note still landing on a strong beat
    // is a chord tone or a dissonance resolving by step.
    //
    // The engine enforces the contract *per phrase* (8 beats here:
    // phrase_len 2 over 4-beat chords); a phrase-final strong-beat
    // note has no in-phrase successor to resolve to, so — like the
    // engine's own `strong_beats_ok` repairs — the check skips pairs
    // that straddle a phrase boundary.
    let chords = eight_chords();
    let phrase_ticks = 8 * TPB;
    for seed in 0..200u64 {
        let notes = generate(&chords, 2, 0, 1.0, seed);
        assert!(!notes.is_empty(), "seed {seed}: empty melody");
        for i in 0..notes.len() {
            let n = &notes[i];
            if !is_strong(&chords, n.start_tick) || is_chord_tone(&chords, n.start_tick, n.note) {
                continue;
            }
            let Some(next) = notes.get(i + 1) else {
                continue; // section-final note: per-phrase contract scope
            };
            if next.start_tick / phrase_ticks != n.start_tick / phrase_ticks {
                continue; // phrase-final note: resolution is out of scope
            }
            let step = (next.note as i16 - n.note as i16).abs();
            assert!(
                (1..=2).contains(&step),
                "seed {seed}: strong-beat dissonance {} at tick {} resolves by {step}",
                n.note,
                n.start_tick
            );
        }
    }
}

#[test]
fn rhythm_transforms_keep_generation_deterministic() {
    let chords = eight_chords();
    for seed in [3u64, 77, 0xC0FFEE] {
        let a = generate(&chords, 2, 0, 1.0, seed);
        let b = generate(&chords, 2, 0, 1.0, seed);
        assert_eq!(a, b, "seed {seed}: nondeterministic output");
    }
}
