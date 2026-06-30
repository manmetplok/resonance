//! Persistence coverage for the MIDI quantize state (ba todo #395):
//! the project's user-extracted groove library and the last-used
//! quantize / humanize settings. Proves both serialize into the on-disk
//! `ProjectFile`, restore back through the same path a project load runs,
//! and round-trip byte-for-byte — while a legacy project that predates
//! the fields loads cleanly with an empty library and neutral defaults.

use resonance_app::project::ProjectFile;
use resonance_app::state::{GrooveSelection, QuantizeSettings, UserGroove};
use resonance_app::Resonance;
use resonance_audio::quantize::{Division, GridValue, GrooveTemplate, QuantizeMode};

/// A non-identity groove template with distinctive per-step data so a
/// round-trip that silently dropped a vector would be caught.
fn groove_template() -> GrooveTemplate {
    let mut t = GrooveTemplate::identity(8);
    t.timing_offsets_ticks = vec![0, 12, -4, 7, 0, 15, -2, 9];
    t.velocity_scale = vec![1.0, 0.9, 1.05, 0.8, 1.0, 0.95, 1.1, 0.85];
    t
}

/// A deliberately non-default settings block (every field differs from
/// `QuantizeSettings::default`) so the round-trip exercises each field.
fn custom_settings() -> QuantizeSettings {
    QuantizeSettings {
        division: Division::triplet(GridValue::Eighth),
        strength: 0.75,
        swing: 0.33,
        mode: QuantizeMode::StartAndLength,
        quantize_ends: true,
        iterative: true,
        humanize_timing_ticks: 18,
        humanize_velocity: 0.2,
        groove: GrooveSelection::User { id: 2 },
        groove_strength: 0.6,
    }
}

#[test]
fn build_project_file_captures_grooves_and_settings() {
    let (mut app, _task) = Resonance::new();

    let q = app.test_quantize_mut();
    q.groove_library.push(UserGroove {
        id: 1,
        name: "Stock Swing".into(),
        template: GrooveTemplate::identity(16),
    });
    q.groove_library.push(UserGroove {
        id: 2,
        name: "My Feel".into(),
        template: groove_template(),
    });
    q.settings = custom_settings();

    let file = app.test_build_project_file();

    assert_eq!(file.groove_library.len(), 2, "both grooves serialized");
    assert_eq!(file.groove_library[0].id, 1);
    assert_eq!(file.groove_library[0].name, "Stock Swing");
    assert_eq!(file.groove_library[1].id, 2);
    assert_eq!(file.groove_library[1].name, "My Feel");
    assert_eq!(file.groove_library[1].template, groove_template());

    assert_eq!(file.quantize_settings, custom_settings());
}

#[test]
fn grooves_and_settings_round_trip_identically() {
    // -- Author a project with a groove library + custom settings ----
    let (mut authored, _t1) = Resonance::new();
    {
        let q = authored.test_quantize_mut();
        q.groove_library.push(UserGroove {
            id: 1,
            name: "Stock Swing".into(),
            template: GrooveTemplate::identity(16),
        });
        q.groove_library.push(UserGroove {
            id: 2,
            name: "My Feel".into(),
            template: groove_template(),
        });
        q.settings = custom_settings();
    }

    let saved = authored.test_build_project_file();

    // Persist through real JSON, exactly as a save/reload would.
    let json = serde_json::to_string(&saved).expect("serialize project");
    let reloaded: ProjectFile = serde_json::from_str(&json).expect("deserialize project");

    // -- Restore into a fresh app via the production restore path -----
    let (mut restored, _t2) = Resonance::new();
    restored.test_restore_quantize(&reloaded);

    let q = restored.test_quantize();
    assert_eq!(q.groove_library.len(), 2);
    assert_eq!(q.groove_library[1].name, "My Feel");
    assert_eq!(q.groove_library[1].template, groove_template());
    assert_eq!(q.settings, custom_settings());
    // Derived next id picks up after the highest restored id.
    assert_eq!(q.next_groove_id(), 3);
    // The user-groove referenced by the restored settings resolves.
    assert!(q.user_groove(2).is_some());

    // Re-serializing the restored app yields byte-identical JSON — the
    // definition of a clean round-trip.
    let re_saved = restored.test_build_project_file();
    let re_json = serde_json::to_string(&re_saved).expect("re-serialize");
    assert_eq!(re_json, json, "quantize state round-trips identically");
}

#[test]
fn legacy_project_without_quantize_loads_clean() {
    // A project authored before the quantize fields existed has neither
    // `groove_library` nor `quantize_settings`. `#[serde(default)]` must
    // fill them in without error.
    let mut json = serde_json::to_value(ProjectFile::default()).unwrap();
    let obj = json.as_object_mut().unwrap();
    obj.remove("groove_library");
    obj.remove("quantize_settings");

    let file: ProjectFile =
        serde_json::from_value(json).expect("legacy project deserializes");
    assert!(file.groove_library.is_empty());
    assert_eq!(file.quantize_settings, QuantizeSettings::default());

    // Restoring the empty legacy block is a clean no-op on a fresh app.
    let (mut app, _task) = Resonance::new();
    app.test_restore_quantize(&file);
    assert!(app.test_quantize().groove_library.is_empty());
    assert_eq!(app.test_quantize().settings, QuantizeSettings::default());
    assert_eq!(app.test_quantize().next_groove_id(), 0);
}
