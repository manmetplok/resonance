//! `refit_melody`: a melody re-fitted to new chords keeps its rhythm, note
//! count and velocities, and every pitch lands on a chord tone (strong
//! beats, long notes) or a scale tone (elsewhere), moving as little as it
//! can (vocal-melody-on-chord-change, BUG-70).

use resonance_music_theory::{
    derive_vocal, generate_lyrics, refit_melody, Chord, ChordQuality, GeneratedNote, Mode,
    PitchClass, Scale, TimedChord, VocalParams,
};

const TPB: u32 = 480;

fn chord(root: PitchClass, quality: ChordQuality, start_beat: u32, duration_beats: u32) -> TimedChord {
    TimedChord {
        chord: Chord::new(root, quality),
        start_beat,
        duration_beats,
    }
}

fn note(pitch: u8, start_beat: f64, beats: f64) -> GeneratedNote {
    GeneratedNote {
        note: pitch,
        velocity: 0.7,
        start_tick: (start_beat * f64::from(TPB)) as u64,
        duration_ticks: (beats * f64::from(TPB)) as u64,
    }
}

fn c_major() -> Scale {
    Scale::new(PitchClass::C, Mode::Major)
}

fn pcs(c: &TimedChord) -> Vec<u8> {
    c.chord.pitch_classes().map(|p| p.to_semitone()).collect()
}

/// The chord sounding at a note's start.
fn chord_under<'a>(chords: &'a [TimedChord], n: &GeneratedNote) -> Option<&'a TimedChord> {
    let beat = (n.start_tick / u64::from(TPB)) as u32;
    chords
        .iter()
        .find(|c| beat >= c.start_beat && beat < c.start_beat + c.duration_beats)
}

fn is_strong(n: &GeneratedNote, chords: &[TimedChord]) -> bool {
    let tpb = u64::from(TPB);
    if n.duration_ticks >= tpb {
        return true;
    }
    if n.start_tick % tpb != 0 {
        return false;
    }
    let beat = n.start_tick / tpb;
    beat % 4 == 0 || beat % 4 == 2 || chords.iter().any(|c| u64::from(c.start_beat) == beat)
}

/// Every invariant the intent promises, checked note by note.
fn assert_refit_invariants(
    before: &[GeneratedNote],
    after: &[GeneratedNote],
    chords: &[TimedChord],
    scale: Scale,
) {
    assert_eq!(before.len(), after.len(), "note count unchanged");
    for (b, a) in before.iter().zip(after) {
        assert_eq!(b.start_tick, a.start_tick, "start unchanged");
        assert_eq!(b.duration_ticks, a.duration_ticks, "duration unchanged");
        assert_eq!(b.velocity, a.velocity, "velocity unchanged");
        assert!(
            (i16::from(a.note) - i16::from(b.note)).abs() <= 6,
            "a re-fit moves a pitch at most a tritone: {} -> {}",
            b.note,
            a.note
        );
        match chord_under(chords, a) {
            Some(c) if is_strong(a, chords) => assert!(
                pcs(c).contains(&(a.note % 12)),
                "strong/long note {} at tick {} is a chord tone of {}",
                a.note,
                a.start_tick,
                c.chord
            ),
            _ => assert!(
                scale.contains(a.note),
                "weak note {} at tick {} is a scale tone",
                a.note,
                a.start_tick
            ),
        }
    }
}

#[test]
fn rhythm_count_and_velocity_survive_and_pitches_fit() {
    // A line written over C | Am, re-fitted to F | G.
    let line = vec![
        note(64, 0.0, 1.0),  // E, downbeat, long
        note(65, 1.0, 0.5),  // F, weak
        note(67, 1.5, 0.5),  // G, off-beat
        note(69, 2.0, 1.0),  // A, mid-bar, long
        note(72, 4.0, 2.0),  // C, chord change
        note(71, 6.0, 0.5),  // B, weak
        note(69, 6.5, 0.5),  // A, off-beat
        note(67, 7.0, 1.0),  // G, long
    ];
    let chords = vec![
        chord(PitchClass::F, ChordQuality::Maj, 0, 4),
        chord(PitchClass::G, ChordQuality::Maj, 4, 4),
    ];
    let refit = refit_melody(&line, &chords, Some(c_major()), TPB, 4);
    assert_refit_invariants(&line, &refit, &chords, c_major());
}

#[test]
fn notes_that_already_fit_do_not_move() {
    let line = vec![note(65, 0.0, 1.0), note(69, 1.0, 1.0), note(72, 2.0, 2.0)];
    let chords = vec![chord(PitchClass::F, ChordQuality::Maj, 0, 4)];
    let refit = refit_melody(&line, &chords, Some(c_major()), TPB, 4);
    assert_eq!(refit, line);
}

/// A weak note with no scale to fit (none, or chromatic) keeps its pitch:
/// a passing tone has nothing to be wrong against.
#[test]
fn a_weak_note_without_a_scale_keeps_its_pitch() {
    let line = vec![note(66, 1.5, 0.5)];
    let chords = vec![chord(PitchClass::C, ChordQuality::Maj, 0, 4)];
    assert_eq!(refit_melody(&line, &chords, None, TPB, 4), line);
    let chromatic = Scale::new(PitchClass::C, Mode::Chromatic);
    assert_eq!(refit_melody(&line, &chords, Some(chromatic), TPB, 4), line);
}

/// D on a C-major downbeat is a tone from both C and E: the contour
/// decides. Rising into it from C keeps rising (E, not a repeated C);
/// falling into it from E keeps falling (C).
#[test]
fn an_equal_choice_keeps_the_previous_interval_direction() {
    let chords = vec![chord(PitchClass::C, ChordQuality::Maj, 0, 8)];
    let rising = vec![note(60, 3.0, 1.0), note(62, 4.0, 1.0)];
    let refit = refit_melody(&rising, &chords, Some(c_major()), TPB, 4);
    assert_eq!(refit[1].note, 64, "C -> D rises, so D becomes E");

    let falling = vec![note(64, 3.0, 1.0), note(62, 4.0, 1.0)];
    let refit = refit_melody(&falling, &chords, Some(c_major()), TPB, 4);
    assert_eq!(refit[1].note, 60, "E -> D falls, so D becomes C");
}

/// A repeated note stays a repeat when the tie allows it.
#[test]
fn a_repeated_note_repeats_after_a_refit() {
    let chords = vec![chord(PitchClass::C, ChordQuality::Maj, 0, 8)];
    let line = vec![note(62, 0.0, 1.0), note(62, 4.0, 1.0)];
    let refit = refit_melody(&line, &chords, Some(c_major()), TPB, 4);
    assert_eq!(refit[0].note, refit[1].note, "{refit:?}");
}

/// The property over real generated material: a melody derived over one
/// progression and re-fitted to another keeps every invariant.
#[test]
fn a_generated_melody_refits_to_another_progression() {
    let before_chords = vec![
        chord(PitchClass::A, ChordQuality::Min, 0, 4),
        chord(PitchClass::D, ChordQuality::Min, 4, 4),
        chord(PitchClass::E, ChordQuality::Min, 8, 4),
        chord(PitchClass::A, ChordQuality::Min, 12, 4),
    ];
    let after_chords = vec![
        chord(PitchClass::F, ChordQuality::Maj, 0, 4),
        chord(PitchClass::C, ChordQuality::Maj, 4, 4),
        chord(PitchClass::G, ChordQuality::Maj, 8, 4),
        chord(PitchClass::A, ChordQuality::Min, 12, 4),
    ];
    let a_minor = Scale::new(PitchClass::A, Mode::Minor);
    for seed in 1..20u64 {
        let mut params = VocalParams::default();
        params.draft = generate_lyrics(&params, seed);
        let melody = derive_vocal(&before_chords, &params, TPB, seed);
        assert!(!melody.is_empty());
        let refit = refit_melody(&melody, &after_chords, Some(a_minor), TPB, 4);
        assert_refit_invariants(&melody, &refit, &after_chords, a_minor);
    }
}
