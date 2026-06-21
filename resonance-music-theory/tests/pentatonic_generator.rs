//! Tests for the pentatonic harmony generator (`GeneratorSpec::Pentatonic`).

use std::collections::HashSet;

use resonance_music_theory::generator::degree::Degree;
use resonance_music_theory::generator::pentatonic::PentatonicFlavour;
use resonance_music_theory::generator::table::TableRegistry;
use resonance_music_theory::generator::{
    GenContext, GenerateError, GeneratedMaterial, Generator, GeneratorSpec,
};
use resonance_music_theory::{ChordQuality, Mode, PitchClass, Scale};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn penta_spec(flavour: PentatonicFlavour, length: u8, color: f32) -> GeneratorSpec {
    GeneratorSpec::Pentatonic {
        flavour,
        length,
        color,
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

/// Pitch class of a degree's root, projected through C in the flavour's
/// mode. Pentatonic harmony is defined by which roots are reachable, so
/// the root pitch is what the tests assert on.
fn root_pc(degree: Degree, mode: Mode) -> u8 {
    degree
        .to_chord(Scale::new(PitchClass::C, mode))
        .root
        .to_semitone()
}

/// The pentatonic root pitch classes for a flavour, projected through C.
fn allowed_root_pcs(flavour: PentatonicFlavour) -> HashSet<u8> {
    match flavour {
        // C major pentatonic: C D E G A.
        PentatonicFlavour::Major => [0, 2, 4, 7, 9].into_iter().collect(),
        // C minor pentatonic: C Eb F G Bb.
        PentatonicFlavour::Minor => [0, 3, 5, 7, 10].into_iter().collect(),
    }
}

// ---------------------------------------------------------------------------
// 1. Roots stay inside the pentatonic scale
// ---------------------------------------------------------------------------

#[test]
fn all_roots_are_pentatonic() {
    for flavour in PentatonicFlavour::ALL {
        let allowed = allowed_root_pcs(flavour);
        let mode = flavour.mode();
        // High colour to exercise the quality palette too; quality must
        // not change the root.
        let spec = penta_spec(flavour, 16, 1.0);
        for seed in 0..200u64 {
            let mat = generate_with(&spec, seed, &no_locks(16)).unwrap();
            for chord in &mat.chords {
                let pc = root_pc(chord.degree, mode);
                assert!(
                    allowed.contains(&pc),
                    "{flavour:?} seed {seed}: root pc {pc} ({}) outside pentatonic set",
                    chord.degree
                );
            }
        }
    }
}

#[test]
fn walk_opens_on_the_tonic() {
    for flavour in PentatonicFlavour::ALL {
        let spec = penta_spec(flavour, 8, 0.5);
        for seed in 0..50u64 {
            let mat = generate_with(&spec, seed, &no_locks(8)).unwrap();
            assert_eq!(
                mat.chords[0].degree.root, 1,
                "{flavour:?} seed {seed}: first chord must be the tonic"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Quality: plain triad at color 0, free qualities otherwise
// ---------------------------------------------------------------------------

#[test]
fn zero_color_uses_only_plain_triads() {
    let cases = [
        (PentatonicFlavour::Major, ChordQuality::Maj),
        (PentatonicFlavour::Minor, ChordQuality::Min),
    ];
    for (flavour, plain) in cases {
        let spec = penta_spec(flavour, 16, 0.0);
        for seed in 0..100u64 {
            let mat = generate_with(&spec, seed, &no_locks(16)).unwrap();
            for chord in &mat.chords {
                assert_eq!(
                    chord.degree.quality, plain,
                    "{flavour:?} seed {seed}: color 0 should yield only plain triads"
                );
            }
        }
    }
}

#[test]
fn color_one_introduces_palette_qualities() {
    for flavour in PentatonicFlavour::ALL {
        let palette: HashSet<ChordQuality> =
            flavour.color_qualities().iter().copied().collect();
        let spec = penta_spec(flavour, 16, 1.0);
        let mut seen_colour = false;
        for seed in 0..100u64 {
            let mat = generate_with(&spec, seed, &no_locks(16)).unwrap();
            for chord in &mat.chords {
                // Every non-plain quality must come from the palette.
                if chord.degree.quality != flavour.plain_quality() {
                    assert!(
                        palette.contains(&chord.degree.quality),
                        "{flavour:?}: quality {:?} not in palette",
                        chord.degree.quality
                    );
                    seen_colour = true;
                }
            }
        }
        assert!(seen_colour, "{flavour:?}: color 1.0 never produced a colour quality");
    }
}

// ---------------------------------------------------------------------------
// 3. Root motion favours small steps (smooth walk)
// ---------------------------------------------------------------------------

#[test]
fn root_motion_mostly_steps_around_the_ring() {
    // Over a long walk, adjacent pentatonic steps (ring distance 1) should
    // dominate distance-2 leaps, reflecting the step-weighted transition.
    let flavour = PentatonicFlavour::Major;
    let roots = flavour.roots();
    let spec = penta_spec(flavour, 64, 0.0);
    let mut step = 0u32;
    let mut leap = 0u32;
    for seed in 0..50u64 {
        let mat = generate_with(&spec, seed, &no_locks(64)).unwrap();
        let positions: Vec<usize> = mat
            .chords
            .iter()
            .map(|c| roots.iter().position(|&r| r == c.degree.root).unwrap())
            .collect();
        for w in positions.windows(2) {
            if w[0] == w[1] {
                continue; // staying put
            }
            let d = w[0].abs_diff(w[1]);
            let ring = d.min(5 - d);
            if ring == 1 {
                step += 1;
            } else {
                leap += 1;
            }
        }
    }
    assert!(
        step > leap,
        "expected more single steps than leaps, got {step} steps / {leap} leaps"
    );
}

// ---------------------------------------------------------------------------
// 4. Determinism
// ---------------------------------------------------------------------------

#[test]
fn determinism_same_seed_same_output() {
    for flavour in PentatonicFlavour::ALL {
        let spec = penta_spec(flavour, 24, 0.6);
        let a = generate_with(&spec, 42, &no_locks(24)).unwrap();
        let b = generate_with(&spec, 42, &no_locks(24)).unwrap();
        assert_eq!(a, b, "determinism failed for {flavour:?}");
    }
}

#[test]
fn different_seeds_produce_variety() {
    let spec = penta_spec(PentatonicFlavour::Minor, 8, 0.5);
    let mut unique = HashSet::new();
    for seed in 0..100u64 {
        unique.insert(degrees_of(&generate_with(&spec, seed, &no_locks(8)).unwrap()));
    }
    assert!(
        unique.len() >= 20,
        "only {} unique progressions from 100 seeds",
        unique.len()
    );
}

#[test]
fn length_zero_returns_empty() {
    let spec = penta_spec(PentatonicFlavour::Major, 0, 1.0);
    let mat = generate_with(&spec, 42, &no_locks(0)).unwrap();
    assert!(mat.chords.is_empty());
    assert!(mat.splits.is_empty());
}

// ---------------------------------------------------------------------------
// 5. Locks
// ---------------------------------------------------------------------------

#[test]
fn locked_chords_preserved_verbatim() {
    let mut locked = no_locks(8);
    // A non-pentatonic borrowed chord at a fixed waypoint must survive.
    locked[3] = Some(Degree::FLAT_VII);
    locked[6] = Some(Degree::V);

    let spec = penta_spec(PentatonicFlavour::Major, 8, 1.0);
    for seed in [0u64, 7, 42, 999] {
        let mat = generate_with(&spec, seed, &locked).unwrap();
        assert_eq!(mat.chords[3].degree, Degree::FLAT_VII, "seed {seed}");
        assert!(mat.chords[3].locked);
        assert_eq!(mat.chords[6].degree, Degree::V, "seed {seed}");
        assert!(mat.chords[6].locked);
        for (i, c) in mat.chords.iter().enumerate() {
            if i != 3 && i != 6 {
                assert!(!c.locked, "seed {seed} pos {i} should be free");
            }
        }
    }
}

#[test]
fn locks_do_not_disturb_free_positions() {
    // Free positions must be identical whether or not a *later* lock is
    // present, because locks consume no RNG draws (matches the schema
    // generator's contract).
    let spec = penta_spec(PentatonicFlavour::Minor, 8, 0.4);
    let unlocked = generate_with(&spec, 5, &no_locks(8)).unwrap();

    let mut locked = no_locks(8);
    locked[7] = Some(Degree::FLAT_VI);
    let with_lock = generate_with(&spec, 5, &locked).unwrap();

    for i in 0..7 {
        assert_eq!(
            unlocked.chords[i].degree, with_lock.chords[i].degree,
            "free position {i} disturbed by a trailing lock"
        );
    }
}

// ---------------------------------------------------------------------------
// 6. Serde: round-trip and default `color`
// ---------------------------------------------------------------------------

#[test]
fn json_roundtrip_pentatonic_spec() {
    let spec = penta_spec(PentatonicFlavour::Minor, 12, 0.35);
    let json = serde_json::to_string(&spec).unwrap();
    let parsed: GeneratorSpec = serde_json::from_str(&json).unwrap();
    let a = generate_with(&spec, 42, &no_locks(12)).unwrap();
    let b = generate_with(&parsed, 42, &no_locks(12)).unwrap();
    assert_eq!(a, b);
}

#[test]
fn color_field_defaults_to_zero() {
    let json = r#"{"type":"Pentatonic","flavour":"Major","length":4}"#;
    let parsed: GeneratorSpec = serde_json::from_str(json).unwrap();
    let mat = generate_with(&parsed, 3, &no_locks(4)).unwrap();
    // Default color 0 => all plain major triads.
    assert!(mat
        .chords
        .iter()
        .all(|c| c.degree.quality == ChordQuality::Maj));
}

// ---------------------------------------------------------------------------
// 7. PentatonicFlavour surface
// ---------------------------------------------------------------------------

#[test]
fn flavour_ids_unique_and_kebab_case() {
    let mut seen = HashSet::new();
    for flavour in PentatonicFlavour::ALL {
        assert!(seen.insert(flavour.id()), "duplicate id {}", flavour.id());
        assert!(
            flavour
                .id()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "id {} is not kebab-case",
            flavour.id()
        );
        assert!(!flavour.name().is_empty());
        assert_eq!(flavour.to_string(), flavour.name());
    }
}

#[test]
fn flavours_report_their_mode() {
    assert_eq!(PentatonicFlavour::Major.mode(), Mode::Major);
    assert_eq!(PentatonicFlavour::Minor.mode(), Mode::Minor);
}
