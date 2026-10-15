//! BUG-71: with `stay_in_scale` on, the vocal melody keeps to the
//! section's key — not to a scale guessed from the first chord. The live
//! repro was a D-minor chorus on Bbmaj7 – F/A – Gm7 – A7: the generator
//! took Bb major from the Bbmaj7 and sang Eb on 9 of 29 notes.
//!
//! "In scale" here means: a tone of the section's scale, or a tone of the
//! chord sounding under the note. The second clause admits A7's C# — the
//! raised leading tone a minor key's dominant chord carries — when the
//! generator anchors a note on that chord; anything else (Eb) is out.

use resonance_music_theory::{
    derive_vocal_with_holds, generate_lyrics, Chord, ChordQuality, GeneratedNote, Mode, PitchClass,
    Scale, SyllableMode, TimedChord, VocalParams, VocalStyle,
};

const TPB: u32 = 480;

/// Bbmaj7 | F/A | Gm7 | A7, a bar each, twice: an 8-bar chorus.
fn chorus_chords() -> Vec<TimedChord> {
    let bar = [
        Chord::new(PitchClass::As, ChordQuality::Maj7),
        Chord::new(PitchClass::F, ChordQuality::Maj).with_bass(PitchClass::A),
        Chord::new(PitchClass::G, ChordQuality::Min7),
        Chord::new(PitchClass::A, ChordQuality::Dom7),
    ];
    (0..8)
        .map(|i| TimedChord {
            chord: bar[i % 4],
            start_beat: i as u32 * 4,
            duration_beats: 4,
        })
        .collect()
}

fn d_minor() -> Scale {
    Scale::new(PitchClass::D, Mode::Minor)
}

fn chord_under<'a>(chords: &'a [TimedChord], n: &GeneratedNote) -> &'a TimedChord {
    let beat = (n.start_tick / u64::from(TPB)) as u32;
    chords
        .iter()
        .find(|c| beat >= c.start_beat && beat < c.start_beat + c.duration_beats)
        .expect("every note starts under a chord")
}

fn params(style: VocalStyle, mode: SyllableMode, seed: u64) -> VocalParams {
    let mut p = VocalParams {
        style,
        syllable_mode: mode,
        stay_in_scale: true,
        ..VocalParams::default()
    };
    p.draft = generate_lyrics(&p, seed);
    p
}

/// Every note is a D-minor tone or a tone of the chord under it.
fn assert_in_key(notes: &[GeneratedNote], chords: &[TimedChord], label: &str) {
    assert!(!notes.is_empty(), "{label}: no notes");
    let scale = d_minor();
    for n in notes {
        let chord = chord_under(chords, n);
        let chord_tone = chord.chord.pitch_classes().any(|p| p.to_semitone() == n.note % 12);
        assert!(
            scale.contains(n.note) || chord_tone,
            "{label}: note {} at tick {} is neither in D minor nor a tone of {}",
            n.note,
            n.start_tick,
            chord.chord,
        );
        // Eb is in neither, whatever the chord.
        assert_ne!(n.note % 12, 3, "{label}: Eb/D# at tick {}", n.start_tick);
    }
}

/// The live repro: pop ballad, mixed syllables, seed 7, the section's D
/// minor passed in.
#[test]
fn d_minor_chorus_on_bbmaj7_sings_no_eb() {
    let chords = chorus_chords();
    let p = params(VocalStyle::PopBallad, SyllableMode::Mixed, 7);
    let derived = derive_vocal_with_holds(&chords, &p, TPB, 4, None, Some(d_minor()), 7);
    assert_in_key(&derived.notes, &chords, "pop_ballad/mixed/7");
}

/// Every style that snaps to the scale, every syllable mode, a spread of
/// seeds — with the section's key, and with no key (the generator then
/// infers one from all the chords, not the first).
#[test]
fn every_snapping_style_keeps_to_the_section_key() {
    let chords = chorus_chords();
    for style in [VocalStyle::PopBallad, VocalStyle::Anthemic, VocalStyle::Conversational] {
        for mode in [SyllableMode::Syllabic, SyllableMode::Mixed, SyllableMode::Melismatic] {
            for seed in [1u64, 7, 42, 99, 0xABCD] {
                let p = params(style, mode, seed);
                for scale in [Some(d_minor()), None] {
                    let derived = derive_vocal_with_holds(&chords, &p, TPB, 4, None, scale, seed);
                    assert_in_key(
                        &derived.notes,
                        &chords,
                        &format!("{style:?}/{mode:?}/{seed}/{scale:?}"),
                    );
                }
            }
        }
    }
}
