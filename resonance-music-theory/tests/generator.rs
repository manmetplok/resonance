//! Tests for the generator module.

use std::collections::HashMap;

use resonance_music_theory::generator::degree::Degree;
use resonance_music_theory::generator::table::TableRegistry;
use resonance_music_theory::generator::{
    GenContext, GenerateError, GeneratedMaterial, Generator, GeneratorSpec,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn pop_spec(length: u8) -> GeneratorSpec {
    GeneratorSpec::MarkovProgression {
        length,
        table_id: "pop".to_string(),
        order: 1,
        start: None,
        end: None,
    }
}

fn registry() -> TableRegistry {
    TableRegistry::with_builtins()
}

fn no_locks(len: usize) -> Vec<Option<Degree>> {
    vec![None; len]
}

fn generate_with(
    spec: &GeneratorSpec,
    seed: u64,
    locked: &[Option<Degree>],
) -> Result<GeneratedMaterial, GenerateError> {
    let reg = registry();
    let ctx = GenContext {
        registry: &reg,
        locked,
    };
    spec.generate(seed, &ctx)
}

// ---------------------------------------------------------------------------
// 1. Determinism: same seed + same spec + same table -> identical output
// ---------------------------------------------------------------------------

#[test]
fn determinism_same_seed_same_output() {
    let spec = pop_spec(8);
    let locked = no_locks(8);

    let a = generate_with(&spec, 42, &locked).unwrap();
    let b = generate_with(&spec, 42, &locked).unwrap();
    assert_eq!(a, b);
}

#[test]
fn determinism_across_all_builtin_tables() {
    for table_id in ["pop", "modal", "jazz", "post-rock", "metal", "classical"] {
        let order = if table_id == "jazz" || table_id == "classical" {
            2
        } else {
            1
        };
        let spec = GeneratorSpec::MarkovProgression {
            length: 8,
            table_id: table_id.to_string(),
            order,
            start: None,
            end: None,
        };
        let locked = no_locks(8);
        let a = generate_with(&spec, 1234, &locked).unwrap();
        let b = generate_with(&spec, 1234, &locked).unwrap();
        assert_eq!(a, b, "determinism failed for table {table_id}");
    }
}

// ---------------------------------------------------------------------------
// 2. Different seeds produce different outputs (statistical)
// ---------------------------------------------------------------------------

#[test]
fn different_seeds_produce_variety() {
    let spec = pop_spec(8);
    let locked = no_locks(8);

    let mut unique = std::collections::HashSet::new();
    for seed in 0..100u64 {
        let mat = generate_with(&spec, seed, &locked).unwrap();
        let degrees: Vec<Degree> = mat.chords.iter().map(|c| c.degree).collect();
        unique.insert(degrees);
    }
    // With 100 seeds and 6 possible degrees per position (pop table),
    // we should get a good spread. At minimum 10 unique progressions.
    assert!(
        unique.len() >= 10,
        "only {} unique progressions from 100 seeds",
        unique.len()
    );
}

// ---------------------------------------------------------------------------
// 3. Locked chords are preserved across regeneration
// ---------------------------------------------------------------------------

#[test]
fn locked_chords_preserved() {
    let spec = pop_spec(6);
    let mut locked = no_locks(6);
    locked[1] = Some(Degree::IV);
    locked[3] = Some(Degree::VI_MIN);

    for seed in [0, 42, 999, 123456] {
        let mat = generate_with(&spec, seed, &locked).unwrap();
        assert_eq!(mat.chords.len(), 6);
        assert_eq!(mat.chords[1].degree, Degree::IV);
        assert!(mat.chords[1].locked);
        assert_eq!(mat.chords[3].degree, Degree::VI_MIN);
        assert!(mat.chords[3].locked);
        // Other positions should not be locked.
        assert!(!mat.chords[0].locked);
        assert!(!mat.chords[2].locked);
        assert!(!mat.chords[4].locked);
        assert!(!mat.chords[5].locked);
    }
}

// ---------------------------------------------------------------------------
// 4. Start constraint: first chord equals start when set
// ---------------------------------------------------------------------------

#[test]
fn start_constraint() {
    let spec = GeneratorSpec::MarkovProgression {
        length: 6,
        table_id: "pop".to_string(),
        order: 1,
        start: Some(Degree::V),
        end: None,
    };
    let locked = no_locks(6);

    for seed in 0..50 {
        let mat = generate_with(&spec, seed, &locked).unwrap();
        assert_eq!(
            mat.chords[0].degree,
            Degree::V,
            "seed {seed}: first chord should be V"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. End constraint: last chord equals end when set and reachable
// ---------------------------------------------------------------------------

#[test]
fn end_constraint() {
    let spec = GeneratorSpec::MarkovProgression {
        length: 6,
        table_id: "pop".to_string(),
        order: 1,
        start: None,
        end: Some(Degree::I),
    };
    let locked = no_locks(6);

    for seed in 0..50 {
        let mat = generate_with(&spec, seed, &locked).unwrap();
        assert_eq!(
            mat.chords.last().unwrap().degree,
            Degree::I,
            "seed {seed}: last chord should be I"
        );
    }
}

#[test]
fn start_and_end_constraint() {
    let spec = GeneratorSpec::MarkovProgression {
        length: 8,
        table_id: "pop".to_string(),
        order: 1,
        start: Some(Degree::I),
        end: Some(Degree::V),
    };
    let locked = no_locks(8);

    for seed in 0..20 {
        let mat = generate_with(&spec, seed, &locked).unwrap();
        assert_eq!(mat.chords[0].degree, Degree::I, "seed {seed}");
        assert_eq!(mat.chords.last().unwrap().degree, Degree::V, "seed {seed}");
    }
}

// ---------------------------------------------------------------------------
// 6. End constraint unreachable -> error returned, no panic
// ---------------------------------------------------------------------------

#[test]
fn end_unreachable_returns_error() {
    // Build a tiny table that can only produce degree I.
    // Requesting end = IV is unreachable.
    let mut reg = TableRegistry::new();
    use resonance_music_theory::generator::table::MarkovTable;
    let mut transitions = HashMap::new();
    transitions.insert(vec![Degree::I], vec![(Degree::I, 1.0)]);
    reg.register(MarkovTable {
        id: "single".to_string(),
        order: 1,
        transitions,
    });

    let spec = GeneratorSpec::MarkovProgression {
        length: 4,
        table_id: "single".to_string(),
        order: 1,
        start: None,
        end: Some(Degree::IV),
    };
    let locked = no_locks(4);
    let ctx = GenContext {
        registry: &reg,
        locked: &locked,
    };

    let result = spec.generate(42, &ctx);
    assert!(result.is_err(), "should return error for unreachable end");
    let err = result.unwrap_err();
    assert!(
        matches!(err, GenerateError::EndUnreachable { .. }),
        "expected EndUnreachable, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// 7. Order back-off: second-order table with short requests
// ---------------------------------------------------------------------------

#[test]
fn order_backoff_length_1() {
    let spec = GeneratorSpec::MarkovProgression {
        length: 1,
        table_id: "jazz".to_string(),
        order: 2,
        start: None,
        end: None,
    };
    let locked = no_locks(1);
    let mat = generate_with(&spec, 42, &locked).unwrap();
    assert_eq!(mat.chords.len(), 1);
}

#[test]
fn order_backoff_length_2() {
    let spec = GeneratorSpec::MarkovProgression {
        length: 2,
        table_id: "jazz".to_string(),
        order: 2,
        start: None,
        end: None,
    };
    let locked = no_locks(2);
    let mat = generate_with(&spec, 42, &locked).unwrap();
    assert_eq!(mat.chords.len(), 2);
}

#[test]
fn order_backoff_classical() {
    let spec = GeneratorSpec::MarkovProgression {
        length: 3,
        table_id: "classical".to_string(),
        order: 2,
        start: None,
        end: None,
    };
    let locked = no_locks(3);
    let mat = generate_with(&spec, 42, &locked).unwrap();
    assert_eq!(mat.chords.len(), 3);
}

// ---------------------------------------------------------------------------
// 8. JSON round-trip of GeneratorSpec + GeneratedMaterial
// ---------------------------------------------------------------------------

#[test]
fn json_roundtrip_generator_spec() {
    let spec = GeneratorSpec::MarkovProgression {
        length: 8,
        table_id: "modal".to_string(),
        order: 1,
        start: Some(Degree::I),
        end: Some(Degree::FLAT_VII),
    };
    let json = serde_json::to_string(&spec).unwrap();
    let parsed: GeneratorSpec = serde_json::from_str(&json).unwrap();
    // Roundtrip: regenerate with both and compare.
    let locked = no_locks(8);
    let a = generate_with(&spec, 42, &locked).unwrap();
    let b = generate_with(&parsed, 42, &locked).unwrap();
    assert_eq!(a, b);
}

#[test]
fn json_roundtrip_generated_material() {
    let spec = pop_spec(6);
    let locked = no_locks(6);
    let mat = generate_with(&spec, 42, &locked).unwrap();
    let json = serde_json::to_string(&mat).unwrap();
    let parsed: GeneratedMaterial = serde_json::from_str(&json).unwrap();
    assert_eq!(mat, parsed);
}

// ---------------------------------------------------------------------------
// 9. Table not found -> error
// ---------------------------------------------------------------------------

#[test]
fn table_not_found() {
    let spec = GeneratorSpec::MarkovProgression {
        length: 4,
        table_id: "nonexistent".to_string(),
        order: 1,
        start: None,
        end: None,
    };
    let locked = no_locks(4);
    let result = generate_with(&spec, 42, &locked);
    assert!(matches!(
        result,
        Err(GenerateError::TableNotFound(ref id)) if id == "nonexistent"
    ));
}

// ---------------------------------------------------------------------------
// 10. Distribution sanity check: each built-in table's output roughly
//     matches the table weights over many seeds.
// ---------------------------------------------------------------------------

#[test]
fn distribution_sanity_pop() {
    distribution_check(
        "pop",
        1,
        &[Degree::I, Degree::IV, Degree::V, Degree::VI_MIN],
    );
}

#[test]
fn distribution_sanity_modal() {
    distribution_check(
        "modal",
        1,
        &[Degree::I, Degree::IV, Degree::FLAT_VII, Degree::FLAT_VI],
    );
}

#[test]
fn distribution_sanity_jazz() {
    distribution_check(
        "jazz",
        2,
        &[Degree::I_MAJ7, Degree::II_MIN7, Degree::V_DOM7],
    );
}

#[test]
fn distribution_sanity_post_rock() {
    distribution_check("post-rock", 1, &[Degree::I, Degree::IV, Degree::VI_MIN]);
}

#[test]
fn distribution_sanity_metal() {
    distribution_check(
        "metal",
        1,
        &[Degree::I_MIN, Degree::VI_MAJ, Degree::VII_MAJ],
    );
}

#[test]
fn distribution_sanity_classical() {
    distribution_check(
        "classical",
        2,
        &[Degree::I, Degree::V, Degree::IV, Degree::II_MIN],
    );
}

/// Run 1000 seeds through a table and verify that each of the expected
/// degrees appears at least once in the aggregated output. This is a
/// loose sanity check that sampling isn't stuck on a single degree.
fn distribution_check(table_id: &str, order: u8, expected_present: &[Degree]) {
    let spec = GeneratorSpec::MarkovProgression {
        length: 16,
        table_id: table_id.to_string(),
        order,
        start: None,
        end: None,
    };
    let locked = no_locks(16);

    let mut counts: HashMap<Degree, usize> = HashMap::new();
    for seed in 0..1000u64 {
        let mat = generate_with(&spec, seed, &locked).unwrap();
        for chord in &mat.chords {
            *counts.entry(chord.degree).or_insert(0) += 1;
        }
    }

    for deg in expected_present {
        let count = counts.get(deg).copied().unwrap_or(0);
        assert!(
            count > 0,
            "table {table_id}: degree {deg} never appeared in 1000 seeds ({} total chords)",
            counts.values().sum::<usize>()
        );
    }
}

// ---------------------------------------------------------------------------
// 11. Degree::to_chord conversion
// ---------------------------------------------------------------------------

#[test]
fn degree_to_chord_c_major() {
    use resonance_music_theory::{Chord, ChordQuality, Mode, PitchClass, Scale};
    let scale = Scale::new(PitchClass::C, Mode::Major);
    assert_eq!(
        Degree::I.to_chord(scale),
        Chord::new(PitchClass::C, ChordQuality::Maj)
    );
    assert_eq!(
        Degree::II_MIN.to_chord(scale),
        Chord::new(PitchClass::D, ChordQuality::Min)
    );
    assert_eq!(
        Degree::V.to_chord(scale),
        Chord::new(PitchClass::G, ChordQuality::Maj)
    );
    assert_eq!(
        Degree::FLAT_VII.to_chord(scale),
        Chord::new(PitchClass::As, ChordQuality::Maj)
    );
    assert_eq!(
        Degree::FLAT_VI.to_chord(scale),
        Chord::new(PitchClass::Gs, ChordQuality::Maj)
    );
}

#[test]
fn degree_to_chord_a_minor() {
    use resonance_music_theory::{Chord, ChordQuality, Mode, PitchClass, Scale};
    let scale = Scale::new(PitchClass::A, Mode::Minor);
    // In A natural minor: A B C D E F G
    assert_eq!(
        Degree::I_MIN.to_chord(scale),
        Chord::new(PitchClass::A, ChordQuality::Min)
    );
    assert_eq!(
        Degree::III_MAJ.to_chord(scale),
        Chord::new(PitchClass::C, ChordQuality::Maj)
    );
    assert_eq!(
        Degree::VI_MAJ.to_chord(scale),
        Chord::new(PitchClass::F, ChordQuality::Maj)
    );
    assert_eq!(
        Degree::VII_MAJ.to_chord(scale),
        Chord::new(PitchClass::G, ChordQuality::Maj)
    );
}

// ---------------------------------------------------------------------------
// 12. End constraint conflicts with locked last position
// ---------------------------------------------------------------------------

#[test]
fn end_conflicts_with_lock() {
    let spec = GeneratorSpec::MarkovProgression {
        length: 4,
        table_id: "pop".to_string(),
        order: 1,
        start: None,
        end: Some(Degree::I),
    };
    // Lock the last position to V, conflicting with end = I.
    let mut locked = no_locks(4);
    locked[3] = Some(Degree::V);

    let result = generate_with(&spec, 42, &locked);
    assert!(result.is_err(), "should return error for lock/end conflict");
    let err = result.unwrap_err();
    assert!(
        matches!(err, GenerateError::EndConflictsWithLock),
        "expected EndConflictsWithLock, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// 13. Length 0 returns empty material
// ---------------------------------------------------------------------------

#[test]
fn length_zero() {
    let spec = pop_spec(0);
    let locked = no_locks(0);
    let mat = generate_with(&spec, 42, &locked).unwrap();
    assert!(mat.chords.is_empty());
}

// ---------------------------------------------------------------------------
// 14. Markov degrees projected to minor scales produce correct qualities
// ---------------------------------------------------------------------------

#[test]
fn pop_degrees_projected_to_b_minor_have_correct_quality() {
    use resonance_music_theory::progression::diatonic_chord;
    use resonance_music_theory::{Mode, PitchClass, Scale};

    let scale = Scale::new(PitchClass::B, Mode::Minor);
    let spec = pop_spec(8);
    let locked = no_locks(8);

    // The pop table uses major-key degree constants (I, IV, V, etc.).
    // When projected to a minor scale via diatonic_chord(), the qualities
    // must match the scale: degree 1 in B minor = Bm, not B major.
    for seed in 0..50u64 {
        let mat = generate_with(&spec, seed, &locked).unwrap();
        for gc in &mat.chords {
            let chord = if gc.degree.flat {
                gc.degree.to_chord(scale)
            } else {
                diatonic_chord(scale, gc.degree.root, false)
            };
            let expected = diatonic_chord(scale, gc.degree.root, false);
            assert_eq!(
                chord.quality, expected.quality,
                "seed {seed}: degree {} projected to B minor should be {:?}, got {:?}",
                gc.degree.root, expected, chord
            );
        }
    }
}

#[test]
fn minor_scale_tonic_is_always_minor_across_tables() {
    use resonance_music_theory::progression::diatonic_chord;
    use resonance_music_theory::{Mode, PitchClass, Scale};

    let scale = Scale::new(PitchClass::B, Mode::Minor);

    // For every table, degree 1 projected to a minor scale must be minor.
    for (table_id, _order) in [
        ("pop", 1),
        ("modal", 1),
        ("metal", 1),
        ("post-rock", 1),
        ("jazz", 2),
        ("classical", 2),
    ] {
        let chord = diatonic_chord(scale, 1, false);
        assert_eq!(
            chord.quality,
            resonance_music_theory::ChordQuality::Min,
            "table {table_id}: degree 1 in B minor should be minor"
        );
    }
}

// ---------------------------------------------------------------------------
// 15. Display for Degree
// ---------------------------------------------------------------------------

#[test]
fn degree_display() {
    assert_eq!(Degree::I.to_string(), "I");
    assert_eq!(Degree::II_MIN.to_string(), "ii");
    assert_eq!(Degree::IV.to_string(), "IV");
    assert_eq!(Degree::V.to_string(), "V");
    assert_eq!(Degree::VI_MIN.to_string(), "vi");
    assert_eq!(Degree::VII_DIM.to_string(), "vii\u{b0}");
    assert_eq!(Degree::FLAT_VII.to_string(), "bVII");
    assert_eq!(Degree::FLAT_VI.to_string(), "bVI");
    assert_eq!(Degree::V_DOM7.to_string(), "V7");
    assert_eq!(Degree::II_MIN7.to_string(), "ii7");
}
