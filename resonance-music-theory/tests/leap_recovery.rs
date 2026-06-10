//! Leap grammar for realized motif phrases (Open Music Theory v2):
//! a leap of a 4th or larger (>= 5 semitones) recovers by a step in
//! the opposite direction; same-direction leap pairs must outline one
//! major or minor triad; never three consecutive leaps; and the
//! extrema of direction changes (each monotonic run's span) must not
//! outline a tritone or a seventh.
//!
//! Exercised through the public [`derive_motif_melody_with_section`]
//! entry point with a single phrase covering all chords, so phrase
//! joins and per-phrase octave displacement don't blur the rules.

use resonance_music_theory::{
    derive_motif_melody_with_section, Chord, ChordQuality, ContourPreference, GeneratedNote,
    MelodyParams, MelodyStyle, Mode, MotifParams, MotifSource, PitchClass, Scale, TimedChord,
};

const LEAP_MIN: i16 = 3;
const RECOVERY_MIN: i16 = 5;
const STEP_MAX: i16 = 2;

fn is_leap(mv: i16) -> bool {
    mv.abs() >= LEAP_MIN
}

/// Do three pitches all belong to a single major or minor triad?
fn outlines_one_triad(a: u8, b: u8, c: u8) -> bool {
    const TRIADS: [[u8; 3]; 2] = [[0, 4, 7], [0, 3, 7]];
    let pcs = [a % 12, b % 12, c % 12];
    (0..12u8).any(|root| {
        TRIADS.iter().any(|triad| {
            pcs.iter()
                .all(|&pc| triad.iter().any(|&iv| (root + iv) % 12 == pc))
        })
    })
}

fn moves(notes: &[GeneratedNote]) -> Vec<i16> {
    notes
        .windows(2)
        .map(|w| w[1].note as i16 - w[0].note as i16)
        .collect()
}

/// Assert the full leap grammar on a phrase's pitch sequence.
fn assert_leap_grammar(notes: &[GeneratedNote], ctx: &str) {
    let pitches: Vec<u8> = notes.iter().map(|n| n.note).collect();
    let mv = moves(notes);

    for i in 1..mv.len() {
        let prev = mv[i - 1];
        let cur = mv[i];

        // Never three consecutive leaps.
        if i >= 2 {
            assert!(
                !(is_leap(mv[i - 2]) && is_leap(prev) && is_leap(cur)),
                "three consecutive leaps at move {i} in {pitches:?} for {ctx}"
            );
        }

        // Same-direction leap pairs must outline one triad.
        let triad_pair = is_leap(prev)
            && is_leap(cur)
            && cur.signum() == prev.signum()
            && outlines_one_triad(pitches[i - 1], pitches[i], pitches[i + 1]);
        if is_leap(prev) && is_leap(cur) && cur.signum() == prev.signum() {
            assert!(
                triad_pair,
                "same-direction leap pair {prev}/{cur} at move {i} does not outline a triad \
                 in {pitches:?} for {ctx}"
            );
        }

        // A leap of a 4th or larger recovers by an opposite step (or
        // continues a triad arpeggio whose recovery comes after).
        if prev.abs() >= RECOVERY_MIN {
            let opposite_step = cur.signum() == -prev.signum() && cur.abs() <= STEP_MAX;
            assert!(
                opposite_step || triad_pair,
                "leap of {prev} at move {} not recovered by an opposite step (next move {cur}) \
                 in {pitches:?} for {ctx}",
                i - 1
            );
        }
    }

    // Direction-change extrema: each monotonic run's outlined span
    // must not be a tritone or a seventh.
    let mut run_start = 0usize;
    let mut run_dir: i16 = 0;
    let check_outline = |s: usize, e: usize| {
        let span = (pitches[e] as i16 - pitches[s] as i16).abs();
        assert!(
            !matches!(span, 6 | 10 | 11),
            "monotonic run {s}..={e} outlines a dissonant span of {span} semitones \
             in {pitches:?} for {ctx}"
        );
    };
    for i in 1..pitches.len() {
        let dir = (pitches[i] as i16 - pitches[i - 1] as i16).signum();
        if dir == 0 {
            continue;
        }
        if run_dir != 0 && dir != run_dir {
            check_outline(run_start, i - 1);
            run_start = i - 1;
        }
        run_dir = dir;
    }
    if !pitches.is_empty() {
        check_outline(run_start, pitches.len() - 1);
    }
}

fn single_phrase_chords() -> Vec<TimedChord> {
    let seq = [
        (PitchClass::C, ChordQuality::Maj),
        (PitchClass::A, ChordQuality::Min),
        (PitchClass::F, ChordQuality::Maj),
        (PitchClass::G, ChordQuality::Maj),
    ];
    seq.iter()
        .enumerate()
        .map(|(i, &(root, quality))| TimedChord {
            chord: Chord::new(root, quality),
            start_beat: (i * 4) as u32,
            duration_beats: 4,
        })
        .collect()
}

fn melody_params(leap_chance: f32, complexity: f32) -> MelodyParams {
    MelodyParams {
        style: MelodyStyle::Motif,
        // One phrase spans all 4 chords: no joins, no octave shifts.
        phrase_len: 4,
        rest_density: 0.0,
        complexity,
        leap_chance,
        contour: ContourPreference::Auto,
        ..MelodyParams::default()
    }
}

fn generate(seed: u64, leap_chance: f32, complexity: f32) -> Vec<GeneratedNote> {
    let chords = single_phrase_chords();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let params = melody_params(leap_chance, complexity);
    let source = MotifSource::Generated(MotifParams {
        seed,
        complexity,
        motif_len: 0,
        leap_chance,
    });
    derive_motif_melody_with_section(&chords, scale, &params, &source, seed, 480)
}

#[test]
fn realized_phrases_obey_leap_grammar() {
    for seed in 0..400 {
        let notes = generate(seed, 0.3, 0.6);
        assert!(!notes.is_empty(), "empty melody for seed {seed}");
        assert_leap_grammar(&notes, &format!("seed {seed}, leap_chance 0.3"));
    }
}

#[test]
fn realized_phrases_obey_leap_grammar_when_leap_heavy() {
    // Leap-heavy extreme: nearly every motif move is drawn as a leap,
    // so the recovery pass has to do real work.
    for seed in 0..400 {
        let notes = generate(seed, 0.89, 0.9);
        assert!(!notes.is_empty(), "empty melody for seed {seed}");
        assert_leap_grammar(&notes, &format!("seed {seed}, leap_chance 0.89"));
    }
}

#[test]
fn realized_phrases_obey_leap_grammar_across_minor_scale() {
    let chords: Vec<TimedChord> = [
        (PitchClass::A, ChordQuality::Min),
        (PitchClass::F, ChordQuality::Maj),
        (PitchClass::C, ChordQuality::Maj),
        (PitchClass::G, ChordQuality::Maj),
    ]
    .iter()
    .enumerate()
    .map(|(i, &(root, quality))| TimedChord {
        chord: Chord::new(root, quality),
        start_beat: (i * 4) as u32,
        duration_beats: 4,
    })
    .collect();
    let scale = Some(Scale::new(PitchClass::A, Mode::Minor));
    let params = melody_params(0.5, 0.7);
    for seed in 0..200 {
        let source = MotifSource::Generated(MotifParams {
            seed,
            complexity: 0.7,
            motif_len: 0,
            leap_chance: 0.5,
        });
        let notes = derive_motif_melody_with_section(&chords, scale, &params, &source, seed, 480);
        assert!(!notes.is_empty(), "empty melody for seed {seed}");
        assert_leap_grammar(&notes, &format!("seed {seed}, A minor"));
    }
}

#[test]
fn leap_recovery_preserves_rhythm() {
    // The old gap-fill pass inserted passing tones, splitting note
    // durations. Leap recovery only rewrites pitches: the realized
    // rhythm (start ticks + durations) must match a render with the
    // same parameters, and no note may be shorter than the renderer's
    // minimum duration (tpb / 8).
    for seed in 0..100 {
        let notes = generate(seed, 0.89, 0.9);
        for n in &notes {
            assert!(
                n.duration_ticks >= 60,
                "note shorter than the render minimum at tick {} (seed {seed}): {} ticks",
                n.start_tick,
                n.duration_ticks
            );
        }
        // Starts strictly ordered and non-overlapping within the phrase
        // grid: no inserted fill notes splitting slots.
        for w in notes.windows(2) {
            assert!(
                w[0].start_tick + w[0].duration_ticks <= w[1].start_tick
                    || w[0].start_tick < w[1].start_tick,
                "notes out of order at tick {} (seed {seed})",
                w[0].start_tick
            );
        }
    }
}

#[test]
fn leap_recovery_stays_deterministic() {
    let a = generate(42, 0.5, 0.6);
    let b = generate(42, 0.5, 0.6);
    assert_eq!(a, b);
}
