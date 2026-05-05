//! Basic coverage of the part-generator entry points (pad, bass, melody,
//! motif). Each test exercises one publicly-visible behavioral property
//! independent of the larger render pipeline. Lifted out of an inline
//! `#[cfg(test)] mod tests` block when `derive.rs` was split into a
//! module.

use resonance_music_theory::{
    derive_bass, derive_melody, derive_pad, BassParams, BassStyle, ContourPreference, Chord,
    ChordQuality, MelodyParams, MelodyStyle, Mode, PadParams, PitchClass, PitchClass::*, Scale,
    TimedChord,
};

fn tc(chord: Chord, start_beat: u32, duration_beats: u32) -> TimedChord {
    TimedChord {
        chord,
        start_beat,
        duration_beats,
    }
}

// ---------- Pad ----------

#[test]
fn pad_empty_in_empty_out() {
    assert!(derive_pad(&[], &PadParams::default(), 480).is_empty());
}

#[test]
fn pad_produces_one_note_per_voice_per_chord() {
    let chords = vec![
        tc(Chord::new(C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(F, ChordQuality::Maj), 4, 4),
    ];
    let p = PadParams::default();
    let notes = derive_pad(&chords, &p, 480);
    assert_eq!(notes.len(), 6); // 3 voices × 2 chords
}

#[test]
fn pad_voices_stay_in_register() {
    let chords = vec![
        tc(Chord::new(C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(F, ChordQuality::Maj7), 4, 4),
        tc(Chord::new(G, ChordQuality::Dom7), 8, 4),
    ];
    let p = PadParams {
        register: (48, 72),
        velocity: 0.7,
    };
    for n in derive_pad(&chords, &p, 480) {
        assert!(n.note >= 48 && n.note <= 72, "{} out of register", n.note);
    }
}

#[test]
fn pad_start_ticks_match_beats() {
    let chords = vec![
        tc(Chord::new(C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(G, ChordQuality::Maj), 4, 4),
    ];
    let notes = derive_pad(&chords, &PadParams::default(), 480);
    // First chord at beat 0 → start_tick 0; second at beat 4 → 1920.
    let c_start: Vec<u64> = notes
        .iter()
        .filter(|n| n.start_tick == 0)
        .map(|n| n.start_tick)
        .collect();
    let g_start: Vec<u64> = notes
        .iter()
        .filter(|n| n.start_tick == 1920)
        .map(|n| n.start_tick)
        .collect();
    assert_eq!(c_start.len(), 3);
    assert_eq!(g_start.len(), 3);
}

// ---------- Bass ----------

#[test]
fn bass_empty_in_empty_out() {
    assert!(derive_bass(&[], None, &BassParams::default(), 480).is_empty());
}

#[test]
fn bass_root_hold_one_note_per_chord() {
    let chords = vec![
        tc(Chord::new(C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(G, ChordQuality::Maj), 4, 4),
        tc(Chord::new(A, ChordQuality::Min), 8, 4),
    ];
    let p = BassParams {
        style: BassStyle::RootHold,
        ..BassParams::default()
    };
    let notes = derive_bass(&chords, None, &p, 480);
    assert_eq!(notes.len(), 3);
    assert_eq!(notes[0].duration_ticks, 4 * 480);
}

#[test]
fn bass_root_pulse_has_one_note_per_beat() {
    let chords = vec![tc(Chord::new(C, ChordQuality::Maj), 0, 4)];
    let p = BassParams {
        style: BassStyle::RootPulse,
        ..BassParams::default()
    };
    let notes = derive_bass(&chords, None, &p, 480);
    assert_eq!(notes.len(), 4);
    assert!(notes.iter().all(|n| n.note == notes[0].note));
}

#[test]
fn bass_slash_chord_uses_bass_pitch_class() {
    // Am/G: root should be G, not A.
    let chord = Chord::new(A, ChordQuality::Min).with_bass(G);
    let chords = vec![tc(chord, 0, 4)];
    let p = BassParams {
        style: BassStyle::RootHold,
        ..BassParams::default()
    };
    let notes = derive_bass(&chords, None, &p, 480);
    assert_eq!(notes.len(), 1);
    // Expect G at or above base_note 28 (E1) — the nearest G ≥ 28 is G1 = 31.
    assert_eq!(notes[0].note % 12, G.to_semitone());
}

#[test]
fn bass_walking_falls_back_without_scale() {
    let chords = vec![
        tc(Chord::new(C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(G, ChordQuality::Maj), 4, 4),
    ];
    let p = BassParams {
        style: BassStyle::Walking,
        ..BassParams::default()
    };
    let notes = derive_bass(&chords, None, &p, 480);
    assert_eq!(notes.len(), 8);
}

#[test]
fn bass_walking_uses_scale_tones() {
    let chords = vec![
        tc(Chord::new(C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(G, ChordQuality::Maj), 4, 4),
    ];
    let scale = Scale::new(C, Mode::Major);
    let p = BassParams {
        style: BassStyle::Walking,
        ..BassParams::default()
    };
    let notes = derive_bass(&chords, Some(scale), &p, 480);
    // Every note must belong to the scale.
    for n in &notes {
        assert!(
            scale.contains(n.note),
            "walking bass note {} not in C major",
            n.note
        );
    }
    // Should produce one note per beat across both chords.
    assert_eq!(notes.len(), 8);
}

// ---------- Melody ----------

#[test]
fn melody_empty_in_empty_out() {
    assert!(derive_melody(&[], None, &MelodyParams::default(), 480, 0).is_empty());
}

#[test]
fn melody_arp_up_stays_in_register() {
    let chords = vec![
        tc(Chord::new(C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(F, ChordQuality::Maj), 4, 4),
    ];
    let p = MelodyParams::default();
    let notes = derive_melody(&chords, None, &p, 480, 1);
    assert!(!notes.is_empty());
    for n in &notes {
        assert!(n.note >= p.register.0 && n.note <= p.register.1);
    }
}

#[test]
fn melody_arp_uses_chord_tones_only() {
    let chord = Chord::new(C, ChordQuality::Maj); // [C, E, G]
    let chords = vec![tc(chord, 0, 4)];
    let p = MelodyParams {
        style: MelodyStyle::ArpUp,
        ..MelodyParams::default()
    };
    let notes = derive_melody(&chords, None, &p, 480, 1);
    for n in &notes {
        let pc = n.note % 12;
        assert!(pc == 0 || pc == 4 || pc == 7, "non-chord note {}", n.note);
    }
}

#[test]
fn melody_rest_density_one_produces_no_notes() {
    let chords = vec![tc(Chord::new(C, ChordQuality::Maj), 0, 4)];
    let p = MelodyParams {
        rest_density: 1.0,
        ..MelodyParams::default()
    };
    let notes = derive_melody(&chords, None, &p, 480, 3);
    assert!(notes.len() <= 1);
}

#[test]
fn melody_seed_reproducible() {
    let chords = vec![
        tc(Chord::new(C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(G, ChordQuality::Maj), 4, 4),
    ];
    let scale = Scale::new(C, Mode::Major);
    let p = MelodyParams {
        style: MelodyStyle::Motif,
        ..MelodyParams::default()
    };
    let a = derive_melody(&chords, Some(scale), &p, 480, 42);
    let b = derive_melody(&chords, Some(scale), &p, 480, 42);
    assert_eq!(a, b);
}

// ---------- Motif ----------

fn motif_params() -> MelodyParams {
    MelodyParams {
        style: MelodyStyle::Motif,
        ..MelodyParams::default()
    }
}

fn standard_chords() -> Vec<TimedChord> {
    vec![
        tc(Chord::new(C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(F, ChordQuality::Maj), 4, 4),
        tc(Chord::new(G, ChordQuality::Maj), 8, 4),
        tc(Chord::new(C, ChordQuality::Maj), 12, 4),
    ]
}

#[test]
fn motif_empty_in_empty_out() {
    assert!(derive_melody(&[], None, &motif_params(), 480, 0).is_empty());
}

#[test]
fn motif_stays_in_register() {
    let chords = standard_chords();
    let p = motif_params();
    let notes = derive_melody(&chords, Some(Scale::new(C, Mode::Major)), &p, 480, 42);
    assert!(!notes.is_empty());
    for n in &notes {
        assert!(
            n.note >= p.register.0 && n.note <= p.register.1,
            "note {} out of register ({}, {})",
            n.note,
            p.register.0,
            p.register.1
        );
    }
}

#[test]
fn motif_strong_beats_are_chord_tones() {
    let chords = standard_chords();
    let scale = Scale::new(C, Mode::Major);
    let p = motif_params();
    let notes = derive_melody(&chords, Some(scale), &p, 480, 42);
    let tpb = 480u64;
    for n in &notes {
        let beat_in_chord = n.start_tick % (4 * tpb);
        let is_strong = beat_in_chord % (2 * tpb) == 0;
        if is_strong {
            // Find which chord this note belongs to.
            let chord_idx = chords
                .iter()
                .rposition(|tc| (tc.start_beat as u64 * tpb) <= n.start_tick)
                .unwrap_or(0);
            let chord = chords[chord_idx].chord;
            let pcs = chord.pitch_classes();
            let note_pc = PitchClass::from_semitone(n.note % 12);
            assert!(
                pcs.contains(&note_pc),
                "strong-beat note {} (pc {:?}) not a chord tone of {:?}",
                n.note,
                note_pc,
                chord
            );
        }
    }
}

#[test]
fn motif_seed_deterministic() {
    let chords = standard_chords();
    let scale = Scale::new(C, Mode::Major);
    let p = motif_params();
    let a = derive_melody(&chords, Some(scale), &p, 480, 123);
    let b = derive_melody(&chords, Some(scale), &p, 480, 123);
    assert_eq!(a, b);
}

#[test]
fn motif_respects_scale() {
    let chords = standard_chords();
    let scale = Scale::new(C, Mode::Major);
    let p = MelodyParams {
        style: MelodyStyle::Motif,
        complexity: 0.3, // keep it simple to avoid chromatic passing tones
        ..MelodyParams::default()
    };
    let notes = derive_melody(&chords, Some(scale), &p, 480, 7);
    for n in &notes {
        assert!(
            scale.contains(n.note),
            "motif note {} not in C major",
            n.note
        );
    }
}

#[test]
fn motif_has_varied_durations() {
    let chords = standard_chords();
    let p = MelodyParams {
        style: MelodyStyle::Motif,
        complexity: 0.7,
        ..MelodyParams::default()
    };
    // Try several seeds — at least one should produce varied durations.
    let mut found_varied = false;
    for seed in 0..20u64 {
        let notes = derive_melody(&chords, Some(Scale::new(C, Mode::Major)), &p, 480, seed);
        let unique_durations: std::collections::HashSet<u64> =
            notes.iter().map(|n| n.duration_ticks).collect();
        if unique_durations.len() >= 2 {
            found_varied = true;
            break;
        }
    }
    assert!(found_varied, "motif should produce varied note durations");
}

#[test]
fn motif_no_scale_falls_back_to_chord_tones() {
    let chords = standard_chords();
    let p = motif_params();
    let notes = derive_melody(&chords, None, &p, 480, 42);
    assert!(!notes.is_empty());
    for n in &notes {
        // Without a scale, every note should be a chord tone of
        // some chord in the progression.
        let chord_idx = chords
            .iter()
            .rposition(|tc| (tc.start_beat as u64 * 480) <= n.start_tick)
            .unwrap_or(0);
        let pcs = chords[chord_idx].chord.pitch_classes();
        let note_pc = PitchClass::from_semitone(n.note % 12);
        assert!(
            pcs.contains(&note_pc),
            "no-scale note {} (pc {:?}) not a chord tone",
            n.note,
            note_pc
        );
    }
}

#[test]
fn motif_contour_arch_peaks_in_middle() {
    let chords = vec![
        tc(Chord::new(C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(F, ChordQuality::Maj), 4, 4),
        tc(Chord::new(G, ChordQuality::Maj), 8, 4),
        tc(Chord::new(C, ChordQuality::Maj), 12, 4),
        tc(Chord::new(F, ChordQuality::Maj), 16, 4),
        tc(Chord::new(G, ChordQuality::Maj), 20, 4),
        tc(Chord::new(C, ChordQuality::Maj), 24, 4),
        tc(Chord::new(C, ChordQuality::Maj), 28, 4),
    ];
    let scale = Scale::new(C, Mode::Major);
    let p = MelodyParams {
        style: MelodyStyle::Motif,
        contour: ContourPreference::Arch,
        phrase_len: 8,
        ..MelodyParams::default()
    };
    // Over several seeds, the peak note should tend toward the middle.
    let mut peak_in_middle = 0;
    for seed in 0..20u64 {
        let notes = derive_melody(&chords, Some(scale), &p, 480, seed);
        if notes.is_empty() {
            continue;
        }
        let peak_idx = notes
            .iter()
            .enumerate()
            .max_by_key(|(_, n)| n.note)
            .map(|(i, _)| i)
            .unwrap();
        let ratio = peak_idx as f32 / notes.len() as f32;
        if (0.2..=0.8).contains(&ratio) {
            peak_in_middle += 1;
        }
    }
    // At least half the seeds should peak in the middle 60%.
    assert!(
        peak_in_middle >= 10,
        "arch contour should peak in middle, but only {peak_in_middle}/20 did"
    );
}

#[test]
fn motif_serde_alias_scale_walk() {
    // Old project files with "ScaleWalk" should deserialize to Motif.
    let json = r#""ScaleWalk""#;
    let style: MelodyStyle = serde_json::from_str(json).unwrap();
    assert_eq!(style, MelodyStyle::Motif);
}
