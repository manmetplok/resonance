//! Inversions and classical cadence machinery (research §2C):
//!
//! - `Degree`/`Chord` inversion support (figured-bass display, slash
//!   bass projection);
//! - the generator's inversion decoration pass (ii6 before V,
//!   IV-precedes-ii ordering, cadential 6/4 in the cadence slot);
//! - the SATB pass resolving the cadential 6/4's 6th and 4th down by
//!   step in the same voices over a stationary dominant bass.

use resonance_music_theory::generator::degree::Degree;
use resonance_music_theory::generator::table::TableRegistry;
use resonance_music_theory::generator::{
    GenContext, GeneratedMaterial, Generator, GeneratorSpec, HarmonicFunction,
};
use resonance_music_theory::{satb_voicings, Chord, ChordQuality, Mode, PitchClass, Scale};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn generate(table_id: &str, order: u8, length: u8, seed: u64) -> GeneratedMaterial {
    let reg = TableRegistry::with_builtins();
    let locked = vec![None; length as usize];
    let ctx = GenContext {
        registry: &reg,
        locked: &locked,
    };
    GeneratorSpec::MarkovProgression {
        length,
        table_id: table_id.to_string(),
        order,
        start: None,
        end: None,
    }
    .generate(seed, &ctx)
    .unwrap()
}

fn c_major() -> Scale {
    Scale::new(PitchClass::C, Mode::Major)
}

// ---------------------------------------------------------------------------
// 1. Model: Degree / Chord inversion support
// ---------------------------------------------------------------------------

#[test]
fn inversion_projects_as_slash_bass() {
    // ii6 in C major: Dm/F.
    let ii6 = Degree::II_MIN.with_inversion(1);
    let chord = ii6.to_chord(c_major());
    assert_eq!(chord.root, PitchClass::D);
    assert_eq!(chord.bass, Some(PitchClass::F));

    // Cadential 6/4 in C major: C/G.
    let i64 = Degree::I.with_inversion(2);
    let chord = i64.to_chord(c_major());
    assert_eq!(chord.root, PitchClass::C);
    assert_eq!(chord.bass, Some(PitchClass::G));

    // Third inversion of V7 in C major: G7/F.
    let v42 = Degree::V_DOM7.with_inversion(3);
    let chord = v42.to_chord(c_major());
    assert_eq!(chord.root, PitchClass::G);
    assert_eq!(chord.bass, Some(PitchClass::F));
}

#[test]
fn chord_inverted_bounds() {
    let c = Chord::new(PitchClass::C, ChordQuality::Maj);
    // Root position and out-of-range inversions leave the chord alone.
    assert_eq!(c.inverted(0), c);
    assert_eq!(c.inverted(3), c); // triad has no 3rd inversion
    assert_eq!(c.inverted(9), c);
    // First/second inversions set the slash bass.
    assert_eq!(c.inverted(1).bass, Some(PitchClass::E));
    assert_eq!(c.inverted(2).bass, Some(PitchClass::G));
}

#[test]
fn figured_bass_display() {
    assert_eq!(Degree::II_MIN.with_inversion(1).to_string(), "ii6");
    assert_eq!(Degree::I.with_inversion(2).to_string(), "I64");
    assert_eq!(Degree::I_MIN.with_inversion(2).to_string(), "i64");
    assert_eq!(Degree::V_DOM7.with_inversion(1).to_string(), "V65");
    assert_eq!(Degree::V_DOM7.with_inversion(2).to_string(), "V43");
    assert_eq!(Degree::V_DOM7.with_inversion(3).to_string(), "V42");
    assert_eq!(Degree::VII_DIM.with_inversion(1).to_string(), "vii\u{b0}6");
    // Root position is unchanged by the figured-bass machinery.
    assert_eq!(Degree::V.to_string(), "V");
    assert_eq!(Degree::V_DOM7.to_string(), "V7");
    assert_eq!(Degree::I_MAJ7.to_string(), "I\u{0394}7");
}

#[test]
fn cadential_six_four_is_dominant_function() {
    let reg = TableRegistry::with_builtins();
    let table = reg.get("pop").unwrap();
    assert_eq!(
        table.function_of(Degree::I.with_inversion(2)),
        HarmonicFunction::Dominant,
        "the cadential 6/4 decorates V"
    );
    // Other inversions inherit the root-position tag.
    assert_eq!(
        table.function_of(Degree::II_MIN.with_inversion(1)),
        HarmonicFunction::Predominant
    );
    assert_eq!(
        table.function_of(Degree::I.with_inversion(1)),
        HarmonicFunction::Tonic
    );
}

#[test]
fn degree_json_without_inversion_still_parses() {
    let old = r#"{ "root": 2, "flat": false, "quality": "Min" }"#;
    let parsed: Degree = serde_json::from_str(old).unwrap();
    assert_eq!(parsed, Degree::II_MIN);
    assert_eq!(parsed.inversion, 0);
}

// ---------------------------------------------------------------------------
// 2. Generator: decoration pass
// ---------------------------------------------------------------------------

/// The cadential 6/4 appears on phrase-final dominant slots, and only
/// in its well-formed shape: tonic 6/4 on the front half, the original
/// root-position V on the back half.
#[test]
fn cadential_six_four_decorates_cadence_slots() {
    let mut seen = false;
    for seed in 0..60u64 {
        let mat = generate("classical", 2, 8, seed);
        for chord in &mat.chords {
            if chord.degree.is_cadential_six_four() {
                seen = true;
            }
        }
        for split in &mat.splits {
            let slot = split.slot as usize;
            let front = mat.chords[slot].degree;
            if front.is_cadential_six_four() {
                assert_eq!(slot % 4, 3, "seed {seed}: 6/4 only on cadence slots");
                assert_eq!(split.degree, Degree::V, "seed {seed}: 6/4 resolves to V");
            } else {
                // Any inverted chord outside a 6/4 split is a ii6.
                assert_eq!(front.inversion, 0, "seed {seed}: unexpected inversion");
            }
        }
    }
    assert!(seen, "no cadential 6/4 in 60 seeds of classical output");
}

/// ii6 appears before V (bass 4→5), and every first-inversion
/// supertonic is directly followed by a root-position V — within the
/// slot when split, across slots otherwise.
#[test]
fn ii6_walks_the_bass_into_the_dominant() {
    let mut seen = false;
    for seed in 0..60u64 {
        let mat = generate("classical", 2, 8, seed);
        for (i, chord) in mat.chords.iter().enumerate() {
            let d = chord.degree;
            if d.root == 2 && d.inversion == 1 {
                seen = true;
                // The next sounding chord must be the dominant; a slot
                // split's back half would sound first, but ii6 is only
                // applied to unsplit slots, so chords[i + 1] is next.
                assert!(
                    !mat.splits.iter().any(|s| s.slot as usize == i),
                    "seed {seed}: slot-level ii6 must not be split"
                );
                let next = mat.chords[i + 1].degree;
                assert!(
                    next.root_position().root == 5 || next.is_cadential_six_four(),
                    "seed {seed}: ii6 must precede the dominant (bass on 5), got {next:?}"
                );
            }
        }
        for split in &mat.splits {
            let d = split.degree;
            if d.root == 2 && d.inversion == 1 {
                seen = true;
                let next = mat.chords[split.slot as usize + 1].degree;
                assert!(
                    next.root_position().root == 5 || next.is_cadential_six_four(),
                    "seed {seed}: split-back-half ii6 must precede the dominant \
                     (bass on 5), got {next:?}"
                );
            }
        }
    }
    assert!(seen, "no ii6 in 60 seeds of classical output");
}

/// Inversions never appear anywhere else: only ii6 and the cadential
/// 6/4 forms are in the decoration vocabulary.
#[test]
fn inversion_vocabulary_is_closed() {
    for table_id in ["pop", "classical", "jazz", "metal", "modal"] {
        let order = if table_id == "jazz" || table_id == "classical" {
            2
        } else {
            1
        };
        for seed in 0..40u64 {
            let mat = generate(table_id, order, 8, seed);
            let degrees = mat
                .chords
                .iter()
                .map(|c| c.degree)
                .chain(mat.splits.iter().map(|s| s.degree));
            for d in degrees {
                assert!(
                    d.inversion == 0
                        || (d.root == 2 && d.inversion == 1)
                        || d.is_cadential_six_four(),
                    "{table_id} seed {seed}: unexpected inversion {d:?}"
                );
            }
        }
    }
}

/// Locked slots are never decorated, even on cadence slots.
#[test]
fn locked_dominant_slot_is_never_decorated() {
    let reg = TableRegistry::with_builtins();
    let mut locked: Vec<Option<Degree>> = vec![None; 8];
    locked[3] = Some(Degree::V);
    let ctx = GenContext {
        registry: &reg,
        locked: &locked,
    };
    let spec = GeneratorSpec::MarkovProgression {
        length: 8,
        table_id: "classical".to_string(),
        order: 2,
        start: None,
        end: None,
    };
    for seed in 0..40u64 {
        let mat = spec.generate(seed, &ctx).unwrap();
        assert_eq!(mat.chords[3].degree, Degree::V, "seed {seed}");
        assert!(
            !mat.splits.iter().any(|s| s.slot == 3),
            "seed {seed}: locked dominant slot must not gain a 6/4 split"
        );
    }
}

/// The decoration pass draws from the same seeded RNG as the walk:
/// identical inputs produce identical decorated output.
#[test]
fn decorated_output_is_deterministic() {
    for table_id in ["pop", "classical", "metal"] {
        let order = if table_id == "classical" { 2 } else { 1 };
        let a = generate(table_id, order, 12, 4242);
        let b = generate(table_id, order, 12, 4242);
        assert_eq!(a, b, "determinism failed for {table_id}");
    }
}

/// Decorated material round-trips through JSON (inversions persist).
#[test]
fn decorated_material_roundtrips_json() {
    for seed in 0..60u64 {
        let mat = generate("classical", 2, 8, seed);
        let has_inversion = mat
            .chords
            .iter()
            .map(|c| c.degree)
            .chain(mat.splits.iter().map(|s| s.degree))
            .any(|d| d.inversion != 0);
        if !has_inversion {
            continue;
        }
        let json = serde_json::to_string(&mat).unwrap();
        let parsed: GeneratedMaterial = serde_json::from_str(&json).unwrap();
        assert_eq!(mat, parsed);
        return;
    }
    panic!("no decorated material found to round-trip");
}

// ---------------------------------------------------------------------------
// 3. SATB: cadential 6/4 voice-leading
// ---------------------------------------------------------------------------

/// I6/4 → V over a stationary dominant bass: the bass holds, every
/// upper voice on the 4th above the bass (the tonic) falls a semitone
/// to the leading tone, and every upper voice on the 6th above the
/// bass (scale degree 3) falls a step to degree 2 — the 6→5 / 4→3
/// resolutions in the same voices.
#[test]
fn cadential_six_four_resolves_in_same_voices() {
    let scale = c_major();
    let i64 = Chord::new(PitchClass::C, ChordQuality::Maj).with_bass(PitchClass::G);
    let v = Chord::new(PitchClass::G, ChordQuality::Maj);
    let i = Chord::new(PitchClass::C, ChordQuality::Maj);
    let voicings = satb_voicings(&[i64, v, i], Some(scale), (40, 76));

    let (prev, next) = (&voicings[0], &voicings[1]);
    assert_eq!(
        prev[0] % 12,
        PitchClass::G.to_semitone(),
        "6/4 voiced over the dominant bass"
    );
    assert_eq!(prev[0], next[0], "the dominant bass holds through 6/4 → V");

    for (&p, &n) in prev[1..].iter().zip(next[1..].iter()) {
        match p % 12 {
            // 4th above the bass (C) falls to the 3rd (B).
            0 => assert_eq!(n, p - 1, "4→3 must resolve in the same voice"),
            // 6th above the bass (E) falls to the 5th (D).
            4 => assert_eq!(n, p - 2, "6→5 must resolve in the same voice"),
            _ => {}
        }
    }
}

/// The pre-dominant bass idiom end-to-end through the SATB pass: with
/// IV → ii6 → V → I the bass holds scale degree 4 across IV → ii6 and
/// then steps up to 5 (the rising 4→5 the contrary-motion rule wants).
#[test]
fn ii6_holds_the_predominant_bass() {
    let scale = c_major();
    let iv = Chord::new(PitchClass::F, ChordQuality::Maj);
    let ii6 = Chord::new(PitchClass::D, ChordQuality::Min).with_bass(PitchClass::F);
    let v = Chord::new(PitchClass::G, ChordQuality::Maj);
    let i = Chord::new(PitchClass::C, ChordQuality::Maj);
    let voicings = satb_voicings(&[iv, ii6, v, i], Some(scale), (40, 76));

    let basses: Vec<u8> = voicings.iter().map(|v| v[0]).collect();
    assert_eq!(basses[0] % 12, PitchClass::F.to_semitone());
    assert_eq!(basses[0], basses[1], "bass holds 4 across IV → ii6");
    assert_eq!(basses[2] % 12, PitchClass::G.to_semitone());
    assert!(basses[2] > basses[1], "bass rises 4 → 5 into the cadence");
}
