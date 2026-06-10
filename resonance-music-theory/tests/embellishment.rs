//! Embellishing-tone decoration pass (Open Music Theory v2,
//! embellishing tones): the motif engine generates a chord-tone
//! skeleton on strong beats and decorates the surface from the OMT
//! table (passing/neighbor on weak beats; appoggiatura/suspension on
//! strong beats; escape tone; anticipation) with style-weighted
//! probabilities.
//!
//! Contracts under test, on the public generator output:
//!   - dissonance discipline: a non-chord tone is never both leaped
//!     into and left by leap;
//!   - strong-beat contract (evolved from "strong beats are chord
//!     tones"): a strong-beat note is a chord tone, or a dissonance
//!     resolving by step on the very next note;
//!   - style weighting: pop ballad places strong-beat dissonances
//!     where folk (whose appoggiatura/suspension weights are zero)
//!     adds none beyond the repair-pass baseline, and jazz
//!     anticipates across chord boundaries more than folk;
//!   - density scales with complexity (measured against the folk
//!     baseline, which shares the identical pre-decoration phrase);
//!   - output is deterministic.
//!
//! Single-phrase progressions (phrase_len = number of chords) keep the
//! per-phrase octave displacement and rest-density filters out of the
//! picture, mirroring tests/leap_recovery.rs.

use resonance_music_theory::{
    derive_motif_melody_with_section, Chord, ChordQuality, ContourPreference, EmbellishmentStyle,
    GeneratedNote, MelodyParams, MelodyStyle, Mode, MotifParams, MotifSource, PitchClass, Scale,
    TimedChord,
};

const TPB: u64 = 480;
const LEAP_MIN: i16 = 3;

fn chords() -> Vec<TimedChord> {
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

fn generate(
    seed: u64,
    style: EmbellishmentStyle,
    complexity: f32,
    leap_chance: f32,
) -> Vec<GeneratedNote> {
    let chords = chords();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let params = MelodyParams {
        style: MelodyStyle::Motif,
        phrase_len: 4, // one phrase spans all chords
        rest_density: 0.0,
        complexity,
        leap_chance,
        contour: ContourPreference::Auto,
        embellishment: style,
        ..MelodyParams::default()
    };
    let source = MotifSource::Generated(MotifParams {
        seed,
        complexity,
        motif_len: 0,
        leap_chance,
    });
    derive_motif_melody_with_section(&chords, scale, &params, &source, seed, TPB as u32)
}

/// Index of the chord sounding at `tick`.
fn chord_index(chords: &[TimedChord], tick: u64) -> usize {
    chords
        .iter()
        .rposition(|tc| tc.start_beat as u64 * TPB <= tick)
        .unwrap_or(0)
}

fn is_chord_tone(chords: &[TimedChord], tick: u64, note: u8) -> bool {
    chords[chord_index(chords, tick)]
        .chord
        .pitch_classes()
        .any(|pc| pc.to_semitone() == note % 12)
}

/// Is `tick` a strong beat (multiple of 2 beats from its chord start)?
fn is_strong(chords: &[TimedChord], tick: u64) -> bool {
    let start = chords[chord_index(chords, tick)].start_beat as u64 * TPB;
    (tick - start).is_multiple_of(2 * TPB)
}

#[test]
fn dissonances_are_never_leaped_both_into_and_out_of() {
    let chords = chords();
    for style in [
        EmbellishmentStyle::Folk,
        EmbellishmentStyle::PopBallad,
        EmbellishmentStyle::Jazz,
    ] {
        for &leap_chance in &[0.3, 0.8] {
            for seed in 0..200 {
                let notes = generate(seed, style, 0.8, leap_chance);
                assert!(!notes.is_empty(), "empty melody for seed {seed}");
                for i in 1..notes.len().saturating_sub(1) {
                    if is_chord_tone(&chords, notes[i].start_tick, notes[i].note) {
                        continue;
                    }
                    let leap_in =
                        (notes[i].note as i16 - notes[i - 1].note as i16).abs() >= LEAP_MIN;
                    let leap_out =
                        (notes[i + 1].note as i16 - notes[i].note as i16).abs() >= LEAP_MIN;
                    assert!(
                        !(leap_in && leap_out),
                        "dissonance {} at tick {} leaped both into ({} -> {}) and out of \
                         ({} -> {}) for seed {seed}, style {style:?}, leap_chance {leap_chance}",
                        notes[i].note,
                        notes[i].start_tick,
                        notes[i - 1].note,
                        notes[i].note,
                        notes[i].note,
                        notes[i + 1].note
                    );
                }
            }
        }
    }
}

#[test]
fn strong_beat_dissonances_resolve_by_step() {
    let chords = chords();
    for style in [
        EmbellishmentStyle::Folk,
        EmbellishmentStyle::PopBallad,
        EmbellishmentStyle::Jazz,
    ] {
        for seed in 0..200 {
            let notes = generate(seed, style, 0.9, 0.3);
            for i in 0..notes.len() {
                let n = &notes[i];
                if !is_strong(&chords, n.start_tick)
                    || is_chord_tone(&chords, n.start_tick, n.note)
                {
                    continue;
                }
                let next = notes.get(i + 1).unwrap_or_else(|| {
                    panic!(
                        "strong-beat dissonance {} at tick {} has no resolution \
                         (seed {seed}, style {style:?})",
                        n.note, n.start_tick
                    )
                });
                let resolution = (n.note as i16 - next.note as i16).abs();
                assert!(
                    (1..=2).contains(&resolution),
                    "strong-beat dissonance {} at tick {} does not resolve by step \
                     (next {}, seed {seed}, style {style:?})",
                    n.note,
                    n.start_tick,
                    next.note
                );
            }
        }
    }
}

/// Count strong-beat dissonances — only appoggiaturas and suspensions
/// produce them, so this is the pop-ballad signature.
fn strong_beat_dissonances(notes: &[GeneratedNote], chords: &[TimedChord]) -> usize {
    notes
        .iter()
        .filter(|n| {
            is_strong(chords, n.start_tick) && !is_chord_tone(chords, n.start_tick, n.note)
        })
        .count()
}

/// Count anticipation events: a note repeating the next note's pitch
/// across a chord boundary, dissonant against its own chord and
/// consonant in the next — the jazz signature.
fn anticipations(notes: &[GeneratedNote], chords: &[TimedChord]) -> usize {
    notes
        .windows(2)
        .filter(|w| {
            w[0].note == w[1].note
                && chord_index(chords, w[0].start_tick) != chord_index(chords, w[1].start_tick)
                && !is_chord_tone(chords, w[0].start_tick, w[0].note)
                && is_chord_tone(chords, w[1].start_tick, w[1].note)
        })
        .count()
}

#[test]
fn decoration_density_varies_by_style() {
    // For a given (seed, complexity) the pre-decoration phrase is
    // identical across styles — only the decoration pass differs — so
    // folk (appoggiatura/suspension weights of zero) is the exact
    // baseline for strong-beat dissonance counts, and pop ballad's
    // surplus is wholly attributable to its strong-beat decorations.
    // Same logic for jazz's anticipation surplus over folk
    // (anticipation weight zero).
    let chords = chords();
    let mut ballad_strong = 0usize;
    let mut folk_strong = 0usize;
    let mut jazz_ant = 0usize;
    let mut folk_ant = 0usize;
    for seed in 0..200 {
        ballad_strong += strong_beat_dissonances(
            &generate(seed, EmbellishmentStyle::PopBallad, 0.9, 0.3),
            &chords,
        );
        folk_strong +=
            strong_beat_dissonances(&generate(seed, EmbellishmentStyle::Folk, 0.9, 0.3), &chords);
        jazz_ant += anticipations(&generate(seed, EmbellishmentStyle::Jazz, 0.9, 0.3), &chords);
        folk_ant += anticipations(&generate(seed, EmbellishmentStyle::Folk, 0.9, 0.3), &chords);
    }
    assert!(
        ballad_strong > folk_strong,
        "pop ballad strong-beat dissonances ({ballad_strong}) not above the folk \
         baseline ({folk_strong})"
    );
    assert!(
        jazz_ant > folk_ant,
        "jazz anticipations ({jazz_ant}) not above folk baseline ({folk_ant})"
    );
}

#[test]
fn decoration_density_scales_with_complexity() {
    // The density factor is 0.35 + 0.65 * complexity. Complexity also
    // changes the motif itself, so compare *decoration-attributable*
    // counts (pop ballad minus the folk baseline at the same seed and
    // complexity): the low-complexity surplus stays below the
    // high-complexity surplus, aggregated across seeds.
    let chords = chords();
    let mut low = 0isize;
    let mut high = 0isize;
    for seed in 0..200 {
        low += strong_beat_dissonances(
            &generate(seed, EmbellishmentStyle::PopBallad, 0.05, 0.3),
            &chords,
        ) as isize
            - strong_beat_dissonances(&generate(seed, EmbellishmentStyle::Folk, 0.05, 0.3), &chords)
                as isize;
        high += strong_beat_dissonances(
            &generate(seed, EmbellishmentStyle::PopBallad, 0.95, 0.3),
            &chords,
        ) as isize
            - strong_beat_dissonances(&generate(seed, EmbellishmentStyle::Folk, 0.95, 0.3), &chords)
                as isize;
    }
    assert!(
        low < high,
        "low-complexity decoration surplus ({low}) not below high-complexity ({high})"
    );
}

#[test]
fn decoration_is_deterministic() {
    for style in [
        EmbellishmentStyle::Auto,
        EmbellishmentStyle::Folk,
        EmbellishmentStyle::PopBallad,
        EmbellishmentStyle::Jazz,
    ] {
        let a = generate(42, style, 0.7, 0.4);
        let b = generate(42, style, 0.7, 0.4);
        assert_eq!(a, b, "non-deterministic output for {style:?}");
    }
}
