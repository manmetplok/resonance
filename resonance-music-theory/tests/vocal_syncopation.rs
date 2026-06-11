//! Division-level syncopation on stressed vocal syllables: instead of
//! the pure grid + continuous micro-rubato, a primary-stress syllable
//! may anticipate its slot by a quantized half or quarter division.
//!
//! Asserted through the public `derive_vocal` entry point:
//!   - pop-adjacent styles show inter-onset deltas that deviate from
//!     the line's grid step by far more than the rubato wobble allows;
//!   - hymnal stays on its strict grid (no syncopation, no rubato);
//!   - notes never overlap and lines stay in order;
//!   - output stays deterministic.

use resonance_music_theory::{
    count_syllables, derive_vocal, generate_lyrics, Chord, ChordQuality, GeneratedNote,
    PitchClass, TimedChord, VocalParams, VocalStyle,
};

const TPB: u32 = 480;

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

/// Per-line note slices, recovered the same way the generator and the
/// SVS pipeline do (mechanical syllable counts).
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

/// Inter-onset deltas within one line.
fn deltas(line: &[GeneratedNote]) -> Vec<i64> {
    line.windows(2)
        .map(|w| w[1].start_tick as i64 - w[0].start_tick as i64)
        .collect()
}

fn median(values: &[i64]) -> i64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

#[test]
fn stressed_syllables_break_the_uniform_grid() {
    // The pop-ballad rubato wobble is at most ±5 % of the slot, so an
    // inter-onset delta deviating from the line's median step by more
    // than 20 % can only come from a division-level anticipation
    // (half or quarter slot). Aggregated across seeds there must be
    // plenty of them.
    let chords = chords();
    let mut syncopated_deltas = 0usize;
    let mut lines_seen = 0usize;
    for seed in 0..30u64 {
        let mut p = VocalParams::default();
        p.draft = generate_lyrics(&p, seed.wrapping_add(11));
        let notes = derive_vocal(&chords, &p, TPB, seed);
        for line in line_slices(&notes, &p) {
            let d = deltas(line);
            if d.len() < 3 {
                continue;
            }
            lines_seen += 1;
            let step = median(&d).max(1);
            syncopated_deltas += d
                .iter()
                .filter(|&&delta| (delta - step).abs() as f32 > step as f32 * 0.2)
                .count();
        }
    }
    assert!(lines_seen >= 60, "too few usable lines ({lines_seen})");
    assert!(
        syncopated_deltas >= 10,
        "only {syncopated_deltas} syncopated onsets across {lines_seen} lines — \
         stressed syllables are still glued to the grid"
    );
}

#[test]
fn hymnal_keeps_its_strict_grid() {
    // Hymnal opts out of both rubato and stress syncopation: every
    // intra-line delta equals the line's grid step (±2 ticks of
    // float-to-tick rounding).
    let chords = chords();
    for seed in 0..20u64 {
        let mut p = VocalParams::default();
        p.style = VocalStyle::Hymnal;
        p.draft = generate_lyrics(&p, seed.wrapping_add(23));
        let notes = derive_vocal(&chords, &p, TPB, seed);
        for line in line_slices(&notes, &p) {
            let d = deltas(line);
            if d.len() < 2 {
                continue;
            }
            let step = median(&d);
            for delta in &d {
                assert!(
                    (delta - step).abs() <= 2,
                    "seed {seed}: hymnal delta {delta} strays from grid step {step}"
                );
            }
        }
    }
}

#[test]
fn syncopated_vocals_stay_ordered_and_deterministic() {
    let chords = chords();
    for seed in 0..20u64 {
        let mut p = VocalParams::default();
        p.draft = generate_lyrics(&p, seed.wrapping_add(7));
        let a = derive_vocal(&chords, &p, TPB, seed);
        let b = derive_vocal(&chords, &p, TPB, seed);
        assert_eq!(a, b, "seed {seed}: nondeterministic vocal output");
        // Within each line, onsets stay strictly increasing — the
        // anticipation is bounded by half a slot, so it can never
        // reorder syllables.
        for line in line_slices(&a, &p) {
            for w in line.windows(2) {
                assert!(
                    w[0].start_tick < w[1].start_tick,
                    "seed {seed}: syllable order broken ({} >= {})",
                    w[0].start_tick,
                    w[1].start_tick
                );
            }
        }
    }
}
