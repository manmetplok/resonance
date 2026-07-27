use super::*;
use resonance_music_theory::{Chord, ChordQuality, MelodyParams, Mode, MotifParams, MotifSource, PitchClass, Scale};
use std::collections::HashMap;

fn state_with_chords_and_scale() -> ComposeState {
    let mut state = ComposeState::default();
    let def_id = state.fresh_id();
    let chord_a = state.fresh_id();
    let chord_b = state.fresh_id();
    state.definitions.push(SectionDefinitionState {
        id: def_id,
        name: "Verse".to_string(),
        color: [10, 20, 30],
        length_bars: 8,
        chords: vec![
            ChordState {
                id: chord_a,
                start_beat: 0,
                duration_beats: 4,
                chord: Chord::new(PitchClass::C, ChordQuality::Maj7),
            },
            ChordState {
                id: chord_b,
                start_beat: 4,
                duration_beats: 2,
                chord: Chord::new(PitchClass::D, ChordQuality::Min7).with_bass(PitchClass::F),
            },
        ],
        scale: Some(Scale::new(PitchClass::C, Mode::Minor)),
        progression_seed: 12345,
        generate_params: GenerateParams::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators: HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: MotifSource::default(),
        arrangement: Vec::new(),
    });
    let placement_id = state.fresh_id();
    state.placements.push(SectionPlacementState {
        id: placement_id,
        definition_id: def_id,
        start_bar: 0,
    });
    state.selected_placement_id = Some(placement_id);
    state
}

/// Round-tripping ComposeState → ProjectFile shapes → ComposeState must
/// preserve every chord and the selected scale exactly. This is the
/// exact path taken by Save -> Load in the real app.
#[test]
fn in_memory_roundtrip_preserves_chords_and_scale() {
    let src = state_with_chords_and_scale();
    let persisted_defs = src.to_project_definitions();
    let persisted_placements = src.to_project_placements();

    let mut dst = ComposeState::default();
    dst.load_from_project(&persisted_defs, &persisted_placements);

    assert_eq!(dst.definitions.len(), 1);
    let def = &dst.definitions[0];
    assert_eq!(def.name, "Verse");
    assert_eq!(def.length_bars, 8);
    assert_eq!(def.color, [10, 20, 30]);
    assert_eq!(def.scale, Some(Scale::new(PitchClass::C, Mode::Minor)));

    assert_eq!(def.chords.len(), 2);
    assert_eq!(def.chords[0].start_beat, 0);
    assert_eq!(def.chords[0].duration_beats, 4);
    assert_eq!(def.chords[0].chord.root, PitchClass::C);
    assert_eq!(def.chords[0].chord.quality, ChordQuality::Maj7);
    assert_eq!(def.chords[0].chord.bass, None);
    assert_eq!(def.chords[1].chord.root, PitchClass::D);
    assert_eq!(def.chords[1].chord.quality, ChordQuality::Min7);
    assert_eq!(def.chords[1].chord.bass, Some(PitchClass::F));

    assert_eq!(dst.placements.len(), 1);
    assert_eq!(dst.placements[0].definition_id, def.id);
}

/// The actual save path calls serde_json on `ProjectSectionDefinition`
/// values. Make sure that round trip is lossless too.
#[test]
fn json_serde_roundtrip_preserves_chords_and_scale() {
    let src = state_with_chords_and_scale();
    let persisted = src.to_project_definitions();

    let json = serde_json::to_string(&persisted).expect("serialize");
    let parsed: Vec<crate::project::ProjectSectionDefinition> =
        serde_json::from_str(&json).expect("deserialize");

    let mut dst = ComposeState::default();
    dst.load_from_project(&parsed, &[]);

    let def = &dst.definitions[0];
    assert_eq!(def.name, "Verse");
    assert_eq!(def.scale, Some(Scale::new(PitchClass::C, Mode::Minor)));
    assert_eq!(def.chords.len(), 2);
    assert_eq!(def.chords[0].chord.quality, ChordQuality::Maj7);
    assert_eq!(def.chords[1].chord.bass, Some(PitchClass::F));
    // Generation fields round-trip unchanged.
    assert_eq!(def.progression_seed, 12345);
    assert_eq!(def.generate_params.chord_count, 4);
    assert_eq!(def.generate_params.beats_per_chord, 4);
}

/// Old project files predating this feature won't have `chords` or
/// `scale` keys on the definition. Both must default so the file still
/// loads.
#[test]
fn loads_old_project_files_without_chords_or_scale() {
    let legacy_json = r#"[{"id":1,"name":"Intro","color":[0,0,0],"length_bars":4}]"#;
    let parsed: Vec<crate::project::ProjectSectionDefinition> =
        serde_json::from_str(legacy_json).expect("legacy deserialize");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "Intro");
    assert_eq!(parsed[0].length_bars, 4);
    assert!(parsed[0].chords.is_empty());
    assert_eq!(parsed[0].scale, None);
}

/// Section motif knobs round-trip through the project I/O path.
#[test]
fn section_motif_round_trips_through_project_io() {
    let mut state = state_with_chords_and_scale();
    state.definitions[0].motif_source = MotifSource::Generated(MotifParams {
        seed: 0xDEAD_BEEF_1234_5678,
        complexity: 0.73,
        motif_len: 5,
        leap_chance: 0.42,
    });

    let project_defs = state.to_project_definitions();
    let project_placements = state.to_project_placements();

    let mut restored = ComposeState::default();
    restored.load_from_project(&project_defs, &project_placements);

    let restored_motif = restored.definitions[0].motif_source.params();
    assert_eq!(restored_motif.seed, 0xDEAD_BEEF_1234_5678);
    assert_eq!(restored_motif.complexity, 0.73);
    assert_eq!(restored_motif.motif_len, 5);
    assert_eq!(restored_motif.leap_chance, 0.42);
    assert!(matches!(
        restored.definitions[0].motif_source,
        MotifSource::Generated(_)
    ));
}

/// A project file without the `motif` key (predating this feature)
/// must still deserialize, with motif defaulting via serde.
#[test]
fn legacy_project_without_motif_field_loads_with_defaults() {
    let json = r#"{
        "id": 1,
        "name": "Legacy",
        "color": [0, 0, 0],
        "length_bars": 8
    }"#;
    let parsed: crate::project::ProjectSectionDefinition =
        serde_json::from_str(json).expect("legacy section JSON should parse");
    assert_eq!(parsed.motif_source, MotifSource::default());
}

/// A project file written before the `MotifSource` enum existed stored
/// the motif as a flat `MotifParams`. The custom deserializer must
/// accept that legacy shape and lift it into `Generated(...)`.
#[test]
fn legacy_project_with_flat_motif_params_loads_as_generated() {
    let json = r#"{
        "id": 1,
        "name": "Legacy",
        "color": [0, 0, 0],
        "length_bars": 8,
        "motif": { "seed": 7, "complexity": 0.42, "motif_len": 3, "leap_chance": 0.6 }
    }"#;
    let parsed: crate::project::ProjectSectionDefinition =
        serde_json::from_str(json).expect("legacy flat-motif JSON should parse");
    match parsed.motif_source {
        MotifSource::Generated(p) => {
            assert_eq!(p.seed, 7);
            assert_eq!(p.complexity, 0.42);
            assert_eq!(p.motif_len, 3);
            assert_eq!(p.leap_chance, 0.6);
        }
        other => panic!("expected Generated, got {other:?}"),
    }
}

/// Older projects: a melody lane carries non-default complexity / motif_len /
/// leap_chance. After migration these should land on `def.motif`.
#[test]
fn migration_lifts_legacy_melody_motif_knobs_onto_section() {
    use resonance_music_theory::MelodyStyle;

    let mut state = ComposeState::default();
    let def_id = state.fresh_id();
    state.definitions.push(SectionDefinitionState {
        id: def_id,
        name: "Verse".to_string(),
        color: [0, 0, 0],
        length_bars: 8,
        chords: Vec::new(),
        scale: None,
        progression_seed: 0,
        generate_params: GenerateParams::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators: HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: MotifSource::default(),
        arrangement: Vec::new(),
    });

    let custom_melody = MelodyParams {
        style: MelodyStyle::Motif,
        complexity: 0.85,
        motif_len: 6,
        leap_chance: 0.55,
        ..MelodyParams::default()
    };
    state.definitions[0].lane_generators.insert(
        100,
        LaneGeneratorConfig {
            kind: LaneGeneratorKind::Melody(custom_melody),
            seed: 0xCAFEBABE,
        },
    );

    state.migrate_old_generate_params(&[]);

    let migrated = state.definitions[0].motif_source.params();
    assert_eq!(migrated.complexity, 0.85);
    assert_eq!(migrated.motif_len, 6);
    assert_eq!(migrated.leap_chance, 0.55);
    assert_eq!(migrated.seed, 0xCAFEBABE);
}

/// If the section's motif was loaded with explicit values, the
/// migration path must not overwrite them.
#[test]
fn migration_skips_when_motif_already_customized() {
    use resonance_music_theory::MelodyStyle;

    let mut state = ComposeState::default();
    let def_id = state.fresh_id();
    state.definitions.push(SectionDefinitionState {
        id: def_id,
        name: "Verse".to_string(),
        color: [0, 0, 0],
        length_bars: 8,
        chords: Vec::new(),
        scale: None,
        progression_seed: 0,
        generate_params: GenerateParams::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators: HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: MotifSource::Generated(MotifParams {
            seed: 7,
            complexity: 0.1,
            motif_len: 2,
            leap_chance: 0.05,
        }),
        arrangement: Vec::new(),
    });

    let custom_melody = MelodyParams {
        style: MelodyStyle::Motif,
        complexity: 0.99,
        motif_len: 6,
        leap_chance: 0.99,
        ..MelodyParams::default()
    };
    state.definitions[0].lane_generators.insert(
        100,
        LaneGeneratorConfig {
            kind: LaneGeneratorKind::Melody(custom_melody),
            seed: 1234,
        },
    );

    state.migrate_old_generate_params(&[]);

    let after = state.definitions[0].motif_source.params();
    assert_eq!(after.complexity, 0.1);
    assert_eq!(after.motif_len, 2);
    assert_eq!(after.leap_chance, 0.05);
    assert_eq!(after.seed, 7);
}
