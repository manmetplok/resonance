//! Section-level climax orchestration (Open Music Theory v2: one
//! climax per *section*, not one per phrase): the section designates a
//! single phrase — the sentence continuation or the period chain's
//! departure-position antecedent, phrase 3 of 4 — as the carrier of
//! the section's highest note, and every other phrase's peak is
//! demoted strictly below it (the consequent paired with a carrier
//! antecedent may tie, preserving the period's parallel structure).
//! Same idea for vocal lines: the srdc departure line carries the
//! section peak, fixing the old behavior where every lyric line arched
//! to an identical top.
//!
//! Composes with the per-phrase/per-line single-climax rule
//! (tests/phrase_climax.rs, tests/vocal_climax.rs), the cadence
//! overlay (tests/cadence_formulas.rs), and the strong-beat contract
//! (tests/rhythm_transforms.rs) — those suites stay authoritative for
//! their invariants; this one asserts the section-level skyline.
//!
//! The engine skips sections whose carrier peak rides within an octave
//! of the register floor (no headroom to demote into), so the
//! assertions below filter to seeds whose carrier demonstrably sits
//! above that band.

use resonance_music_theory::{
    count_syllables, derive_motif_melody_with_section, derive_vocal, generate_lyrics,
    phrase_grammar_roles, section_climax_phrase, Chord, ChordQuality, ContourPreference,
    GeneratedNote, MelodyParams, MelodyStyle, Mode, MotifParams, MotifSource, PhraseGrammarRole,
    PitchClass, Scale, TimedChord, VocalParams, VocalStyle,
};

const TPB: u64 = 480;
const REGISTER: (u8, u8) = (48, 84);

fn tc(root: PitchClass, quality: ChordQuality, start: u32, dur: u32) -> TimedChord {
    TimedChord {
        chord: Chord::new(root, quality),
        start_beat: start,
        duration_beats: dur,
    }
}

/// 8 chords / phrase_len 2 = 4 phrases — one full sentence or period
/// group, the canonical section shape.
fn eight_chords() -> Vec<TimedChord> {
    vec![
        tc(PitchClass::C, ChordQuality::Maj, 0, 4),
        tc(PitchClass::A, ChordQuality::Min, 4, 4),
        tc(PitchClass::F, ChordQuality::Maj, 8, 4),
        tc(PitchClass::G, ChordQuality::Maj, 12, 4),
        tc(PitchClass::C, ChordQuality::Maj, 16, 4),
        tc(PitchClass::A, ChordQuality::Min, 20, 4),
        tc(PitchClass::G, ChordQuality::Maj, 24, 4),
        tc(PitchClass::C, ChordQuality::Maj, 28, 4),
    ]
}

fn generate_melody(seed: u64) -> Vec<GeneratedNote> {
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let params = MelodyParams {
        style: MelodyStyle::Motif,
        phrase_len: 2,
        rest_density: 0.0,
        complexity: 0.6,
        leap_chance: 0.3,
        contour: ContourPreference::Auto,
        register: REGISTER,
        ..MelodyParams::default()
    };
    let source = MotifSource::Generated(MotifParams {
        seed,
        complexity: 0.6,
        motif_len: 0,
        leap_chance: 0.3,
    });
    derive_motif_melody_with_section(&eight_chords(), scale, &params, &source, seed, TPB as u32)
}

/// Per-phrase pitch slices by tick range (phrase_len 2 over 4-beat
/// chords = 8 beats per phrase).
fn phrase_pitches(notes: &[GeneratedNote], num_phrases: usize) -> Vec<Vec<u8>> {
    let phrase_ticks = 8 * TPB;
    let mut out = vec![Vec::new(); num_phrases];
    for n in notes {
        let pi = ((n.start_tick / phrase_ticks) as usize).min(num_phrases - 1);
        out[pi].push(n.note);
    }
    out
}

// ---------------------------------------------------------------------------
// Carrier designation
// ---------------------------------------------------------------------------

#[test]
fn carrier_is_the_continuation_or_departure_antecedent() {
    use PhraseGrammarRole::*;
    // Sentence: the continuation (phrase 3 of 4) carries the climax.
    assert_eq!(
        section_climax_phrase(&[BasicIdea, VariedRepeat, Continuation, ContinuationCadence]),
        2
    );
    // Period chain: the departure-position second antecedent.
    assert_eq!(
        section_climax_phrase(&[Antecedent, Consequent, Antecedent, Consequent]),
        2
    );
    // A lone period: the antecedent opens, the consequent resolves.
    assert_eq!(section_climax_phrase(&[Antecedent, Consequent]), 0);
    assert_eq!(section_climax_phrase(&[Antecedent]), 0);
    // Longer sections place the climax late: the last group's open
    // phrase.
    assert_eq!(
        section_climax_phrase(&[
            BasicIdea,
            VariedRepeat,
            Continuation,
            ContinuationCadence,
            BasicIdea,
            VariedRepeat,
            Continuation,
            ContinuationCadence,
        ]),
        6
    );
    assert_eq!(
        section_climax_phrase(&[
            BasicIdea,
            VariedRepeat,
            Continuation,
            ContinuationCadence,
            Antecedent,
            Consequent,
        ]),
        4
    );
}

// ---------------------------------------------------------------------------
// Instrumental: the designated phrase carries the section peak
// ---------------------------------------------------------------------------

#[test]
fn sentence_continuation_carries_the_strict_section_peak() {
    // Sentence groups have no consequents, so every non-carrier phrase
    // must peak strictly below the continuation. Filter to seeds whose
    // carrier demonstrably cleared the engine's headroom guard
    // (carrier peak at least an octave-and-a-bit above the floor; the
    // small extra margin covers the embellishment pass's ability to
    // raise the carrier after the section pass measured it).
    let mut asserted = 0usize;
    for seed in 0..300u64 {
        let roles = phrase_grammar_roles(4, seed);
        if roles[0] != PhraseGrammarRole::BasicIdea {
            continue; // period group — covered below
        }
        let carrier = section_climax_phrase(&roles);
        assert_eq!(carrier, 2, "sentence carrier should be the continuation");
        let notes = generate_melody(seed);
        let phrases = phrase_pitches(&notes, 4);
        if phrases.iter().any(|p| p.is_empty()) {
            continue;
        }
        let carrier_max = *phrases[carrier].iter().max().unwrap();
        if carrier_max < REGISTER.0 + 15 {
            continue; // engine may have skipped: no headroom to demote into
        }
        asserted += 1;
        for (pi, phrase) in phrases.iter().enumerate() {
            if pi == carrier {
                continue;
            }
            let pmax = *phrase.iter().max().unwrap();
            assert!(
                pmax < carrier_max,
                "seed {seed}: phrase {pi} peaks at {pmax}, not strictly below \
                 the carrier's {carrier_max}"
            );
        }
    }
    assert!(asserted >= 80, "too few enforced sentence seeds ({asserted})");
}

#[test]
fn period_chain_peak_lives_in_the_second_pair() {
    // Period chains: the carrier is the second antecedent and its
    // paired consequent may tie it (the period restates its material),
    // but the first pair must stay strictly below the carrier.
    let mut asserted = 0usize;
    for seed in 0..300u64 {
        let roles = phrase_grammar_roles(4, seed);
        if roles[0] != PhraseGrammarRole::Antecedent {
            continue;
        }
        let carrier = section_climax_phrase(&roles);
        assert_eq!(carrier, 2, "period carrier should be the second antecedent");
        let notes = generate_melody(seed);
        let phrases = phrase_pitches(&notes, 4);
        if phrases.iter().any(|p| p.is_empty()) {
            continue;
        }
        let carrier_max = *phrases[carrier].iter().max().unwrap();
        if carrier_max < REGISTER.0 + 15 {
            continue;
        }
        asserted += 1;
        for pi in [0usize, 1] {
            let pmax = *phrases[pi].iter().max().unwrap();
            assert!(
                pmax < carrier_max,
                "seed {seed}: first-pair phrase {pi} peaks at {pmax}, not strictly \
                 below the carrier's {carrier_max}"
            );
        }
        // The carrier's own consequent may tie but never exceed.
        let cons_max = *phrases[3].iter().max().unwrap();
        assert!(
            cons_max <= carrier_max,
            "seed {seed}: paired consequent peaks at {cons_max}, above the \
             carrier's {carrier_max}"
        );
    }
    assert!(asserted >= 80, "too few enforced period seeds ({asserted})");
}

#[test]
fn per_phrase_single_climax_still_holds_in_multi_phrase_sections() {
    // The section pass extends — and must not regress — the per-phrase
    // single-climax rule from tests/phrase_climax.rs, which only
    // exercises single-phrase sections.
    for seed in 0..300u64 {
        let notes = generate_melody(seed);
        for (pi, pitches) in phrase_pitches(&notes, 4).iter().enumerate() {
            let n = pitches.len();
            if n < 3 {
                continue;
            }
            let max = *pitches.iter().max().unwrap();
            let min = *pitches.iter().min().unwrap();
            if max == min {
                continue; // flat: exempt
            }
            let window_max = *pitches[n / 2..n - 1].iter().max().unwrap();
            if window_max <= REGISTER.0 {
                continue; // climax window pinned on the register floor
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
                "seed {seed}: phrase {pi} has peaks at {peaks:?} in {pitches:?}"
            );
            assert!(
                peaks[0] >= n / 2,
                "seed {seed}: phrase {pi} climax in the first half ({pitches:?})"
            );
            assert_ne!(
                peaks[0],
                n - 1,
                "seed {seed}: phrase {pi} climax on the final note ({pitches:?})"
            );
        }
    }
}

#[test]
fn section_climax_orchestration_is_deterministic() {
    for seed in [7u64, 42, 0xFEED] {
        assert_eq!(generate_melody(seed), generate_melody(seed));
    }
}

// ---------------------------------------------------------------------------
// Vocal: the srdc departure line carries the section peak
// ---------------------------------------------------------------------------

fn c_major_chords() -> Vec<TimedChord> {
    (0..4)
        .map(|i| tc(PitchClass::C, ChordQuality::Maj, i * 4, 4))
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

#[test]
fn departure_line_carries_the_strict_section_peak() {
    // 4-line drafts form one statement–restatement–departure–conclusion
    // group; the departure (line 3 of 4) is the designated carrier and
    // every other line must peak strictly below it. This is the fix
    // for "every vocal line arches identically".
    for style in [
        VocalStyle::PopBallad,
        VocalStyle::Anthemic,
        VocalStyle::Conversational,
        VocalStyle::Folk,
    ] {
        let mut asserted = 0usize;
        for seed in 0..80u64 {
            let mut p = VocalParams::default();
            p.style = style;
            p.draft = generate_lyrics(&p, seed.wrapping_add(17));
            let notes = derive_vocal(&c_major_chords(), &p, TPB as u32, seed);
            let slices = line_slices(&notes, &p);
            if slices.len() != 4 {
                continue;
            }
            let carrier = 2usize; // srdc departure
            let carrier_max = slices[carrier].iter().map(|n| n.note).max().unwrap_or(0);
            if carrier_max < p.range.0 + 4 {
                continue; // degenerate: engine skips floor-pinned carriers
            }
            asserted += 1;
            for (li, slice) in slices.iter().enumerate() {
                if li == carrier {
                    continue;
                }
                let lmax = slice.iter().map(|n| n.note).max().unwrap_or(0);
                assert!(
                    lmax < carrier_max,
                    "style {style:?}, seed {seed}: line {li} peaks at {lmax}, not \
                     strictly below the departure's {carrier_max}"
                );
            }
        }
        assert!(
            asserted >= 50,
            "style {style:?}: too few enforced sections ({asserted})"
        );
    }
}

#[test]
fn vocal_line_peaks_are_no_longer_identical() {
    // Regression target of the task: before section orchestration,
    // independent per-line contour draws gave every line the same top.
    // Now the four line peaks can never all be equal in an enforced
    // section (the carrier sits strictly above), and across seeds the
    // secondary peaks themselves vary.
    let mut all_equal = 0usize;
    let mut total = 0usize;
    for seed in 0..80u64 {
        let mut p = VocalParams::default();
        p.draft = generate_lyrics(&p, seed.wrapping_add(17));
        let notes = derive_vocal(&c_major_chords(), &p, TPB as u32, seed);
        let slices = line_slices(&notes, &p);
        if slices.len() != 4 {
            continue;
        }
        let peaks: Vec<u8> = slices
            .iter()
            .map(|s| s.iter().map(|n| n.note).max().unwrap_or(0))
            .collect();
        total += 1;
        if peaks.iter().all(|&x| x == peaks[0]) {
            all_equal += 1;
        }
    }
    assert!(total >= 60, "too few 4-line sections ({total})");
    assert!(
        all_equal as f32 <= total as f32 * 0.05,
        "{all_equal}/{total} sections still have identical line peaks"
    );
}

#[test]
fn vocal_section_climax_is_deterministic() {
    let mut p = VocalParams::default();
    p.draft = generate_lyrics(&p, 33);
    let a = derive_vocal(&c_major_chords(), &p, TPB as u32, 33);
    let b = derive_vocal(&c_major_chords(), &p, TPB as u32, 33);
    assert_eq!(a, b);
}
