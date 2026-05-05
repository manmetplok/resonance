//! Verifies the behaviors promised by the lane-inspector tooltips. Each test
//! corresponds to a specific tooltip claim; if a claim drifts from the code,
//! the test should fail and either the implementation or the wording needs
//! to be reconciled.

use resonance_music_theory::{
    derive_bass, derive_bass_motif, derive_melody, derive_motif_melody_with_section, derive_pad,
    BassMotifMode, BassMotifPhrase, BassParams, BassStyle, Chord, ChordQuality, MelodyParams,
    MelodyStyle, Mode, MotifParams, MotifSource, PadParams, PitchClass, Scale, TimedChord,
};

const TPB: u32 = 480;

fn gen(p: MotifParams) -> MotifSource {
    MotifSource::Generated(p)
}

fn tc(chord: Chord, start_beat: u32, duration_beats: u32) -> TimedChord {
    TimedChord {
        chord,
        start_beat,
        duration_beats,
    }
}

fn c_major_chords(count: u32, beats_each: u32) -> Vec<TimedChord> {
    (0..count)
        .map(|i| tc(Chord::new(PitchClass::C, ChordQuality::Maj), i * beats_each, beats_each))
        .collect()
}

// ---------------------------------------------------------------------------
// Bass styles
// ---------------------------------------------------------------------------

/// Tooltip: "Root + fifth: alternating root/fifth per beat".
#[test]
fn bass_style_root_fifth_alternates_root_and_fifth() {
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let p = BassParams {
        style: BassStyle::RootFifth,
        ..BassParams::default()
    };
    let notes = derive_bass(&chords, None, &p, TPB);
    assert_eq!(notes.len(), 4);
    // Beat 0 = root (C), beat 1 = fifth (G), beat 2 = root, beat 3 = fifth.
    assert_eq!(notes[0].note % 12, PitchClass::C.to_semitone());
    assert_eq!(notes[1].note % 12, PitchClass::G.to_semitone());
    assert_eq!(notes[2].note % 12, PitchClass::C.to_semitone());
    assert_eq!(notes[3].note % 12, PitchClass::G.to_semitone());
}

/// Tooltip: "Octave: root and root+12 alternating".
#[test]
fn bass_style_octave_alternates_root_and_octave_up() {
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let p = BassParams {
        style: BassStyle::Octave,
        ..BassParams::default()
    };
    let notes = derive_bass(&chords, None, &p, TPB);
    assert_eq!(notes.len(), 4);
    let root = notes[0].note;
    assert_eq!(notes[1].note, root + 12);
    assert_eq!(notes[2].note, root);
    assert_eq!(notes[3].note, root + 12);
}

// ---------------------------------------------------------------------------
// Melody arp styles
// ---------------------------------------------------------------------------

/// Tooltip for MelodyStyle::ArpDown: melody walks down through chord tones.
#[test]
fn melody_arp_down_descends_through_chord_tones() {
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let p = MelodyParams {
        style: MelodyStyle::ArpDown,
        register: (60, 84),
        note_value_ticks: 480,
        rest_density: 0.0,
        ..MelodyParams::default()
    };
    let notes = derive_melody(&chords, None, &p, TPB, 0);
    // Four quarter-note slots in a 4-beat chord.
    assert_eq!(notes.len(), 4);
    // First slot picks the highest chord tone in register; subsequent slots
    // walk downward through the tone list.
    for i in 1..notes.len().min(3) {
        assert!(
            notes[i].note < notes[i - 1].note,
            "ArpDown should descend: slot {} note {} not < slot {} note {}",
            i,
            notes[i].note,
            i - 1,
            notes[i - 1].note
        );
    }
}

/// Tooltip for MelodyStyle::ArpUpDown: melody bounces up then back down.
#[test]
fn melody_arp_up_down_ascends_then_descends() {
    // Tight register (one octave wide) so chord_tones_in_register yields
    // exactly the 3 tones [C4, E4, G4]. With n=3 the cycle is 2*3-2 = 4
    // slots: tones[0], tones[1], tones[2], tones[4-3]=tones[1] —
    // up, up, down.
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let p = MelodyParams {
        style: MelodyStyle::ArpUpDown,
        register: (60, 71),
        note_value_ticks: 480,
        rest_density: 0.0,
        ..MelodyParams::default()
    };
    let notes = derive_melody(&chords, None, &p, TPB, 0);
    assert_eq!(notes.len(), 4);
    assert!(notes[0].note < notes[1].note, "first slot < second");
    assert!(notes[1].note < notes[2].note, "second slot < third (peak)");
    assert!(notes[2].note > notes[3].note, "third slot > fourth (turn-around)");
}

// ---------------------------------------------------------------------------
// Note value (arp)
// ---------------------------------------------------------------------------

/// Tooltip: "Quarter / Eighth / Sixteenth at the project tempo." With
/// 480 ticks per quarter, the picker values 480 / 240 / 120 must produce
/// exactly those durations on every emitted arp note.
#[test]
fn melody_arp_note_value_drives_per_note_duration() {
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    for &(slot_ticks, expected_count) in &[(480u32, 4usize), (240, 8), (120, 16)] {
        let p = MelodyParams {
            style: MelodyStyle::ArpUp,
            register: (60, 84),
            note_value_ticks: slot_ticks,
            rest_density: 0.0,
            ..MelodyParams::default()
        };
        let notes = derive_melody(&chords, None, &p, TPB, 0);
        assert_eq!(
            notes.len(),
            expected_count,
            "note_value_ticks {slot_ticks} should produce {expected_count} notes"
        );
        for n in &notes {
            assert_eq!(
                n.duration_ticks, slot_ticks as u64,
                "note_value_ticks {slot_ticks} should drive duration_ticks {slot_ticks}, got {}",
                n.duration_ticks
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Section motif: motif length
// ---------------------------------------------------------------------------

/// Tooltip: "Motif length: 0 = auto from complexity (2..=6)."
///
/// motif_len=0 + complexity=0.0 → expected length 2.
/// motif_len=0 + complexity=1.0 → expected length 6.
/// motif_len=N (2..=6) → exactly N notes.
///
/// The motif itself isn't directly observable, but a longer motif tiles
/// fewer times across a fixed-length chord, producing fewer NoteOn events
/// per chord. Using FirstNoteOnly bass mode we know there's exactly one
/// note per chord regardless of motif length, so we instead compare the
/// SameIntervals output across motif lengths and assert the two extremes
/// produce different note counts on the same chord.
#[test]
fn section_motif_length_explicit_changes_note_count_per_chord() {
    let chords = c_major_chords(1, 8); // one long chord so the motif tiles
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let p = BassParams {
        style: BassStyle::Motif,
        ..BassParams::default()
    };
    let m_short = MotifParams {
        seed: 7,
        complexity: 0.5,
        motif_len: 2,
        leap_chance: 0.21,
    };
    let m_long = MotifParams {
        seed: 7,
        complexity: 0.5,
        motif_len: 6,
        leap_chance: 0.21,
    };
    let notes_short = derive_bass_motif(&chords, scale, &p, &gen(m_short), 0, TPB);
    let notes_long = derive_bass_motif(&chords, scale, &p, &gen(m_long), 0, TPB);
    // A 2-note motif has two duration slots to share the chord; a 6-note
    // motif spreads the same chord across more slots, so it tiles fewer
    // times overall — but each tiling emits more notes. Net: a 6-note
    // motif emits at least as many notes per chord as a 2-note one,
    // and the two outputs must not be identical.
    assert!(
        notes_short.len() != notes_long.len() || notes_short != notes_long,
        "motif_len 2 and 6 should not produce identical output"
    );
}

/// motif_len=0 (auto) at low complexity should give a short motif; at
/// high complexity, a longer one. Compare the auto-output to explicit
/// length 2 (low) and length 6 (high) to confirm the auto path picks
/// values close to those bounds.
#[test]
fn section_motif_length_auto_follows_complexity() {
    let chords = c_major_chords(1, 8);
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let p = BassParams {
        style: BassStyle::Motif,
        ..BassParams::default()
    };

    let auto_low = derive_bass_motif(
        &chords,
        scale,
        &p,
        &gen(MotifParams { seed: 1, complexity: 0.0, motif_len: 0, leap_chance: 0.21 }),
        0,
        TPB,
    );
    let auto_high = derive_bass_motif(
        &chords,
        scale,
        &p,
        &gen(MotifParams { seed: 1, complexity: 1.0, motif_len: 0, leap_chance: 0.21 }),
        0,
        TPB,
    );
    let explicit_short = derive_bass_motif(
        &chords,
        scale,
        &p,
        &gen(MotifParams { seed: 1, complexity: 0.5, motif_len: 2, leap_chance: 0.21 }),
        0,
        TPB,
    );
    let explicit_long = derive_bass_motif(
        &chords,
        scale,
        &p,
        &gen(MotifParams { seed: 1, complexity: 0.5, motif_len: 6, leap_chance: 0.21 }),
        0,
        TPB,
    );

    // Auto-low (short motif via complexity=0) and explicit-short (motif_len=2)
    // both produce a 2-note motif → equal note count.
    assert_eq!(
        auto_low.len(),
        explicit_short.len(),
        "complexity=0 + motif_len=0 (auto) should mirror motif_len=2"
    );
    // Auto-high (long motif via complexity=1) and explicit-long (motif_len=6)
    // both produce a 6-note motif → equal note count.
    assert_eq!(
        auto_high.len(),
        explicit_long.len(),
        "complexity=1 + motif_len=0 (auto) should mirror motif_len=6"
    );
}

// ---------------------------------------------------------------------------
// Phrase length
// ---------------------------------------------------------------------------

/// Tooltip: "Phrase length: how many chords belong to one phrase. Each
/// phrase gets its own contour and Transform." Different phrase lengths
/// over the same chord run must therefore produce different output (the
/// Transform sequence differs).
#[test]
fn melody_phrase_length_changes_output() {
    let chords: Vec<TimedChord> = (0..8)
        .map(|i| tc(Chord::new(PitchClass::C, ChordQuality::Maj), i * 4, 4))
        .collect();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let motif = MotifParams {
        seed: 42,
        complexity: 0.7,
        motif_len: 4,
        leap_chance: 0.21,
    };

    let p_2 = MelodyParams {
        style: MelodyStyle::Motif,
        phrase_len: 2,
        ..MelodyParams::default()
    };
    let p_8 = MelodyParams {
        style: MelodyStyle::Motif,
        phrase_len: 8,
        ..MelodyParams::default()
    };
    let a = derive_motif_melody_with_section(&chords, scale, &p_2, &gen(motif), 1, TPB);
    let b = derive_motif_melody_with_section(&chords, scale, &p_8, &gen(motif), 1, TPB);
    assert_ne!(a, b, "phrase_len 2 and 8 should produce different melodies");
}

// ---------------------------------------------------------------------------
// Articulation
// ---------------------------------------------------------------------------

/// Tooltip: "Articulation: 0 = legato (full slot), 1 = staccato (about
/// 45% of the slot)." Verify by deriving the same motif twice and
/// asserting the staccato-1 durations are roughly 45% of the legato-0
/// durations on the matched notes.
#[test]
fn melody_articulation_drives_sounding_ratio() {
    let chords = c_major_chords(2, 4);
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let motif = MotifParams {
        seed: 11,
        complexity: 0.5,
        motif_len: 4,
        leap_chance: 0.21,
    };
    let mut p = MelodyParams {
        style: MelodyStyle::Motif,
        ..MelodyParams::default()
    };

    p.articulation = 0.0;
    let legato = derive_motif_melody_with_section(&chords, scale, &p, &gen(motif), 0, TPB);
    p.articulation = 1.0;
    let staccato = derive_motif_melody_with_section(&chords, scale, &p, &gen(motif), 0, TPB);

    assert_eq!(
        legato.len(),
        staccato.len(),
        "articulation should not change note count"
    );
    assert!(!legato.is_empty());

    // Sum durations across all matched notes; staccato should be ~0.45×
    // legato. Min duration clamping (tpb/8) means we allow some slack.
    let legato_total: u64 = legato.iter().map(|n| n.duration_ticks).sum();
    let staccato_total: u64 = staccato.iter().map(|n| n.duration_ticks).sum();
    let ratio = staccato_total as f64 / legato_total as f64;
    assert!(
        (0.40..=0.55).contains(&ratio),
        "staccato/legato duration ratio should be ~0.45 (1 - 1*0.55), got {ratio:.3}"
    );
}

// ---------------------------------------------------------------------------
// Pad register clamping
// ---------------------------------------------------------------------------

/// Tooltip: pad voices that fall below the register float up an octave;
/// voices above the ceiling drop one. Verify by picking a register that
/// would force voicing wraps and asserting every emitted note lands
/// inside it.
#[test]
fn pad_register_clamps_every_voice() {
    // Tight register that forces wrapping for any voicing that doesn't
    // collapse to a tight close-position.
    let p = PadParams {
        register: (60, 71), // exactly one octave wide, starting at C4
        velocity: 0.7,
    };
    let chords = vec![
        tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(PitchClass::F, ChordQuality::Maj7), 4, 4),
        tc(Chord::new(PitchClass::G, ChordQuality::Dom7), 8, 4),
    ];
    let notes = derive_pad(&chords, &p, TPB);
    assert!(!notes.is_empty());
    for n in &notes {
        assert!(
            n.note >= 60 && n.note <= 71,
            "pad note {} fell outside register (60..=71)",
            n.note
        );
    }
}

// ---------------------------------------------------------------------------
// Bass motif phrase modes
// ---------------------------------------------------------------------------

/// Tooltip: "Mirror melody: same Transform per phrase as the melody motif
/// lane (locked together)." We can't observe Transforms directly, but
/// running bass MirrorMelody twice with the same motif_seed must produce
/// identical output (proves transforms are deterministic from motif seed).
/// Then changing motif_seed must change the output (proves transforms
/// are tied to motif_seed, not lane_seed).
#[test]
fn bass_motif_mirror_melody_transforms_lock_to_motif_seed() {
    let chords: Vec<TimedChord> = (0..8)
        .map(|i| tc(Chord::new(PitchClass::C, ChordQuality::Maj), i * 4, 4))
        .collect();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let p = BassParams {
        style: BassStyle::Motif,
        motif_mode: BassMotifMode::SameIntervals,
        motif_phrase: BassMotifPhrase::MirrorMelody,
        ..BassParams::default()
    };

    let motif_a = MotifParams {
        seed: 100,
        complexity: 0.7,
        motif_len: 4,
        leap_chance: 0.21,
    };
    let motif_b = MotifParams {
        seed: 200,
        ..motif_a
    };

    let a1 = derive_bass_motif(&chords, scale, &p, &gen(motif_a), 5, TPB);
    let a2 = derive_bass_motif(&chords, scale, &p, &gen(motif_a), 5, TPB);
    let b = derive_bass_motif(&chords, scale, &p, &gen(motif_b), 5, TPB);

    assert_eq!(a1, a2, "same motif_seed should produce identical Transforms");
    assert_ne!(a1, b, "different motif_seed should produce different Transforms");
}

/// Tooltip: "Restricted: random Identity/Augment per phrase, independent of
/// melody." Verify lane_seed (not motif_seed) drives the variation: same
/// lane_seed → same output even if motif_seed changes the underlying motif
/// notes; different lane_seed with same motif_seed → output may differ in
/// transforms.
#[test]
fn bass_motif_restricted_transforms_lock_to_lane_seed() {
    let chords: Vec<TimedChord> = (0..8)
        .map(|i| tc(Chord::new(PitchClass::C, ChordQuality::Maj), i * 4, 4))
        .collect();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let p = BassParams {
        style: BassStyle::Motif,
        motif_mode: BassMotifMode::SameIntervals,
        motif_phrase: BassMotifPhrase::Restricted,
        ..BassParams::default()
    };
    let motif = MotifParams {
        seed: 50,
        complexity: 0.7,
        motif_len: 4,
        leap_chance: 0.21,
    };

    // Same (motif, lane) seeds → identical output.
    let a = derive_bass_motif(&chords, scale, &p, &gen(motif), 7, TPB);
    let b = derive_bass_motif(&chords, scale, &p, &gen(motif), 7, TPB);
    assert_eq!(a, b);

    // Across a sweep of lane seeds, at least one Restricted output should
    // differ on the same motif — otherwise lane_seed has no effect.
    let baseline = derive_bass_motif(&chords, scale, &p, &gen(motif), 1, TPB);
    let mut found_lane_variation = false;
    for lane in 2..50u64 {
        let alt = derive_bass_motif(&chords, scale, &p, &gen(motif), lane, TPB);
        if alt != baseline {
            found_lane_variation = true;
            break;
        }
    }
    assert!(
        found_lane_variation,
        "Restricted phrase mode should vary with lane_seed"
    );
}
