//! Tests for the pop schema bank generator (`GeneratorSpec::Schema`).

use std::collections::HashSet;

use resonance_music_theory::generator::degree::Degree;
use resonance_music_theory::generator::schema::{SchemaKind, MIN_SHARED_TONES};
use resonance_music_theory::generator::table::TableRegistry;
use resonance_music_theory::generator::{
    GenContext, GenerateError, GeneratedMaterial, Generator, GeneratorSpec,
};
use resonance_music_theory::{ChordQuality, Mode, PitchClass, Scale};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn schema_spec(schema: SchemaKind, length: u8, rotation: u8, substitution: f32) -> GeneratorSpec {
    GeneratorSpec::Schema {
        schema,
        length,
        rotation,
        substitution,
    }
}

fn no_locks(len: usize) -> Vec<Option<Degree>> {
    vec![None; len]
}

fn generate_with(
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

fn degrees_of(mat: &GeneratedMaterial) -> Vec<Degree> {
    mat.chords.iter().map(|c| c.degree).collect()
}

/// Pitch classes of a degree projected through C in the given mode.
fn pitch_classes(degree: Degree, mode: Mode) -> HashSet<PitchClass> {
    degree
        .to_chord(Scale::new(PitchClass::C, mode))
        .pitch_classes()
        .collect()
}

const II_MAJ: Degree = Degree {
    root: 2,
    flat: false,
    quality: ChordQuality::Maj,
};

// ---------------------------------------------------------------------------
// 1. Canonical degrees: rotation 0, substitution 0, one full pass
// ---------------------------------------------------------------------------

#[test]
fn canonical_twelve_bar_blues_layout() {
    use Degree as D;
    let spec = schema_spec(SchemaKind::TwelveBarBlues, 12, 0, 0.0);
    let mat = generate_with(&spec, 42, &no_locks(12)).unwrap();
    // Bars 1-4: tonic. Bars 5-6: subdominant. Bars 7-8: tonic.
    // Bar 9: dominant. Bar 10: subdominant. Bars 11-12: tonic.
    assert_eq!(
        degrees_of(&mat),
        vec![
            D::I,
            D::I,
            D::I,
            D::I,
            D::IV,
            D::IV,
            D::I,
            D::I,
            D::V,
            D::IV,
            D::I,
            D::I,
        ]
    );
}

#[test]
fn canonical_degrees_all_schemas() {
    use Degree as D;
    let cases: Vec<(SchemaKind, Vec<Degree>)> = vec![
        (SchemaKind::DooWop, vec![D::I, D::VI_MIN, D::IV, D::V]),
        (SchemaKind::Axis, vec![D::I, D::V, D::VI_MIN, D::IV]),
        (SchemaKind::Hopscotch, vec![D::IV, D::V, D::VI_MIN, D::I]),
        (
            SchemaKind::Lament,
            vec![D::I_MIN, D::VII_MAJ, D::VI_MAJ, D::V],
        ),
        (SchemaKind::PlagalVamp, vec![D::I, D::IV]),
        (SchemaKind::DoublePlagal, vec![D::FLAT_VII, D::IV, D::I]),
        (SchemaKind::PlagalSigh, vec![D::IV, D::IV_MIN, D::I]),
        (SchemaKind::MixolydianShuttle, vec![D::I, D::FLAT_VII]),
        (SchemaKind::DorianShuttle, vec![D::I_MIN, D::IV]),
        (SchemaKind::LydianShuttle, vec![D::I, II_MAJ]),
        (
            SchemaKind::CircleOfFifths,
            vec![
                D::I,
                D::IV,
                D::VII_DIM,
                D::III_MIN,
                D::VI_MIN,
                D::II_MIN,
                D::V,
                D::I,
            ],
        ),
        (SchemaKind::Puff, vec![D::I, D::III_MIN, D::IV, D::I]),
    ];
    for (kind, expected) in cases {
        let spec = schema_spec(kind, kind.default_length(), 0, 0.0);
        let mat = generate_with(&spec, 7, &no_locks(expected.len())).unwrap();
        assert_eq!(degrees_of(&mat), expected, "schema {kind:?}");
        assert!(
            mat.chords.iter().all(|c| !c.locked),
            "schema {kind:?}: nothing should be locked"
        );
    }
}

#[test]
fn circle_of_fifths_root_motion_descends_in_fifths() {
    // Each adjacent pair in the canonical loop is a descending diatonic
    // fifth: projected to C major, root motion is up 5 semitones (perfect
    // fourth = inverted perfect fifth) or 6 for the one diatonic tritone
    // step (IV -> vii°, F -> B in C major).
    let spec = schema_spec(SchemaKind::CircleOfFifths, 8, 0, 0.0);
    let mat = generate_with(&spec, 1, &no_locks(8)).unwrap();
    let scale = Scale::new(PitchClass::C, Mode::Major);
    let roots: Vec<u8> = mat
        .chords
        .iter()
        .map(|c| c.degree.to_chord(scale).root.to_semitone())
        .collect();
    for pair in roots.windows(2) {
        let motion = (i32::from(pair[1]) - i32::from(pair[0])).rem_euclid(12);
        assert!(
            motion == 5 || motion == 6,
            "expected descending-fifth motion, got {roots:?}"
        );
    }
    // Exactly one tritone step (the diatonic IV -> vii°).
    let tritones = roots
        .windows(2)
        .filter(|p| (i32::from(p[1]) - i32::from(p[0])).rem_euclid(12) == 6)
        .count();
    assert_eq!(tritones, 1, "got {roots:?}");
}

// ---------------------------------------------------------------------------
// 2. Rotation
// ---------------------------------------------------------------------------

#[test]
fn axis_rotations() {
    use Degree as D;
    let base = [D::I, D::V, D::VI_MIN, D::IV];
    for rotation in 0..8u8 {
        let spec = schema_spec(SchemaKind::Axis, 4, rotation, 0.0);
        let mat = generate_with(&spec, 0, &no_locks(4)).unwrap();
        let expected: Vec<Degree> = (0..4)
            .map(|i| base[(i + rotation as usize) % 4])
            .collect();
        assert_eq!(degrees_of(&mat), expected, "rotation {rotation}");
    }
}

#[test]
fn rotation_wraps_modulo_loop_length() {
    let a = generate_with(&schema_spec(SchemaKind::DooWop, 4, 1, 0.0), 5, &no_locks(4)).unwrap();
    let b = generate_with(&schema_spec(SchemaKind::DooWop, 4, 5, 0.0), 5, &no_locks(4)).unwrap();
    assert_eq!(a, b);
}

// ---------------------------------------------------------------------------
// 3. Tiling: lengths longer / shorter than the loop
// ---------------------------------------------------------------------------

#[test]
fn loop_tiles_to_requested_length() {
    use Degree as D;
    let spec = schema_spec(SchemaKind::DooWop, 10, 0, 0.0);
    let mat = generate_with(&spec, 9, &no_locks(10)).unwrap();
    let one_pass = [D::I, D::VI_MIN, D::IV, D::V];
    let expected: Vec<Degree> = (0..10).map(|i| one_pass[i % 4]).collect();
    assert_eq!(degrees_of(&mat), expected);
}

#[test]
fn length_zero_returns_empty() {
    let spec = schema_spec(SchemaKind::Axis, 0, 0, 1.0);
    let mat = generate_with(&spec, 42, &no_locks(0)).unwrap();
    assert!(mat.chords.is_empty());
}

// ---------------------------------------------------------------------------
// 4. Determinism
// ---------------------------------------------------------------------------

#[test]
fn determinism_same_seed_same_output() {
    for kind in SchemaKind::ALL {
        let spec = schema_spec(kind, 16, 1, 0.7);
        let a = generate_with(&spec, 42, &no_locks(16)).unwrap();
        let b = generate_with(&spec, 42, &no_locks(16)).unwrap();
        assert_eq!(a, b, "determinism failed for {kind:?}");
    }
}

#[test]
fn different_seeds_produce_variety_with_substitution() {
    let spec = schema_spec(SchemaKind::Axis, 8, 0, 1.0);
    let mut unique = HashSet::new();
    for seed in 0..100u64 {
        let mat = generate_with(&spec, seed, &no_locks(8)).unwrap();
        unique.insert(degrees_of(&mat));
    }
    assert!(
        unique.len() >= 10,
        "only {} unique progressions from 100 seeds",
        unique.len()
    );
}

#[test]
fn zero_substitution_is_seed_independent() {
    let spec = schema_spec(SchemaKind::Hopscotch, 8, 0, 0.0);
    let a = generate_with(&spec, 1, &no_locks(8)).unwrap();
    let b = generate_with(&spec, 999_999, &no_locks(8)).unwrap();
    assert_eq!(a, b);
}

// ---------------------------------------------------------------------------
// 5. Substitution shares >= MIN_SHARED_TONES pitch classes
// ---------------------------------------------------------------------------

#[test]
fn substitutions_share_at_least_two_tones() {
    for kind in SchemaKind::ALL {
        let base = kind.base_degrees();
        let mode = kind.mode();
        let len = (base.len() * 2) as u8;
        let spec = schema_spec(kind, len, 0, 1.0);
        for seed in 0..200u64 {
            let mat = generate_with(&spec, seed, &no_locks(len as usize)).unwrap();
            for (i, chord) in mat.chords.iter().enumerate() {
                let canonical = base[i % base.len()];
                if chord.degree == canonical {
                    continue;
                }
                let shared = pitch_classes(canonical, mode)
                    .intersection(&pitch_classes(chord.degree, mode))
                    .count();
                assert!(
                    shared >= MIN_SHARED_TONES as usize,
                    "{kind:?} seed {seed} pos {i}: substitute {} shares only {shared} \
                     tone(s) with canonical {}",
                    chord.degree,
                    canonical
                );
            }
        }
    }
}

#[test]
fn full_substitution_actually_substitutes() {
    // With probability 1.0 every position that has a valid substitute
    // must be swapped; over many seeds at least some output must differ
    // from the canonical loop.
    let spec = schema_spec(SchemaKind::DooWop, 4, 0, 1.0);
    let canonical = degrees_of(&generate_with(&schema_spec(SchemaKind::DooWop, 4, 0, 0.0), 0, &no_locks(4)).unwrap());
    let mut any_diff = false;
    for seed in 0..20u64 {
        if degrees_of(&generate_with(&spec, seed, &no_locks(4)).unwrap()) != canonical {
            any_diff = true;
            break;
        }
    }
    assert!(any_diff, "substitution 1.0 never changed the progression");
}

// ---------------------------------------------------------------------------
// 6. Locks
// ---------------------------------------------------------------------------

#[test]
fn locked_chords_preserved_and_never_substituted() {
    let mut locked = no_locks(8);
    locked[2] = Some(Degree::FLAT_VI);
    locked[5] = Some(Degree::II_MIN);

    let spec = schema_spec(SchemaKind::Axis, 8, 0, 1.0);
    for seed in [0u64, 42, 999, 123_456] {
        let mat = generate_with(&spec, seed, &locked).unwrap();
        assert_eq!(mat.chords[2].degree, Degree::FLAT_VI, "seed {seed}");
        assert!(mat.chords[2].locked);
        assert_eq!(mat.chords[5].degree, Degree::II_MIN, "seed {seed}");
        assert!(mat.chords[5].locked);
        for (i, c) in mat.chords.iter().enumerate() {
            if i != 2 && i != 5 {
                assert!(!c.locked, "seed {seed} pos {i}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 7. Serde: round-trip and backward compatibility
// ---------------------------------------------------------------------------

#[test]
fn json_roundtrip_schema_spec() {
    let spec = schema_spec(SchemaKind::Lament, 8, 2, 0.35);
    let json = serde_json::to_string(&spec).unwrap();
    let parsed: GeneratorSpec = serde_json::from_str(&json).unwrap();
    let a = generate_with(&spec, 42, &no_locks(8)).unwrap();
    let b = generate_with(&parsed, 42, &no_locks(8)).unwrap();
    assert_eq!(a, b);
}

#[test]
fn schema_spec_optional_fields_default() {
    // `rotation` and `substitution` are serde-defaulted so minimal JSON
    // (and forward-written files) parse.
    let json = r#"{"type":"Schema","schema":"Axis","length":4}"#;
    let parsed: GeneratorSpec = serde_json::from_str(json).unwrap();
    let mat = generate_with(&parsed, 3, &no_locks(4)).unwrap();
    assert_eq!(
        degrees_of(&mat),
        vec![Degree::I, Degree::V, Degree::VI_MIN, Degree::IV]
    );
}

#[test]
fn existing_markov_json_still_parses() {
    // Old project files persist MarkovProgression specs; adding the
    // Schema variant must not break them (internally tagged enum).
    let json = r#"{"type":"MarkovProgression","length":8,"table_id":"pop","order":1}"#;
    let parsed: GeneratorSpec = serde_json::from_str(json).unwrap();
    let mat = generate_with(&parsed, 42, &no_locks(8)).unwrap();
    assert_eq!(mat.chords.len(), 8);
}

// ---------------------------------------------------------------------------
// 8. SchemaKind surface
// ---------------------------------------------------------------------------

#[test]
fn schema_ids_unique_and_kebab_case() {
    let mut seen = HashSet::new();
    for kind in SchemaKind::ALL {
        assert!(seen.insert(kind.id()), "duplicate id {}", kind.id());
        assert!(
            kind.id()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "id {} is not kebab-case",
            kind.id()
        );
        assert!(!kind.name().is_empty());
        assert_eq!(kind.to_string(), kind.name());
    }
}

#[test]
fn default_length_matches_loop() {
    assert_eq!(SchemaKind::TwelveBarBlues.default_length(), 12);
    assert_eq!(SchemaKind::CircleOfFifths.default_length(), 8);
    assert_eq!(SchemaKind::Axis.default_length(), 4);
    assert_eq!(SchemaKind::PlagalVamp.default_length(), 2);
}

#[test]
fn minor_schemas_report_minor_mode() {
    assert_eq!(SchemaKind::Lament.mode(), Mode::Minor);
    assert_eq!(SchemaKind::DorianShuttle.mode(), Mode::Minor);
    assert_eq!(SchemaKind::Axis.mode(), Mode::Major);
    assert_eq!(SchemaKind::TwelveBarBlues.mode(), Mode::Major);
}
