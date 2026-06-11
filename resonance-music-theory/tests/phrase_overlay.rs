//! Tests for the phrase-model overlay on the Markov chord generator:
//! T→PD→D arc per 4-slot phrase, harmonic-rhythm acceleration via
//! `SplitChord`s, cadential-dominant placement, and backward-compatible
//! serialization.

use std::collections::BTreeMap;

use resonance_music_theory::generator::degree::Degree;
use resonance_music_theory::generator::table::{MarkovTable, TableRegistry};
use resonance_music_theory::generator::{
    GenContext, GenerateError, GeneratedMaterial, Generator, GeneratorSpec, HarmonicFunction,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const ALL_TABLES: [(&str, u8); 6] = [
    ("pop", 1),
    ("modal", 1),
    ("post-rock", 1),
    ("metal", 1),
    ("jazz", 2),
    ("classical", 2),
];

fn spec(table_id: &str, order: u8, length: u8) -> GeneratorSpec {
    GeneratorSpec::MarkovProgression {
        length,
        table_id: table_id.to_string(),
        order,
        start: None,
        end: None,
    }
}

fn generate(
    spec: &GeneratorSpec,
    seed: u64,
    locked: &[Option<Degree>],
) -> Result<GeneratedMaterial, GenerateError> {
    let reg = TableRegistry::with_builtins();
    let ctx = GenContext {
        registry: &reg,
        locked,
    };
    spec.generate(seed, &ctx)
}

/// Function level of a degree per a builtin table's tagging:
/// T = 0, PD = 1, D = 2.
fn level(table_id: &str, degree: Degree) -> u8 {
    let reg = TableRegistry::with_builtins();
    match reg.get(table_id).unwrap().function_of(degree) {
        HarmonicFunction::Tonic => 0,
        HarmonicFunction::Predominant => 1,
        HarmonicFunction::Dominant => 2,
    }
}

// ---------------------------------------------------------------------------
// 1. Arc structure: every 4-slot phrase traverses T → PD → D exactly once
// ---------------------------------------------------------------------------

/// Slot-position invariants for unconstrained generation, across all
/// builtin tables: phrase openings (slots 0, 4, 8, ...) are tonic
/// function, penultimate phrase slots are predominant, phrase-final
/// slots carry the cadential dominant, and the free middle slot never
/// jumps ahead to dominant.
#[test]
fn arc_structure_per_phrase_all_tables() {
    for (table_id, order) in ALL_TABLES {
        let spec = spec(table_id, order, 12);
        let locked = vec![None; 12];
        for seed in 0..40u64 {
            let mat = generate(&spec, seed, &locked).unwrap();
            for (i, chord) in mat.chords.iter().enumerate() {
                let l = level(table_id, chord.degree);
                match i % 4 {
                    0 => assert_eq!(l, 0, "{table_id} seed {seed}: slot {i} must be tonic"),
                    1 => assert!(l <= 1, "{table_id} seed {seed}: slot {i} must be T or PD"),
                    2 => assert_eq!(l, 1, "{table_id} seed {seed}: slot {i} must be predominant"),
                    _ => assert_eq!(l, 2, "{table_id} seed {seed}: slot {i} must be dominant"),
                }
            }
        }
    }
}

/// Function levels are monotone non-decreasing within each phrase: no
/// premature D→T resolution and no T/PD ping-pong mid-phrase.
#[test]
fn no_mid_phrase_regression() {
    for (table_id, order) in ALL_TABLES {
        let spec = spec(table_id, order, 16);
        let locked = vec![None; 16];
        for seed in 0..40u64 {
            let mat = generate(&spec, seed, &locked).unwrap();
            for phrase in mat.chords.chunks(4) {
                let levels: Vec<u8> = phrase
                    .iter()
                    .map(|c| level(table_id, c.degree))
                    .collect();
                assert!(
                    levels.windows(2).all(|w| w[0] <= w[1]),
                    "{table_id} seed {seed}: function regression within phrase: {levels:?}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Harmonic-rhythm acceleration into bars 4/8
// ---------------------------------------------------------------------------

/// Every full phrase splits its forced-PD slot (slot 2 of the 4-bar
/// group — the bar before the cadence) into two distinct predominant
/// chords, doubling the harmonic rhythm into bars 4/8. Dominant slots
/// (3, 7) may additionally split into a cadential 6/4 + V.
#[test]
fn acceleration_splits_into_bars_4_and_8() {
    let spec = spec("pop", 1, 8);
    let locked = vec![None; 8];
    for seed in 0..50u64 {
        let mat = generate(&spec, seed, &locked).unwrap();
        let pd_slots: Vec<usize> = mat
            .splits
            .iter()
            .map(|s| s.slot as usize)
            .filter(|&s| s % 4 == 2)
            .collect();
        assert_eq!(
            pd_slots,
            vec![2, 6],
            "seed {seed}: expected splits on the penultimate slot of each phrase"
        );
        for split in &mat.splits {
            let slot = split.slot as usize;
            let front = mat.chords[slot].degree;
            assert_ne!(
                split.degree, front,
                "seed {seed}: split back half must differ from the front half"
            );
            if slot % 4 == 2 {
                assert_eq!(
                    level("pop", split.degree),
                    1,
                    "seed {seed}: split back half must stay predominant"
                );
            } else {
                // The only other splits allowed are cadential 6/4
                // decorations of the phrase-final dominant.
                assert_eq!(slot % 4, 3, "seed {seed}: unexpected split slot {slot}");
                assert!(
                    front.is_cadential_six_four(),
                    "seed {seed}: dominant-slot split front must be the cadential 6/4"
                );
                assert_eq!(
                    level("pop", split.degree),
                    2,
                    "seed {seed}: cadential 6/4 back half must be the dominant"
                );
            }
        }
    }
}

/// Remainder phrases shorter than four slots never accelerate their
/// predominant. (Cadential 6/4 decorations of the short phrase's final
/// dominant are allowed — they decorate one slot, not crowd it.)
#[test]
fn short_phrases_do_not_split() {
    let spec = spec("pop", 1, 6);
    let locked = vec![None; 6];
    for seed in 0..20u64 {
        let mat = generate(&spec, seed, &locked).unwrap();
        assert!(
            mat.splits
                .iter()
                .all(|s| (s.slot as usize) < 4
                    || mat.chords[s.slot as usize].degree.is_cadential_six_four()),
            "seed {seed}: the trailing 2-slot phrase must not accelerate"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. Cadential dominants on hyper-strong positions
// ---------------------------------------------------------------------------

/// The cadential dominant owns the final slot of each 4-slot group: it
/// is dominant-function on that bar's downbeat and resolves onto the
/// next group's hyper-downbeat tonic. The only split allowed on the
/// slot is the cadential 6/4 decoration — a dominant-function tonic
/// 6/4 on the downbeat resolving to V on the weak half.
#[test]
fn cadential_dominant_placement() {
    let spec = spec("pop", 1, 16);
    let locked = vec![None; 16];
    for seed in 0..30u64 {
        let mat = generate(&spec, seed, &locked).unwrap();
        for (i, chord) in mat.chords.iter().enumerate() {
            if i % 4 == 3 {
                assert_eq!(level("pop", chord.degree), 2, "seed {seed} slot {i}");
                if let Some(split) = mat.splits.iter().find(|s| s.slot as usize == i) {
                    assert!(
                        chord.degree.is_cadential_six_four(),
                        "seed {seed}: a dominant-slot split must be the cadential 6/4"
                    );
                    assert_eq!(
                        level("pop", split.degree),
                        2,
                        "seed {seed}: the 6/4 must resolve to the dominant in-slot"
                    );
                }
                if i + 1 < mat.chords.len() {
                    assert_eq!(
                        level("pop", mat.chords[i + 1].degree),
                        0,
                        "seed {seed}: resolution tonic on the next hyper-downbeat"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 4. Minor-mode function mapping (metal table)
// ---------------------------------------------------------------------------

/// The metal table's minor-mode tags route the arc through i/III → iv/VI
/// → V/VII rather than the major-key heuristic (which would call VI a
/// tonic substitute).
#[test]
fn minor_table_uses_minor_mode_mapping() {
    let spec = spec("metal", 1, 8);
    let locked = vec![None; 8];
    for seed in 0..40u64 {
        let mat = generate(&spec, seed, &locked).unwrap();
        for (i, chord) in mat.chords.iter().enumerate() {
            let d = chord.degree;
            match i % 4 {
                0 => assert!(
                    d == Degree::I_MIN || d == Degree::III_MAJ,
                    "seed {seed} slot {i}: tonic-function opening, got {d}"
                ),
                2 => assert!(
                    d == Degree::IV_MIN || d == Degree::VI_MAJ,
                    "seed {seed} slot {i}: predominant (iv or VI), got {d}"
                ),
                3 => assert!(
                    d == Degree::V
                        || d == Degree::VII_MAJ
                        || (d.is_cadential_six_four()
                            && mat
                                .splits
                                .iter()
                                .any(|s| s.slot as usize == i && s.degree == Degree::V)),
                    "seed {seed} slot {i}: dominant (V, subtonic VII, or i64+V), got {d}"
                ),
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Constraints win over the overlay
// ---------------------------------------------------------------------------

/// A fixed tonic ending (`end: I`) shifts the cadence left instead of
/// fighting the constraint: ... PD D | I.
#[test]
fn end_tonic_shifts_cadence_left() {
    let spec = GeneratorSpec::MarkovProgression {
        length: 8,
        table_id: "pop".to_string(),
        order: 1,
        start: None,
        end: Some(Degree::I),
    };
    let locked = vec![None; 8];
    for seed in 0..40u64 {
        let mat = generate(&spec, seed, &locked).unwrap();
        assert_eq!(mat.chords[7].degree, Degree::I, "seed {seed}");
        assert_eq!(
            level("pop", mat.chords[6].degree),
            2,
            "seed {seed}: dominant shifted to the slot before the fixed tonic"
        );
        assert_eq!(
            level("pop", mat.chords[5].degree),
            1,
            "seed {seed}: predominant prepares the shifted cadence"
        );
    }
}

/// Locked slots keep their degree, are never masked, and never split —
/// even when the lock contradicts the slot's planned function.
#[test]
fn locked_slot_wins_and_never_splits() {
    let spec = spec("pop", 1, 8);
    let mut locked = vec![None; 8];
    locked[2] = Some(Degree::V); // a dominant on the planned-PD split slot

    for seed in 0..20u64 {
        let mat = generate(&spec, seed, &locked).unwrap();
        assert_eq!(mat.chords[2].degree, Degree::V, "seed {seed}");
        assert!(mat.chords[2].locked, "seed {seed}");
        assert!(
            !mat.splits.iter().any(|s| s.slot == 2),
            "seed {seed}: locked slots must never split"
        );
    }
}

/// The start constraint still pins slot 0 even though it conflicts with
/// the tonic opening the overlay would otherwise enforce.
#[test]
fn start_constraint_overrides_tonic_opening() {
    let spec = GeneratorSpec::MarkovProgression {
        length: 8,
        table_id: "pop".to_string(),
        order: 1,
        start: Some(Degree::V),
        end: None,
    };
    let locked = vec![None; 8];
    for seed in 0..20u64 {
        let mat = generate(&spec, seed, &locked).unwrap();
        assert_eq!(mat.chords[0].degree, Degree::V, "seed {seed}");
    }
}

// ---------------------------------------------------------------------------
// 6. Determinism
// ---------------------------------------------------------------------------

/// Identical inputs produce identical output, including the splits.
#[test]
fn determinism_includes_splits() {
    for (table_id, order) in ALL_TABLES {
        let spec = spec(table_id, order, 8);
        let locked = vec![None; 8];
        let a = generate(&spec, 7777, &locked).unwrap();
        let b = generate(&spec, 7777, &locked).unwrap();
        assert_eq!(a, b, "determinism failed for table {table_id}");
        assert_eq!(a.splits, b.splits, "split determinism failed for {table_id}");
    }
}

// ---------------------------------------------------------------------------
// 7. Serialization backward compatibility
// ---------------------------------------------------------------------------

/// A `GeneratorSpec::MarkovProgression` serialized before the overlay
/// existed (no new fields were added) still deserializes and generates.
#[test]
fn old_markov_spec_json_still_parses() {
    let old_json = r#"{
        "type": "MarkovProgression",
        "length": 8,
        "table_id": "pop",
        "order": 1
    }"#;
    let spec: GeneratorSpec = serde_json::from_str(old_json).unwrap();
    let locked = vec![None; 8];
    let mat = generate(&spec, 42, &locked).unwrap();
    assert_eq!(mat.chords.len(), 8);
}

/// `GeneratedMaterial` persisted before the `splits` field existed
/// deserializes with no splits.
#[test]
fn old_generated_material_json_still_parses() {
    let old_json = r#"{
        "chords": [
            { "degree": { "root": 1, "flat": false, "quality": "Maj" }, "locked": false },
            { "degree": { "root": 5, "flat": false, "quality": "Maj" }, "locked": true }
        ]
    }"#;
    let mat: GeneratedMaterial = serde_json::from_str(old_json).unwrap();
    assert_eq!(mat.chords.len(), 2);
    assert!(mat.splits.is_empty());
    assert_eq!(mat.chords[1].degree, Degree::V);
}

/// New material (with splits) round-trips through JSON.
#[test]
fn material_with_splits_roundtrips() {
    let spec = spec("pop", 1, 8);
    let locked = vec![None; 8];
    let mat = generate(&spec, 42, &locked).unwrap();
    assert!(!mat.splits.is_empty(), "expected splits in pop length-8 output");
    let json = serde_json::to_string(&mat).unwrap();
    let parsed: GeneratedMaterial = serde_json::from_str(&json).unwrap();
    assert_eq!(mat, parsed);
}

// ---------------------------------------------------------------------------
// 8. Untagged tables fall back to the root heuristic
// ---------------------------------------------------------------------------

/// A user-registered table without explicit tags classifies degrees via
/// the default root-based mapping (1/3/6 = T, 2/4 = PD, 5/7 = D; bVII =
/// D, bVI = PD), and degenerate single-function tables still generate.
#[test]
fn untagged_table_falls_back_to_heuristic() {
    let mut transitions: BTreeMap<Vec<Degree>, Vec<(Degree, f32)>> = BTreeMap::new();
    transitions.insert(vec![Degree::I], vec![(Degree::I, 1.0)]);
    let table = MarkovTable {
        id: "single".to_string(),
        order: 1,
        transitions,
        functions: BTreeMap::new(),
    };
    assert_eq!(table.function_of(Degree::I), HarmonicFunction::Tonic);
    assert_eq!(table.function_of(Degree::IV), HarmonicFunction::Predominant);
    assert_eq!(table.function_of(Degree::V), HarmonicFunction::Dominant);
    assert_eq!(
        table.function_of(Degree::FLAT_VII),
        HarmonicFunction::Dominant
    );
    assert_eq!(
        table.function_of(Degree::FLAT_VI),
        HarmonicFunction::Predominant
    );

    // A tonic-only table can't satisfy the arc; the overlay must degrade
    // gracefully instead of erroring or panicking.
    let mut reg = TableRegistry::new();
    reg.register(table);
    let spec = GeneratorSpec::MarkovProgression {
        length: 8,
        table_id: "single".to_string(),
        order: 1,
        start: None,
        end: None,
    };
    let locked = vec![None; 8];
    let ctx = GenContext {
        registry: &reg,
        locked: &locked,
    };
    let mat = spec.generate(42, &ctx).unwrap();
    assert_eq!(mat.chords.len(), 8);
    assert!(mat.splits.is_empty(), "no second predominant exists to split with");
}
