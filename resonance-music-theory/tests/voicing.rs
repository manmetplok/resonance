use resonance_music_theory::chord::{Chord, ChordQuality};
use resonance_music_theory::pitch::PitchClass::*;
use resonance_music_theory::voicing::*;

#[test]
fn nearest_above_wraps_to_next_octave() {
    assert_eq!(nearest_midi_above(C, 60), 60);
    assert_eq!(nearest_midi_above(E, 60), 64);
    assert_eq!(nearest_midi_above(G, 60), 67);
    // C# at floor C5=61 -> next C# is 73.
    assert_eq!(nearest_midi_above(Cs, 61), 61);
    assert_eq!(nearest_midi_above(C, 61), 72);
}

#[test]
fn nearest_to_picks_shorter_direction() {
    // From C4 (60), F is 5 up (65), not 7 down (53).
    assert_eq!(nearest_midi_to(F, 60), 65);
    // From C4 (60), A is 3 down (57), not 9 up.
    assert_eq!(nearest_midi_to(A, 60), 57);
    // From C4 (60), F# is 6 either way — current impl prefers upper.
    assert_eq!(nearest_midi_to(Fs, 60), 66);
}

#[test]
fn close_voicing_c_major_at_60() {
    let c = Chord::new(C, ChordQuality::Maj);
    assert_eq!(close_voicing(c, 60), vec![60, 64, 67]);
}

#[test]
fn close_voicing_f_major_at_60_is_first_inversion() {
    // Floor C4 (60). Each pitch class gets its nearest instance >= floor,
    // so C stays at 60 while F and A rise to 65 and 69 — a first-inversion
    // voicing of F major. This keeps every voice inside the octave above
    // the floor, which is the point of "close voicing".
    let f = Chord::new(F, ChordQuality::Maj);
    assert_eq!(close_voicing(f, 60), vec![60, 65, 69]);
}

#[test]
fn voice_lead_c_to_f_stays_close() {
    // C major (60,64,67) → F major (pcs F,A,C).
    // Good voice leading: C stays as C (60 → 65? no, C is shared; hold
    // it as 60), E → F (64 → 65), G → A (67 → 69). Total = 5+1+2 = 8.
    // Any assignment that drops the root and doubles an inner voice
    // should produce a worse (larger) cost.
    let prev = [60u8, 64, 67];
    let next = [F, A, C];
    let out = voice_lead(&prev, &next, (52, 76));
    assert_eq!(out.len(), 3);
    // Total movement bound: no voice moves more than 2 semitones.
    let cost: i32 = prev
        .iter()
        .zip(out.iter())
        .map(|(&p, &n)| {
            // prev is sorted; out is sorted; compare in order for sanity.
            (p as i32 - n as i32).abs()
        })
        .sum();
    assert!(
        cost <= 4,
        "voice leading too jumpy: cost={cost}, out={out:?}"
    );
    // All three pitch classes must be present.
    let pcs: std::collections::HashSet<u8> = out.iter().map(|n| n % 12).collect();
    assert!(pcs.contains(&(F.to_semitone())));
    assert!(pcs.contains(&(A.to_semitone())));
    assert!(pcs.contains(&(C.to_semitone())));
}

#[test]
fn voice_lead_g_to_c_resolves_leading_tone() {
    // G major (67, 71, 74) → C major (C,E,G).
    // Classic V-I: B (71) leads to C (72), G holds, D goes to E (74→76).
    let prev = [67u8, 71, 74];
    let out = voice_lead(&prev, &[C, E, G], (55, 80));
    assert_eq!(out.len(), 3);
    let cost: i32 = prev
        .iter()
        .zip(out.iter())
        .map(|(&p, &n)| (p as i32 - n as i32).abs())
        .sum();
    assert!(cost <= 4);
}

#[test]
fn voice_lead_preserves_voice_count() {
    // Triad → 4-voice should double one pitch class.
    let prev = [48u8, 52, 55, 60];
    let out = voice_lead(&prev, &[D, F, A], (36, 72));
    assert_eq!(out.len(), 4);
    let pcs: std::collections::HashSet<u8> = out.iter().map(|n| n % 12).collect();
    assert!(pcs.contains(&D.to_semitone()));
    assert!(pcs.contains(&F.to_semitone()));
    assert!(pcs.contains(&A.to_semitone()));
}

#[test]
fn voice_lead_stays_in_register() {
    let prev = [60u8, 64, 67];
    let out = voice_lead(&prev, &[C, E, G], (52, 76));
    for n in out {
        assert!((52..=76).contains(&n), "{n} out of register");
    }
}

#[test]
fn voice_lead_empty_inputs_are_empty() {
    assert!(voice_lead(&[], &[C], (48, 72)).is_empty());
    assert!(voice_lead(&[60], &[], (48, 72)).is_empty());
}
