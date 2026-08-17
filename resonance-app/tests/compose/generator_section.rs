//! Integration tests for generator fields on sections.

use resonance_music_theory::generator::degree::Degree;
use resonance_music_theory::{
    GenContext, GeneratedMaterial, Generator, GeneratorSpec, TableRegistry,
};

/// JSON round-trip: serialize a section with a Markov spec, deserialize,
/// regenerate, confirm equality.
#[test]
fn section_with_generator_json_roundtrip() {
    // Build a section definition with generator fields set.
    let spec = GeneratorSpec::MarkovProgression {
        length: 4,
        table_id: "pop".to_string(),
        order: 1,
        start: Some(Degree::I),
        end: None,
    };
    let seed = 42u64;
    let reg = TableRegistry::with_builtins();
    let locked = vec![None; 4];
    let ctx = GenContext {
        registry: &reg,
        locked: &locked,
    };
    let material = spec.generate(seed, &ctx).unwrap();

    let def = serde_json::json!({
        "id": 1,
        "name": "Verse",
        "color": [255, 128, 0],
        "length_bars": 8,
        "chords": [{
            "id": 100,
            "start_beat": 0,
            "duration_beats": 4,
            "chord": { "root": 0, "quality": "Maj", "bass": null }
        }],
        "scale": { "root": 0, "mode": "Major" },
        "progression_seed": 0,
        "generate_params": {
            "chord_count": 4,
            "beats_per_chord": 4,
            "seventh_chords": false,
            "pad": { "register": 4, "velocity": 80 },
            "bass": { "style": "RootHold", "velocity": 90, "octave": 2 },
            "melody": { "style": "Arpeggiate", "velocity": 70, "register": 5 }
        },
        "generator_spec": serde_json::to_value(&spec).unwrap(),
        "generator_seed": seed,
        "generated_material": serde_json::to_value(&material).unwrap()
    });

    // Serialize and deserialize the whole section.
    let json_str = serde_json::to_string_pretty(&def).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    // Extract and verify generator fields.
    let parsed_spec: GeneratorSpec =
        serde_json::from_value(parsed["generator_spec"].clone()).unwrap();
    let parsed_seed = parsed["generator_seed"].as_u64().unwrap();
    let parsed_material: GeneratedMaterial =
        serde_json::from_value(parsed["generated_material"].clone()).unwrap();

    assert_eq!(parsed_seed, seed);
    assert_eq!(parsed_material, material);

    // Re-generate from parsed spec + seed and confirm same output.
    let regen = parsed_spec.generate(parsed_seed, &ctx).unwrap();
    assert_eq!(regen, material);
}

/// Loading an old section JSON (no generator fields) still deserializes.
#[test]
fn legacy_section_without_generator_fields() {
    let legacy = r#"{
        "id": 1,
        "name": "Intro",
        "color": [0, 0, 0],
        "length_bars": 4
    }"#;

    // This should not fail -- missing generator fields default cleanly.
    let parsed: serde_json::Value = serde_json::from_str(legacy).unwrap();
    assert_eq!(parsed["name"], "Intro");
    assert_eq!(parsed["length_bars"], 4);
    // generator_spec, generator_seed, generated_material should be absent
    // but that's fine -- they default when deserialized into the struct.
    assert!(parsed.get("generator_spec").is_none());
    assert!(parsed.get("generator_seed").is_none());
    assert!(parsed.get("generated_material").is_none());
}
