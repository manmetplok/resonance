//! Basic coverage of the part-generator entry points (pad, bass, melody,
//! motif). Each test exercises one publicly-visible behavioral property
//! independent of the larger render pipeline. Lifted out of an inline
//! `#[cfg(test)] mod tests` block when `derive.rs` was split into a
//! module.

use resonance_music_theory::{
    derive_bass, derive_melody, derive_melody_fill_vocal, derive_pad, derive_vocal_with_meter,
    generate_lyrics, vocal_phrase_spans, BassParams, BassStyle, ContourPreference, Chord,
    ChordQuality, MelodyParams, MelodyStyle, Mode, PadParams, PitchClass, PitchClass::*, Scale,
    TimedChord, VocalParams,
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

// ---------- derive_melody_fill_vocal (fill_vocal_gaps generator) ----------

/// Phrase-level interval (start_tick, end_tick) — the unit the fill
/// generator expects, *not* per-syllable notes. See the doc comment on
/// `derive_melody_fill_vocal` for why.
fn span(start: u64, end: u64) -> (u64, u64) {
    (start, end)
}

fn one_chord_section(beats: u32) -> Vec<TimedChord> {
    vec![TimedChord {
        chord: Chord::new(PitchClass::C, ChordQuality::Maj),
        start_beat: 0,
        duration_beats: beats,
    }]
}

fn arp_params() -> MelodyParams {
    MelodyParams {
        style: MelodyStyle::ArpUp,
        register: (60, 84),
        note_value_ticks: 240,  // eighth notes at TPQN=480
        velocity: 0.8,
        ..Default::default()
    }
}

fn section_end(chords: &[TimedChord]) -> u64 {
    chords
        .iter()
        .map(|c| (c.start_beat + c.duration_beats) as u64 * 480)
        .max()
        .unwrap_or(0)
}

#[test]
fn fill_vocal_with_no_vocal_fills_entire_section() {
    // 4 beats = 1920 ticks, 8 eighth notes expected.
    let chords = one_chord_section(4);
    let end = section_end(&chords);
    let notes = derive_melody_fill_vocal(&chords, &arp_params(), &[], end, 480, 60);
    assert_eq!(notes.len(), 8, "every eighth-note slot should get a note");
    let starts: Vec<u64> = notes.iter().map(|n| n.start_tick).collect();
    assert_eq!(starts, vec![0, 240, 480, 720, 960, 1200, 1440, 1680]);
}

#[test]
fn fill_vocal_skips_slots_inside_vocal_notes() {
    // Vocal phrase spans 0..200 and 800..1000. Two silence intervals:
    // 200..800 and 1000..1920. Slots are anchored to each silence
    // start, stepping by note_value_ticks (240).
    let chords = one_chord_section(4);
    let end = section_end(&chords);
    let vocal = vec![span(0, 200), span(800, 1000)];
    let notes = derive_melody_fill_vocal(&chords, &arp_params(), &vocal, end, 480, 0);
    let starts: Vec<u64> = notes.iter().map(|n| n.start_tick).collect();
    assert_eq!(starts, vec![200, 440, 680, 1000, 1240, 1480, 1720]);
    for n in &notes {
        let inside = vocal
            .iter()
            .any(|(s, e)| n.start_tick >= *s && n.start_tick < *e);
        assert!(!inside, "fill at {} lands inside a vocal span", n.start_tick);
    }
}

#[test]
fn fill_vocal_trims_tail_at_next_vocal_with_min_gap() {
    let chords = one_chord_section(4);
    let end = section_end(&chords);
    let vocal = vec![span(1000, 1200)];
    let notes = derive_melody_fill_vocal(&chords, &arp_params(), &vocal, end, 480, 60);
    let trimmed = notes.iter().find(|n| n.start_tick == 720).expect("slot 720");
    assert_eq!(trimmed.start_tick + trimmed.duration_ticks, 940);
}

#[test]
fn fill_vocal_skips_slots_with_subperceptible_duration() {
    let chords = one_chord_section(4);
    let end = section_end(&chords);
    let vocal = vec![span(250, 1920)];
    let notes = derive_melody_fill_vocal(&chords, &arp_params(), &vocal, end, 480, 60);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].start_tick, 0);
    assert_eq!(notes[0].duration_ticks, 190);
}

#[test]
fn fill_vocal_respects_explicit_section_end_shorter_than_chord_span() {
    // Chord progression spans 16 beats, but the section is only
    // 8 beats long. No vocal. The fill must stop at the section
    // boundary, not at the chord-progression end.
    let chords: Vec<TimedChord> = (0..4)
        .map(|i| TimedChord {
            chord: Chord::new(PitchClass::C, ChordQuality::Maj),
            start_beat: i * 4,
            duration_beats: 4,
        })
        .collect();
    let section_end = 8u64 * 480; // 2 bars
    let notes = derive_melody_fill_vocal(&chords, &arp_params(), &[], section_end, 480, 60);
    assert!(
        notes.iter().all(|n| n.start_tick + n.duration_ticks <= section_end),
        "fill leaked past explicit section_end_ticks"
    );
    // 16 eighth notes fit in 2 bars.
    assert_eq!(notes.len(), 16);
}

#[test]
fn fill_vocal_dense_vocal_yields_notes_in_phrase_gaps() {
    let chords: Vec<TimedChord> = (0..4)
        .map(|i| TimedChord {
            chord: Chord::new(PitchClass::C, ChordQuality::Maj),
            start_beat: i * 4,
            duration_beats: 4,
        })
        .collect();
    let end = section_end(&chords);
    let mut vp = VocalParams::default();
    vp.draft = generate_lyrics(&vp, 42);
    let vocal_notes = derive_vocal_with_meter(&chords, &vp, 480, 4, 42);
    let vocal = vocal_phrase_spans(&vocal_notes, &vp);
    let mut params = arp_params();
    params.register = (60, 84);
    let notes = derive_melody_fill_vocal(&chords, &params, &vocal, end, 480, 60);

    for n in &notes {
        let inside_phrase = vocal
            .iter()
            .any(|(s, e)| n.start_tick >= *s && n.start_tick < *e);
        assert!(
            !inside_phrase,
            "fill note at {} lands inside a vocal phrase",
            n.start_tick
        );
    }

    let quarter = end / 4;
    let quarters_used: std::collections::HashSet<u64> =
        notes.iter().map(|n| n.start_tick / quarter).collect();
    assert!(!notes.is_empty(), "expected some fill notes between vocal phrases");
    assert!(
        quarters_used.len() >= 2,
        "fill notes should land in at least 2 different quarters of the section, got {:?}",
        quarters_used
    );
}

#[test]
fn fill_vocal_realistic_two_phrases_get_filled_between() {
    let chords: Vec<TimedChord> = (0..8)
        .map(|i| TimedChord {
            chord: Chord::new(PitchClass::C, ChordQuality::Maj),
            start_beat: i * 4,
            duration_beats: 4,
        })
        .collect();
    let end = section_end(&chords);
    // Two phrases, each spanning 6 syllables (250-tick stride, last
    // note ends 200 ticks later): phrase 1 = 0..1450, phrase 2 =
    // 7000..8450.
    let vocal = vec![span(0, 1450), span(7000, 8450)];

    let notes = derive_melody_fill_vocal(&chords, &arp_params(), &vocal, end, 480, 60);

    let mid_gap_notes: Vec<u64> = notes
        .iter()
        .filter(|n| n.start_tick >= 1500 && n.start_tick < 7000)
        .map(|n| n.start_tick)
        .collect();
    let tail_notes: Vec<u64> = notes
        .iter()
        .filter(|n| n.start_tick >= 8500)
        .map(|n| n.start_tick)
        .collect();

    assert!(
        mid_gap_notes.len() >= 16,
        "expected ≥16 eighth-note fills in the 5500-tick mid gap, got {} ({:?})",
        mid_gap_notes.len(),
        mid_gap_notes
    );
    assert!(
        !tail_notes.is_empty(),
        "expected fill notes after the last vocal phrase, got 0"
    );
}

#[test]
fn fill_vocal_phrase_spans_block_intra_phrase_stubs() {
    // Regression: when the call site fed raw per-syllable notes the
    // arp wedged a stub note into every quarter-note gap between
    // syllables of a single phrase. The fix is to pass phrase-level
    // spans (one per lyric line) so the algorithm sees the phrase
    // as a single occupied region.
    let chords = one_chord_section(4);
    let end = section_end(&chords);
    // One phrase = three 200-tick syllables with 200-tick gaps:
    // 0..200, 400..600, 800..1000. As a phrase span: 0..1000.
    let vocal = vec![span(0, 1000)];
    let notes = derive_melody_fill_vocal(&chords, &arp_params(), &vocal, end, 480, 60);
    let stubs_in_phrase: Vec<u64> = notes
        .iter()
        .filter(|n| n.start_tick < 1000)
        .map(|n| n.start_tick)
        .collect();
    assert!(
        stubs_in_phrase.is_empty(),
        "fill must not wedge stubs inside the phrase span (got {:?})",
        stubs_in_phrase
    );
    // And the tail after the phrase still gets filled.
    assert!(notes.iter().any(|n| n.start_tick >= 1000));
}

#[test]
fn fill_vocal_end_to_end_two_lyric_lines_with_breath_gap() {
    // End-to-end reproduction of a real session: two lyric lines with a
    // pop-ballad breath_frac that leaves a multi-bar silence in the
    // middle of the section. The fill must produce notes in that
    // breath gap, not just at the tail.
    use resonance_music_theory::LyricLine;
    let chords: Vec<TimedChord> = (0..4)
        .map(|i| TimedChord {
            chord: Chord::new(PitchClass::C, ChordQuality::Maj),
            start_beat: i * 8,
            duration_beats: 8,
        })
        .collect();
    let end = section_end(&chords); // 32 beats * 480 = 15360 ticks
    let mut vp = VocalParams::default();
    vp.breath = 0.5;
    vp.draft = vec![
        LyricLine {
            n: 1,
            rhyme: 'A',
            syllables: 6,
            text: "one two three four five six".to_string(),
            locked: false,
        },
        LyricLine {
            n: 2,
            rhyme: 'A',
            syllables: 6,
            text: "sev en eight nine ten ten".to_string(),
            locked: false,
        },
    ];
    let vocal_notes = derive_vocal_with_meter(&chords, &vp, 480, 4, 42);
    let vocal_spans = vocal_phrase_spans(&vocal_notes, &vp);
    assert_eq!(
        vocal_spans.len(),
        2,
        "expected one span per lyric line, got {:?}",
        vocal_spans
    );
    let (s1_start, s1_end) = vocal_spans[0];
    let (s2_start, s2_end) = vocal_spans[1];
    assert!(
        s1_end < s2_start,
        "phrase 1 must end before phrase 2 starts (got {}..{} and {}..{})",
        s1_start, s1_end, s2_start, s2_end
    );
    let gap_span = s2_start - s1_end;
    assert!(
        gap_span > 480, // > 1 beat
        "expected a real breath gap, got {} ticks",
        gap_span
    );

    let mut params = arp_params();
    params.register = (60, 84);
    let fill = derive_melody_fill_vocal(&chords, &params, &vocal_spans, end, 480, 60);
    let gap_fills: Vec<u64> = fill
        .iter()
        .filter(|n| n.start_tick >= s1_end && n.start_tick + n.duration_ticks <= s2_start)
        .map(|n| n.start_tick)
        .collect();
    assert!(
        !gap_fills.is_empty(),
        "fill must place notes in the breath gap between phrase 1 ({}..{}) and phrase 2 ({}..{}); got fills at {:?}",
        s1_start, s1_end, s2_start, s2_end,
        fill.iter().map(|n| n.start_tick).collect::<Vec<_>>()
    );
}
