//! Persistence + legacy migration for the section drum arrangement
//! (epic #38, todo #485).
//!
//! Covers the save -> load -> save path for the new arrangement shape and
//! the three legacy migrations called out in acceptance #6:
//!   * a new-shape `Vec<PatternEntry>` round-trips through the project file
//!     (in-memory *and* serde JSON) byte-for-byte (semantically), and a
//!     re-save is stable;
//!   * a legacy `drum_pattern_id: Some(id)` opens as a single one-entry
//!     arrangement tiling the whole section, then re-saves in the new shape;
//!   * a legacy `drum_pattern_id: None` opens as an empty arrangement;
//!   * the legacy flat-`drum_groups` project path (promoted by
//!     `restore_drum_patterns`) still loads and keeps the drum lane working.
//!
//! `resonance-app` exposes a `lib.rs`, so the integration crate can drive
//! `ComposeState` round-trips directly and replay a `ProjectFile` into a
//! `Resonance` via the test-only `test_replay_compose` helper.

use std::collections::HashMap;

use resonance_app::compose::drumroll::default_drum_groups;
use resonance_app::compose::{ComposeState, EntryLength, PatternEntry, SectionDefinitionState};
use resonance_app::project::{ProjectFile, ProjectSectionDefinition};
use resonance_app::state::ViewMode;
use resonance_app::{demo, Resonance, STARTUP_TAB};

// ---- helpers ----------------------------------------------------------

/// A bare section definition with the given id, bar length, and
/// arrangement. All generator/chord fields are left at their defaults —
/// this todo only exercises the drum arrangement.
fn section(id: u64, length_bars: u32, arrangement: Vec<PatternEntry>) -> SectionDefinitionState {
    SectionDefinitionState {
        id,
        name: format!("S{id}"),
        color: [1, 2, 3],
        length_bars,
        chords: Vec::new(),
        scale: None,
        progression_seed: 0,
        generate_params: Default::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators: HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: Default::default(),
        arrangement,
    }
}

/// A `ComposeState` seeded with a single section carrying `arrangement`.
fn state_with_arrangement(length_bars: u32, arrangement: Vec<PatternEntry>) -> ComposeState {
    ComposeState {
        definitions: vec![section(1, length_bars, arrangement)],
        ..ComposeState::default()
    }
}

/// Round-trip a `ComposeState` through the in-memory project shapes
/// (`to_project_definitions` -> `load_from_project`) and return the
/// reloaded section's arrangement.
fn roundtrip_in_memory(state: &ComposeState) -> Vec<PatternEntry> {
    let defs = state.to_project_definitions();
    let placements = state.to_project_placements();
    let mut dst = ComposeState::default();
    dst.load_from_project(&defs, &placements);
    dst.definitions[0].arrangement.clone()
}

/// Round-trip through serialized JSON, the exact bytes Save -> Load writes.
fn roundtrip_json(state: &ComposeState) -> Vec<PatternEntry> {
    let defs = state.to_project_definitions();
    let json = serde_json::to_string(&defs).expect("serialize definitions");
    let parsed: Vec<ProjectSectionDefinition> =
        serde_json::from_str(&json).expect("deserialize definitions");
    let mut dst = ComposeState::default();
    dst.load_from_project(&parsed, &[]);
    dst.definitions[0].arrangement.clone()
}

// ---- new-shape round-trip ---------------------------------------------

#[test]
fn new_shape_arrangement_round_trips_in_memory() {
    let arrangement = vec![
        PatternEntry {
            pattern_id: 7,
            length: EntryLength::RepeatN(3),
            fill: Some(9),
        },
        PatternEntry {
            pattern_id: 8,
            length: EntryLength::Bars(2),
            fill: None,
        },
        PatternEntry::once(7),
    ];
    let state = state_with_arrangement(8, arrangement.clone());

    assert_eq!(roundtrip_in_memory(&state), arrangement);
}

#[test]
fn new_shape_arrangement_round_trips_through_json() {
    let arrangement = vec![
        PatternEntry {
            pattern_id: 11,
            length: EntryLength::Bars(4),
            fill: None,
        },
        PatternEntry {
            pattern_id: 12,
            length: EntryLength::RepeatN(2),
            fill: Some(13),
        },
    ];
    let state = state_with_arrangement(8, arrangement.clone());

    assert_eq!(roundtrip_json(&state), arrangement);
}

/// save -> load -> save must be stable: the second persisted form equals
/// the first, so re-saving an already-migrated project never drifts.
#[test]
fn save_load_save_is_stable() {
    let arrangement = vec![
        PatternEntry {
            pattern_id: 4,
            length: EntryLength::RepeatN(2),
            fill: Some(5),
        },
        PatternEntry {
            pattern_id: 6,
            length: EntryLength::Bars(3),
            fill: None,
        },
    ];
    let state = state_with_arrangement(8, arrangement);

    // First save.
    let defs1 = state.to_project_definitions();
    // Load it back...
    let mut reloaded = ComposeState::default();
    reloaded.load_from_project(&defs1, &state.to_project_placements());
    // ...and save again.
    let defs2 = reloaded.to_project_definitions();

    let json1 = serde_json::to_string(&defs1).expect("serialize #1");
    let json2 = serde_json::to_string(&defs2).expect("serialize #2");
    assert_eq!(
        json1, json2,
        "re-saving a loaded project must be byte-stable"
    );
}

/// The new shape is authoritative: a save writes the full `arrangement`
/// and clears the legacy `drum_pattern_id` so loaders never double-count.
#[test]
fn new_save_clears_legacy_pattern_id_and_writes_arrangement() {
    let state = state_with_arrangement(
        4,
        vec![PatternEntry {
            pattern_id: 42,
            length: EntryLength::RepeatN(4),
            fill: None,
        }],
    );
    let defs = state.to_project_definitions();
    assert_eq!(defs[0].drum_pattern_id, None);
    assert_eq!(defs[0].arrangement.len(), 1);
    assert_eq!(defs[0].arrangement[0].pattern_id, 42);

    // And the legacy id is omitted from the JSON entirely once null...
    let json = serde_json::to_string(&defs).expect("serialize");
    assert!(
        json.contains("\"arrangement\""),
        "new save must persist the arrangement field: {json}"
    );
}

// ---- legacy migration: drum_pattern_id Some / None --------------------

/// An old project.json whose section carries `drum_pattern_id: Some(id)`
/// (and no `arrangement`) opens as a single one-entry arrangement that
/// tiles the whole section: `RepeatN(length_bars)` of a (1-bar) legacy
/// pattern covers exactly `length_bars` bars.
#[test]
fn legacy_some_pattern_id_migrates_to_one_entry_tiling_the_section() {
    let legacy_json = r#"[
        {"id":1,"name":"Verse","color":[0,0,0],"length_bars":8,"drum_pattern_id":77}
    ]"#;
    let parsed: Vec<ProjectSectionDefinition> =
        serde_json::from_str(legacy_json).expect("legacy deserialize");
    // No `arrangement` key -> defaults to empty on the persisted struct.
    assert!(parsed[0].arrangement.is_empty());
    assert_eq!(parsed[0].drum_pattern_id, Some(77));

    let mut state = ComposeState::default();
    state.load_from_project(&parsed, &[]);

    let arr = &state.definitions[0].arrangement;
    assert_eq!(
        arr,
        &vec![PatternEntry {
            pattern_id: 77,
            length: EntryLength::RepeatN(8),
            fill: None,
        }],
        "legacy Some(id) must open as one entry tiling all {} bars",
        8
    );
}

/// ...and re-saving that migrated section emits the new shape: the full
/// arrangement, with the legacy `drum_pattern_id` cleared.
#[test]
fn legacy_some_pattern_id_resaves_in_new_shape() {
    let legacy_json = r#"[
        {"id":1,"name":"Verse","color":[0,0,0],"length_bars":6,"drum_pattern_id":5}
    ]"#;
    let parsed: Vec<ProjectSectionDefinition> =
        serde_json::from_str(legacy_json).expect("legacy deserialize");

    let mut state = ComposeState::default();
    state.load_from_project(&parsed, &[]);

    let resaved = state.to_project_definitions();
    assert_eq!(resaved[0].drum_pattern_id, None);
    assert_eq!(resaved[0].arrangement.len(), 1);
    assert_eq!(resaved[0].arrangement[0].pattern_id, 5);
    assert_eq!(
        resaved[0].arrangement[0].length,
        resonance_app::project::ProjectEntryLength::RepeatN(6)
    );

    // Loading the re-saved (new-shape) project reproduces the arrangement
    // verbatim — the migration only happens once.
    let mut reloaded = ComposeState::default();
    reloaded.load_from_project(&resaved, &[]);
    assert_eq!(
        reloaded.definitions[0].arrangement,
        state.definitions[0].arrangement
    );
}

/// A legacy `drum_pattern_id: None` (and no arrangement) opens as an empty
/// arrangement — "use the project default pattern".
#[test]
fn legacy_none_pattern_id_migrates_to_empty_arrangement() {
    let legacy_json = r#"[
        {"id":1,"name":"Intro","color":[0,0,0],"length_bars":4}
    ]"#;
    let parsed: Vec<ProjectSectionDefinition> =
        serde_json::from_str(legacy_json).expect("legacy deserialize");
    assert_eq!(parsed[0].drum_pattern_id, None);
    assert!(parsed[0].arrangement.is_empty());

    let mut state = ComposeState::default();
    state.load_from_project(&parsed, &[]);

    assert!(
        state.definitions[0].arrangement.is_empty(),
        "legacy None must open as an empty arrangement"
    );
}

/// A non-empty persisted `arrangement` always wins over a stray legacy
/// `drum_pattern_id` (belt-and-braces: a hand-edited file with both keys
/// must not double-count).
#[test]
fn explicit_arrangement_takes_precedence_over_legacy_pattern_id() {
    let json = r#"[
        {
            "id":1,"name":"Verse","color":[0,0,0],"length_bars":8,
            "drum_pattern_id":99,
            "arrangement":[{"pattern_id":3,"length":{"Bars":8}}]
        }
    ]"#;
    let parsed: Vec<ProjectSectionDefinition> = serde_json::from_str(json).expect("deserialize");

    let mut state = ComposeState::default();
    state.load_from_project(&parsed, &[]);

    assert_eq!(
        state.definitions[0].arrangement,
        vec![PatternEntry {
            pattern_id: 3,
            length: EntryLength::Bars(8),
            fill: None,
        }],
        "explicit arrangement must override the legacy single id"
    );
}

// ---- legacy drum_groups path ------------------------------------------

/// Demo app pinned to the Compose tab so a focused section is resolvable.
fn build_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Compose);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    app
}

/// A pre-pattern-bank project (flat `drum_groups`, no `drum_patterns`)
/// whose section has no `drum_pattern_id` must still load: the loader
/// promotes the flat groups into a one-pattern "Main" bank and points the
/// section's (empty) arrangement at it via `set_primary_pattern`, so the
/// drum lane resolves identically to how the legacy project rendered.
#[test]
fn legacy_drum_groups_promote_to_pattern_and_seed_primary() {
    let mut app = build_app();

    // Build a v2-era project file: flat drum_groups, empty drum_patterns,
    // one section with a legacy `None` pattern id.
    let mut next_id = 1_000u64;
    let groups = default_drum_groups(&mut next_id);
    let group_count = groups.len();
    assert!(group_count > 0, "default kit has groups");

    let file = ProjectFile {
        section_definitions: vec![ProjectSectionDefinition {
            id: 1,
            name: "Verse".to_string(),
            color: [0, 0, 0],
            length_bars: 4,
            chords: Vec::new(),
            scale: None,
            progression_seed: 0,
            generate_params: Default::default(),
            generator_spec: None,
            generator_seed: 0,
            generated_material: None,
            lane_generators: HashMap::new(),
            beats_per_chord: 4,
            seventh_chords: false,
            motif_source: Default::default(),
            drum_pattern_id: None,
            arrangement: Vec::new(),
        }],
        drum_groups: groups,
        drum_patterns: Vec::new(),
        ..ProjectFile::default()
    };

    app.test_replay_compose(&file);

    let compose = app.compose_state();
    // The flat groups were promoted into a single "Main" pattern bank.
    assert_eq!(compose.drum_patterns.len(), 1, "one promoted pattern");
    let main = &compose.drum_patterns[0];
    assert_eq!(main.name, "Main");
    assert_eq!(main.groups.len(), group_count, "groups carried over intact");
    assert_eq!(compose.default_drum_pattern_id, Some(main.id));

    // The section, which had no pattern id, now resolves to the promoted
    // pattern as its primary — the drum lane keeps working.
    let def = compose
        .find_definition(1)
        .expect("section definition loaded");
    assert_eq!(def.primary_pattern_id(), Some(main.id));
    // pattern_for_definition resolves to the promoted pattern across the
    // section so the lane is not silent.
    let resolved = compose
        .pattern_for_definition(def)
        .expect("a pattern resolves for the drum lane");
    assert_eq!(resolved.id, main.id);
}

/// A modern project with a populated `drum_patterns` bank and a new-shape
/// `arrangement` loads through the same replay path with the arrangement
/// preserved (the legacy `drum_groups` branch is not taken).
#[test]
fn modern_bank_plus_arrangement_loads_through_replay() {
    let mut app = build_app();

    // Reuse the seeded demo bank so pattern ids are real.
    let (p0, p1) = {
        let bank = &app.compose_state().drum_patterns;
        (bank[0].id, bank[1].id)
    };

    let arrangement = vec![
        resonance_app::project::ProjectPatternEntry {
            pattern_id: p0,
            length: resonance_app::project::ProjectEntryLength::RepeatN(2),
            fill: Some(p1),
        },
        resonance_app::project::ProjectPatternEntry {
            pattern_id: p1,
            length: resonance_app::project::ProjectEntryLength::Bars(2),
            fill: None,
        },
    ];

    let bank = app.compose_state().drum_patterns.clone();
    let file = ProjectFile {
        section_definitions: vec![ProjectSectionDefinition {
            id: 1,
            name: "Verse".to_string(),
            color: [0, 0, 0],
            length_bars: 8,
            chords: Vec::new(),
            scale: None,
            progression_seed: 0,
            generate_params: Default::default(),
            generator_spec: None,
            generator_seed: 0,
            generated_material: None,
            lane_generators: HashMap::new(),
            beats_per_chord: 4,
            seventh_chords: false,
            motif_source: Default::default(),
            drum_pattern_id: None,
            arrangement,
        }],
        drum_groups: Vec::new(),
        drum_patterns: bank,
        ..ProjectFile::default()
    };

    app.test_replay_compose(&file);

    let def = app
        .compose_state()
        .find_definition(1)
        .expect("section loaded");
    assert_eq!(
        def.arrangement,
        vec![
            PatternEntry {
                pattern_id: p0,
                length: EntryLength::RepeatN(2),
                fill: Some(p1),
            },
            PatternEntry {
                pattern_id: p1,
                length: EntryLength::Bars(2),
                fill: None,
            },
        ]
    );
}
