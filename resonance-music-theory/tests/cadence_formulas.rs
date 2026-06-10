//! Goal-cadence formula targeting (Open Music Theory v2: intro-to-
//! harmony / strengthening-endings-with-v7): every phrase gets a goal
//! cadence — weak (HC, sometimes IAC) for antecedents, strong (PAC,
//! ~10% deceptive) for consequents — and the final two melody notes
//! are forced to a two-note formula compatible with the chord:
//! 2→1 / 7→1 (PAC), ends-on-3/5 (IAC), 1→7 / 3→2 (HC), lands-on-6
//! (deceptive). Exercised through the public generator entry points
//! for both the instrumental motif engine and the vocal styles.
//!
//! The overlay is validated, so it composes with the leap grammar
//! (tests/leap_recovery.rs), the single-climax rule
//! (tests/phrase_climax.rs, tests/vocal_climax.rs), the strong-beat
//! chord-tone contract (tests/derive_basics.rs), and the motif line
//! rules (tests/motif_line_rules.rs) — those suites run on the same
//! generator output and stay authoritative for their invariants.
//! Because a phrase keeps its old ending when no formula realization
//! passes validation (most commonly: the penult sits on a strong beat,
//! where the chord-tone contract excludes the non-chord approach
//! tones), the instrumental assertions here are distributional.

use resonance_music_theory::{
    count_syllables, derive_motif_melody_with_section, derive_vocal, generate_lyrics, Chord,
    ChordQuality, ContourPreference, GeneratedNote, MelodyParams, MelodyStyle, Mode, MotifParams,
    MotifSource, PitchClass, Scale, TimedChord, VocalParams,
};

// C-major degree pitch classes.
const PC_C: u8 = 0; // degree 1
const PC_D: u8 = 2; // degree 2
const PC_E: u8 = 4; // degree 3
const PC_F: u8 = 5; // degree 4
const PC_G: u8 = 7; // degree 5
const PC_A: u8 = 9; // degree 6
const PC_B: u8 = 11; // degree 7

/// The full two-note formula table as C-major pitch-class pairs
/// (penult_pc, final_pc): PAC 2→1, 7→1; IAC 4→3, 2→3, 6→5, 4→5;
/// HC 1→7, 3→2; deceptive 7→6, 5→6.
const FORMULA_PCS: [(u8, u8); 10] = [
    (PC_D, PC_C),
    (PC_B, PC_C),
    (PC_F, PC_E),
    (PC_D, PC_E),
    (PC_A, PC_G),
    (PC_F, PC_G),
    (PC_C, PC_B),
    (PC_E, PC_D),
    (PC_B, PC_A),
    (PC_G, PC_A),
];

fn is_formula_pair(penult: u8, fin: u8) -> bool {
    FORMULA_PCS
        .iter()
        .any(|&(p, f)| penult % 12 == p && fin % 12 == f)
}

fn tc(chord: Chord, start_beat: u32, duration_beats: u32) -> TimedChord {
    TimedChord {
        chord,
        start_beat,
        duration_beats,
    }
}

fn maj(root: PitchClass) -> Chord {
    Chord::new(root, ChordQuality::Maj)
}

fn min(root: PitchClass) -> Chord {
    Chord::new(root, ChordQuality::Min)
}

fn melody_params(phrase_len: u8) -> MelodyParams {
    MelodyParams {
        style: MelodyStyle::Motif,
        phrase_len,
        rest_density: 0.0,
        complexity: 0.6,
        leap_chance: 0.3,
        contour: ContourPreference::Auto,
        ..MelodyParams::default()
    }
}

fn generate_melody(chords: &[TimedChord], phrase_len: u8, seed: u64) -> Vec<GeneratedNote> {
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let params = melody_params(phrase_len);
    let source = MotifSource::Generated(MotifParams {
        seed,
        complexity: 0.6,
        motif_len: 0,
        leap_chance: 0.3,
    });
    derive_motif_melody_with_section(chords, scale, &params, &source, seed, 480)
}

/// One antecedent phrase (phrase 0) ending on G major (V of C). The HC
/// finals (7, 2) and the IAC final 5 are all chord tones of V, so the
/// open ending always has a compatible formula available.
fn antecedent_over_dominant() -> Vec<TimedChord> {
    vec![
        tc(maj(PitchClass::C), 0, 4),
        tc(min(PitchClass::A), 4, 4),
        tc(maj(PitchClass::F), 8, 4),
        tc(maj(PitchClass::G), 12, 4),
    ]
}

/// Two phrases (phrase_len 2): the second — consequent — ends on the
/// C-major tonic chord, so the PAC final (degree 1) is compatible and
/// the deceptive landing (degree 6) passes the clash guard.
fn consequent_over_tonic() -> Vec<TimedChord> {
    vec![
        tc(maj(PitchClass::C), 0, 4),
        tc(maj(PitchClass::F), 4, 4),
        tc(maj(PitchClass::G), 8, 4),
        tc(maj(PitchClass::C), 12, 4),
    ]
}

// ---------------------------------------------------------------------------
// Instrumental motif engine
// ---------------------------------------------------------------------------

#[test]
fn antecedent_phrases_end_open_over_the_dominant() {
    // Antecedents target HC (1→7 / 3→2) with an IAC fallback, so the
    // final lands on an open degree — 7, 2, or 5 — over V. The single
    // phrase keeps per-phrase octave displacement out of the picture.
    let chords = antecedent_over_dominant();
    let mut open = 0usize;
    let mut pairs = 0usize;
    let mut total = 0usize;
    for seed in 0..300u64 {
        let notes = generate_melody(&chords, 4, seed);
        let n = notes.len();
        if n < 2 {
            continue;
        }
        total += 1;
        let fin = notes[n - 1].note % 12;
        let pen = notes[n - 2].note % 12;
        if [PC_B, PC_D, PC_G].contains(&fin) {
            open += 1;
        }
        if is_formula_pair(pen, fin)
            && (notes[n - 1].note as i16 - notes[n - 2].note as i16).abs() <= 3
        {
            pairs += 1;
        }
    }
    assert!(total > 250, "too few phrases generated ({total})");
    // Final-degree targeting succeeds even when the full pair can't be
    // placed (final-note fallback), so it holds for the large majority.
    assert!(
        open as f32 >= total as f32 * 0.80,
        "only {open}/{total} antecedent phrases ended on an open degree (7/2/5)"
    );
    // The full two-note formula needs a weak-beat penult (strong beats
    // must stay chord tones), so assert a healthy share, not totality.
    assert!(
        pairs as f32 >= total as f32 * 0.40,
        "only {pairs}/{total} antecedent phrases ended with a two-note cadence formula"
    );
}

#[test]
fn consequent_phrases_close_on_the_tonic_with_occasional_deceptive_endings() {
    let chords = consequent_over_tonic();
    let mut hist = [0usize; 12];
    let mut total = 0usize;
    for seed in 0..300u64 {
        let notes = generate_melody(&chords, 2, seed);
        let n = notes.len();
        if n < 2 {
            continue;
        }
        total += 1;
        hist[(notes[n - 1].note % 12) as usize] += 1;
    }
    assert!(total > 250, "too few phrases generated ({total})");
    let tonic = hist[PC_C as usize];
    let deceptive = hist[PC_A as usize];
    let formula_finals: usize = [PC_C, PC_D, PC_E, PC_G, PC_A, PC_B]
        .iter()
        .map(|&pc| hist[pc as usize])
        .sum();
    assert!(
        tonic as f32 >= total as f32 * 0.55,
        "only {tonic}/{total} consequent phrases closed on the tonic ({hist:?})"
    );
    assert!(
        deceptive as f32 >= total as f32 * 0.02 && deceptive as f32 <= total as f32 * 0.20,
        "deceptive endings (degree 6) off target: {deceptive}/{total} ({hist:?})"
    );
    assert!(
        formula_finals as f32 >= total as f32 * 0.90,
        "only {formula_finals}/{total} consequent phrases ended on a formula degree ({hist:?})"
    );
}

#[test]
fn cadence_targeting_stays_deterministic() {
    let chords = consequent_over_tonic();
    let a = generate_melody(&chords, 2, 42);
    let b = generate_melody(&chords, 2, 42);
    assert_eq!(a, b);
}

// ---------------------------------------------------------------------------
// Vocal styles
// ---------------------------------------------------------------------------

/// Per-line note slices, recovered the same way the generator and the
/// SVS pipeline do.
fn line_slices<'a>(notes: &'a [GeneratedNote], params: &VocalParams) -> Vec<&'a [GeneratedNote]> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for line in &params.draft {
        let n = (count_syllables(&line.text) as usize).min(notes.len().saturating_sub(cursor));
        if n == 0 {
            continue;
        }
        out.push(&notes[cursor..cursor + n]);
        cursor += n;
    }
    out
}

/// All-tonic progression: holding the chord on C major makes the
/// expected cadence degrees deterministic — antecedent lines fall back
/// from HC (7/2 don't fit the chord) to IAC (3/5, both chord tones),
/// consequent lines take PAC (1) with the ~10% deceptive 6.
fn c_major_chords() -> Vec<TimedChord> {
    (0..4).map(|i| tc(maj(PitchClass::C), i * 4, 4)).collect()
}

#[test]
fn vocal_lines_land_on_goal_cadence_degrees() {
    let chords = c_major_chords();
    let mut ant_ok = 0usize;
    let mut ant_total = 0usize;
    let mut cons_tonic = 0usize;
    let mut cons_deceptive = 0usize;
    let mut cons_total = 0usize;
    let mut pairs = 0usize;
    let mut pair_total = 0usize;
    for seed in 0..60u64 {
        let mut p = VocalParams::default();
        p.draft = generate_lyrics(&p, seed.wrapping_add(3));
        let notes = derive_vocal(&chords, &p, 480, seed);
        assert!(!notes.is_empty(), "empty vocal for seed {seed}");
        for (li, slice) in line_slices(&notes, &p).iter().enumerate() {
            let n = slice.len();
            if n < 2 {
                continue;
            }
            let fin = slice[n - 1].note % 12;
            let pen = slice[n - 2].note % 12;
            if li % 2 == 0 {
                // Antecedent: open-ish IAC landing over the tonic chord.
                ant_total += 1;
                if [PC_E, PC_G].contains(&fin) {
                    ant_ok += 1;
                }
            } else {
                // Consequent: PAC tonic, occasionally deceptive 6.
                cons_total += 1;
                if fin == PC_C {
                    cons_tonic += 1;
                } else if fin == PC_A {
                    cons_deceptive += 1;
                }
            }
            pair_total += 1;
            if is_formula_pair(pen, fin) {
                pairs += 1;
            }
        }
    }
    assert!(ant_total >= 100 && cons_total >= 100, "too few lines");
    assert!(
        ant_ok as f32 >= ant_total as f32 * 0.85,
        "only {ant_ok}/{ant_total} antecedent lines ended on 3/5"
    );
    assert!(
        cons_tonic as f32 >= cons_total as f32 * 0.60,
        "only {cons_tonic}/{cons_total} consequent lines closed on the tonic"
    );
    assert!(
        cons_deceptive as f32 >= cons_total as f32 * 0.03
            && cons_deceptive as f32 <= cons_total as f32 * 0.25,
        "deceptive endings off target: {cons_deceptive}/{cons_total}"
    );
    // The post-line formula pass rewrites the penult to the formula's
    // approach tone (resolving the tendency tones 7→1, 4→3, 2→1 by
    // construction), so nearly every line ends in a full table pair.
    assert!(
        pairs as f32 >= pair_total as f32 * 0.90,
        "only {pairs}/{pair_total} vocal lines ended with a two-note cadence formula"
    );
}

#[test]
fn vocal_minor_consequents_close_on_the_minor_tonic() {
    // A-minor everywhere: PAC final = A. The deceptive landing (degree
    // 6 = F) sits a semitone above the chord's fifth (E), so the
    // clash guard rejects it and closes stay authentic. Antecedents
    // fall back to IAC over the tonic chord: 3 (C) or 5 (E).
    let chords: Vec<TimedChord> = (0..4).map(|i| tc(min(PitchClass::A), i * 4, 4)).collect();
    let mut cons_tonic = 0usize;
    let mut cons_total = 0usize;
    let mut ant_ok = 0usize;
    let mut ant_total = 0usize;
    for seed in 0..40u64 {
        let mut p = VocalParams::default();
        p.draft = generate_lyrics(&p, seed.wrapping_add(11));
        let notes = derive_vocal(&chords, &p, 480, seed);
        for (li, slice) in line_slices(&notes, &p).iter().enumerate() {
            let n = slice.len();
            if n < 2 {
                continue;
            }
            let fin = slice[n - 1].note % 12;
            if li % 2 == 1 {
                cons_total += 1;
                if fin == 9 {
                    cons_tonic += 1; // A
                }
            } else {
                ant_total += 1;
                if fin == 0 || fin == 4 {
                    ant_ok += 1; // C or E
                }
            }
        }
    }
    assert!(cons_total >= 60 && ant_total >= 60, "too few lines");
    assert!(
        cons_tonic as f32 >= cons_total as f32 * 0.70,
        "only {cons_tonic}/{cons_total} minor consequent lines closed on A"
    );
    assert!(
        ant_ok as f32 >= ant_total as f32 * 0.80,
        "only {ant_ok}/{ant_total} minor antecedent lines ended on 3/5"
    );
}

#[test]
fn vocal_cadence_targeting_stays_deterministic() {
    let chords = c_major_chords();
    let mut p = VocalParams::default();
    p.draft = generate_lyrics(&p, 9);
    let a = derive_vocal(&chords, &p, 480, 9);
    let b = derive_vocal(&chords, &p, 480, 9);
    assert_eq!(a, b);
}
