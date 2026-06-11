//! SATB voice-leading pass (`satb_voicings`) — the §2G rules: bass
//! first, melody backwards from the cadence with correct tendency
//! tones, inner voices to nearest chord tones, no parallel perfect
//! fifths/octaves, no doubled leading tone or chordal seventh, and
//! contrary motion against a rising 4→5 bass.

use resonance_music_theory::{
    chordal_seventh, satb_voicings, Chord, ChordQuality, Mode, PitchClass, PitchClass::*, Scale,
};

const REGISTER: (u8, u8) = (52, 76); // the pad default, two octaves

fn c_major() -> Option<Scale> {
    Some(Scale::new(C, Mode::Major))
}

fn maj(root: PitchClass) -> Chord {
    Chord::new(root, ChordQuality::Maj)
}

fn min(root: PitchClass) -> Chord {
    Chord::new(root, ChordQuality::Min)
}

/// Count how many voices of `voicing` sound the pitch class `pc`.
fn count_pc(voicing: &[u8], pc: PitchClass) -> usize {
    voicing
        .iter()
        .filter(|&&n| n % 12 == pc.to_semitone())
        .count()
}

/// Assert no consecutive perfect fifths/octaves between any voice pair
/// of two adjacent voicings (voices identified by index).
fn assert_no_parallels(prev: &[u8], next: &[u8], at: usize) {
    for a in 0..prev.len() {
        for b in (a + 1)..prev.len() {
            let moved = prev[a] != next[a] || prev[b] != next[b];
            let both_moved = prev[a] != next[a] && prev[b] != next[b];
            if !moved || !both_moved {
                continue;
            }
            let prev_ic = (prev[a] as i32 - prev[b] as i32).unsigned_abs() % 12;
            let next_ic = (next[a] as i32 - next[b] as i32).unsigned_abs() % 12;
            assert!(
                !(prev_ic == next_ic && (prev_ic == 0 || prev_ic == 7)),
                "parallel {} between voices {a}/{b} at chord {at}: {prev:?} -> {next:?}",
                if prev_ic == 0 { "octaves" } else { "fifths" },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Shape
// ---------------------------------------------------------------------------

#[test]
fn empty_progression_yields_no_voicings() {
    assert!(satb_voicings(&[], c_major(), REGISTER).is_empty());
}

#[test]
fn wide_register_renders_four_voices_per_chord() {
    let out = satb_voicings(&[maj(C), maj(F), maj(G), maj(C)], c_major(), REGISTER);
    assert_eq!(out.len(), 4);
    for v in &out {
        assert_eq!(v.len(), 4, "expected SATB (4 voices), got {v:?}");
    }
}

#[test]
fn narrow_register_drops_to_three_voices() {
    let out = satb_voicings(&[maj(C), maj(F)], c_major(), (60, 71));
    for v in &out {
        assert_eq!(v.len(), 3, "narrow register should voice 3 parts, got {v:?}");
    }
}

#[test]
fn voices_stay_in_register_and_are_distinct() {
    let chords = [maj(C), maj(F), min(A), maj(G), maj(C)];
    for register in [(52u8, 76u8), (48, 72), (60, 71), (40, 80)] {
        let out = satb_voicings(&chords, c_major(), register);
        for (i, v) in out.iter().enumerate() {
            for &n in v {
                assert!(
                    n >= register.0 && n <= register.1,
                    "note {n} outside register {register:?} at chord {i}"
                );
            }
            let mut sorted = v.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), v.len(), "duplicate notes in voicing {v:?}");
        }
    }
}

#[test]
fn pass_is_deterministic() {
    let chords = [maj(C), min(A), maj(F), maj(G), maj(C)];
    let a = satb_voicings(&chords, c_major(), REGISTER);
    let b = satb_voicings(&chords, c_major(), REGISTER);
    assert_eq!(a, b);
}

// ---------------------------------------------------------------------------
// Bass first
// ---------------------------------------------------------------------------

#[test]
fn bass_voice_is_the_chord_root() {
    let chords = [maj(C), maj(F), maj(G), min(A)];
    let out = satb_voicings(&chords, c_major(), REGISTER);
    for (chord, v) in chords.iter().zip(out.iter()) {
        assert_eq!(
            v[0] % 12,
            chord.root.to_semitone(),
            "bass of {chord} should be its root, got voicing {v:?}"
        );
        let lowest = *v.iter().min().unwrap();
        assert_eq!(v[0], lowest, "bass should be the lowest voice in {v:?}");
    }
}

#[test]
fn slash_chord_puts_the_named_bass_in_the_bass() {
    let chords = [maj(C), maj(G).with_bass(B)];
    let out = satb_voicings(&chords, c_major(), REGISTER);
    assert_eq!(out[1][0] % 12, B.to_semitone(), "G/B should sit on B");
}

// ---------------------------------------------------------------------------
// Chord coverage
// ---------------------------------------------------------------------------

#[test]
fn all_triad_tones_are_present() {
    let chords = [maj(C), maj(F), maj(G), min(A), maj(C)];
    let out = satb_voicings(&chords, c_major(), REGISTER);
    for (chord, v) in chords.iter().zip(out.iter()) {
        for pc in chord.pitch_classes() {
            assert!(
                count_pc(v, pc) >= 1,
                "chord {chord} is missing tone {pc} in voicing {v:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Forbidden parallels
// ---------------------------------------------------------------------------

#[test]
fn no_parallel_fifths_or_octaves_in_diatonic_progressions() {
    let progressions: Vec<Vec<Chord>> = vec![
        vec![maj(C), maj(F), maj(G), maj(C)],
        vec![maj(C), maj(G), min(A), maj(F)], // axis
        vec![maj(C), min(A), maj(F), maj(G)], // doo-wop
        vec![min(A), maj(G), maj(F), maj(E)], // lament-ish
        vec![maj(F), maj(G), min(A), maj(C)], // hopscotch
    ];
    for chords in &progressions {
        let out = satb_voicings(chords, c_major(), REGISTER);
        for i in 1..out.len() {
            assert_no_parallels(&out[i - 1], &out[i], i);
        }
    }
}

#[test]
fn root_motion_progressions_avoid_parallels_without_a_scale_too() {
    // Root motion by step (F -> G) is the classic parallel trap.
    let chords = [maj(F), maj(G), maj(F), maj(G)];
    let out = satb_voicings(&chords, None, REGISTER);
    for i in 1..out.len() {
        assert_no_parallels(&out[i - 1], &out[i], i);
    }
}

// ---------------------------------------------------------------------------
// Doubling rules
// ---------------------------------------------------------------------------

#[test]
fn leading_tone_is_never_doubled() {
    // B is the leading tone of C major and the third of the G chord:
    // every G voicing must contain exactly one B.
    let chords = [maj(C), maj(G), maj(C), maj(G), min(A)];
    let out = satb_voicings(&chords, c_major(), REGISTER);
    for (chord, v) in chords.iter().zip(out.iter()) {
        if chord.root == G {
            assert_eq!(count_pc(v, B), 1, "leading tone doubled in {v:?}");
        }
    }
}

#[test]
fn chordal_seventh_is_never_doubled() {
    let g7 = Chord::new(G, ChordQuality::Dom7);
    let chords = [maj(C), g7, maj(C), g7, maj(C)];
    let out = satb_voicings(&chords, c_major(), REGISTER);
    let sev = chordal_seventh(g7).unwrap();
    assert_eq!(sev, F);
    for (chord, v) in chords.iter().zip(out.iter()) {
        if chord.quality == ChordQuality::Dom7 {
            assert_eq!(count_pc(v, sev), 1, "chordal 7th doubled in {v:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Tendency tones
// ---------------------------------------------------------------------------

#[test]
fn dominant_seventh_resolves_down_by_step() {
    let g7 = Chord::new(G, ChordQuality::Dom7);
    let chords = [maj(C), maj(F), g7, maj(C)];
    let out = satb_voicings(&chords, c_major(), REGISTER);
    // Find the voice holding F (the 7th of G7) and check it steps down
    // into the C chord (F -> E).
    let v_idx = (0..4)
        .find(|&i| out[2][i] % 12 == F.to_semitone())
        .expect("G7 voicing must contain its seventh");
    let from = out[2][v_idx];
    let to = out[3][v_idx];
    assert!(
        to < from && from - to <= 2,
        "7th must resolve down by step: {from} -> {to}"
    );
}

#[test]
fn soprano_leading_tone_resolves_up_to_the_tonic() {
    // Force the leading tone into the soprano by ending V -> I from a
    // soprano plan that walks down to B, then check it rises to C.
    let chords = [maj(C), maj(F), maj(G), maj(C)];
    let out = satb_voicings(&chords, c_major(), REGISTER);
    let soprano_g = out[2][3];
    let soprano_c = out[3][3];
    if soprano_g % 12 == B.to_semitone() {
        assert_eq!(
            soprano_c,
            soprano_g + 1,
            "soprano leading tone must rise to the tonic"
        );
    }
    // Whatever the path, the cadence should land the soprano on a
    // C-chord tone with the tonic preferred by the backward plan.
    assert_eq!(soprano_c % 12, C.to_semitone());
}

#[test]
fn final_soprano_prefers_the_tonic() {
    let progressions: Vec<Vec<Chord>> = vec![
        vec![maj(C), maj(F), maj(G), maj(C)],
        vec![min(A), maj(F), maj(G), maj(C)],
        vec![maj(F), maj(G), maj(C)],
    ];
    for chords in &progressions {
        let out = satb_voicings(chords, c_major(), REGISTER);
        let soprano = *out.last().unwrap().last().unwrap();
        assert_eq!(
            soprano % 12,
            C.to_semitone(),
            "cadence soprano should be the tonic"
        );
    }
}

// ---------------------------------------------------------------------------
// Contrary motion against a rising 4→5 bass
// ---------------------------------------------------------------------------

#[test]
fn upper_voices_prefer_contrary_motion_on_rising_4_to_5_bass() {
    let chords = [maj(C), maj(F), maj(G), maj(C)];
    let out = satb_voicings(&chords, c_major(), REGISTER);
    let (f, g) = (&out[1], &out[2]);
    // Only meaningful when the bass actually rises F -> G.
    if g[0] > f[0] {
        let rising = (1..4).filter(|&i| g[i] > f[i]).count();
        assert!(
            rising <= 1,
            "most upper voices should move contrary to a rising 4->5 bass: {f:?} -> {g:?}"
        );
    }
}
