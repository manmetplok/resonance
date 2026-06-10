//! Single-climax rule for vocal lines (Open Music Theory v2,
//! well-formed melodic lines): each lyric line — the vocal phrase
//! unit — carries exactly one highest note, placed in the line's
//! second half and never on the final (cadence) syllable. The wave
//! contour previously produced two equal crests per line.
//!
//! Also re-asserts the SVS adjacency cap (`MAX_INTERVAL` = 9
//! semitones between consecutive syllables): the climax pass demotes
//! pitches after the styles' interval capping ran, so it must not
//! widen any adjacent interval past what the synthesis model renders
//! cleanly.

use resonance_music_theory::{
    count_syllables, derive_vocal, Chord, ChordQuality, GeneratedNote, PitchClass, TimedChord,
    VocalContour, VocalParams, VocalStyle,
};

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

/// One note per syllable, in lyric order — recover the per-line note
/// slices the same way the generator and the SVS pipeline do.
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

/// Assert the single-climax rule on one lyric line. Mirrors the
/// enforcement pass's skip conditions: lines under 3 syllables can't
/// host a non-final second-half climax, flat (chant-like) lines are
/// exempt, and so are lines whose entire climax window sits on the
/// register floor (nothing can be demoted below the floor).
fn assert_line_climax(notes: &[GeneratedNote], range_lo: u8, ctx: &str) {
    let pitches: Vec<u8> = notes.iter().map(|n| n.note).collect();
    let n = pitches.len();
    if n < 3 {
        return;
    }
    let max = *pitches.iter().max().unwrap();
    let min = *pitches.iter().min().unwrap();
    if max == min {
        return;
    }
    let window_max = *pitches[n / 2..n - 1].iter().max().unwrap();
    if window_max <= range_lo {
        return;
    }
    let peaks: Vec<usize> = pitches
        .iter()
        .enumerate()
        .filter(|(_, &p)| p == max)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        peaks.len(),
        1,
        "expected exactly one climax, found peaks at {peaks:?} in {pitches:?} for {ctx}"
    );
    let climax = peaks[0];
    assert!(
        climax >= n / 2,
        "climax at {climax} sits in the first half of {n} syllables in {pitches:?} for {ctx}"
    );
    assert_ne!(
        climax,
        n - 1,
        "climax must not be the final (cadence) syllable in {pitches:?} for {ctx}"
    );
}

/// Adjacent syllables must stay within the SVS render cap.
fn assert_adjacency_cap(notes: &[GeneratedNote], ctx: &str) {
    for w in notes.windows(2) {
        let iv = (w[1].note as i16 - w[0].note as i16).abs();
        assert!(
            iv <= 9,
            "adjacent interval of {iv} semitones exceeds the SVS cap for {ctx}"
        );
    }
}

const STYLES: [VocalStyle; 6] = [
    VocalStyle::PopBallad,
    VocalStyle::Conversational,
    VocalStyle::Hymnal,
    VocalStyle::Folk,
    VocalStyle::Anthemic,
    VocalStyle::Chant,
];

const CONTOURS: [VocalContour; 5] = [
    VocalContour::Arch,
    VocalContour::Rise,
    VocalContour::Fall,
    VocalContour::Wave,
    VocalContour::Flat,
];

#[test]
fn vocal_lines_have_a_single_second_half_climax() {
    for style in STYLES {
        if style == VocalStyle::Chant {
            // Chant is exempt: recitation on a speaking tone has no
            // melodic climax to discipline.
            continue;
        }
        for contour in CONTOURS {
            for seed in 0..40u64 {
                let mut p = VocalParams::default();
                p.style = style;
                p.contour = contour;
                p.draft = resonance_music_theory::generate_lyrics(&p, seed.wrapping_add(99));
                let notes = derive_vocal(&chords(), &p, 480, seed);
                assert!(!notes.is_empty(), "empty vocal for seed {seed}");
                for (li, slice) in line_slices(&notes, &p).iter().enumerate() {
                    assert_line_climax(
                        slice,
                        p.range.0,
                        &format!("style {style:?}, contour {contour:?}, seed {seed}, line {li}"),
                    );
                }
            }
        }
    }
}

#[test]
fn vocal_climax_pass_preserves_the_svs_interval_cap() {
    for style in STYLES {
        for seed in 0..60u64 {
            let mut p = VocalParams::default();
            p.style = style;
            p.contour = VocalContour::Wave;
            p.draft = resonance_music_theory::generate_lyrics(&p, seed.wrapping_add(7));
            let notes = derive_vocal(&chords(), &p, 480, seed);
            for (li, slice) in line_slices(&notes, &p).iter().enumerate() {
                assert_adjacency_cap(slice, &format!("style {style:?}, seed {seed}, line {li}"));
            }
        }
    }
}

#[test]
fn vocal_climax_holds_without_scale_snapping() {
    // `stay_in_scale = false` drops the scale: demotion then steps by
    // semitones instead of scale degrees.
    for seed in 0..60u64 {
        let mut p = VocalParams::default();
        p.stay_in_scale = false;
        p.contour = VocalContour::Wave;
        p.draft = resonance_music_theory::generate_lyrics(&p, seed);
        let notes = derive_vocal(&chords(), &p, 480, seed);
        for (li, slice) in line_slices(&notes, &p).iter().enumerate() {
            assert_line_climax(slice, p.range.0, &format!("no scale, seed {seed}, line {li}"));
            assert_adjacency_cap(slice, &format!("no scale, seed {seed}, line {li}"));
        }
    }
}

#[test]
fn vocal_climax_enforcement_stays_deterministic() {
    let mut p = VocalParams::default();
    p.contour = VocalContour::Wave;
    p.draft = resonance_music_theory::generate_lyrics(&p, 42);
    let a = derive_vocal(&chords(), &p, 480, 42);
    let b = derive_vocal(&chords(), &p, 480, 42);
    assert_eq!(a, b);
}
