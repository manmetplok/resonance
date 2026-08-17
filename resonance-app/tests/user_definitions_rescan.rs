//! Tests for the user-definition import/reveal entry point and registry re-scan
//! (architecture doc #201 §5 "A5", ba todo #728).
//!
//! DoD: re-scan picks up a newly added user definition. We test this by
//! directly scanning a temp directory via `test_rescan_definitions_from` (which
//! exercises the same code path as `RescanDefinitions` but without the
//! `$XDG_DATA_HOME` dependency so tests remain hermetic).
//!
//! We also verify the undo classifier marks the runtime-only messages as `Skip`
//! so they don't pollute the undo history.

use resonance_app::message::{ExternalInstrumentMessage as Eim, Message};
use resonance_app::undo::{classify, UndoAction};
use resonance_app::Resonance;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A minimal valid device-definition JSON for a fictional synth. Uses the
/// same `schema_version` and field shape as the bundled Moog Muse definition
/// (the validator accepts any definition that passes its checks).
const TEST_DEF_JSON: &str = r#"{
  "id": "test-synth-x1",
  "manufacturer": "Test Corp",
  "model": "Synth X1",
  "schema_version": 1,
  "params": [
    {
      "id": "cutoff",
      "name": "Filter Cutoff",
      "group": "Filter",
      "binding": { "Cc": { "cc": 74 } },
      "min": 0,
      "max": 127,
      "curve": "Linear"
    }
  ],
  "patches": []
}"#;

/// Write `TEST_DEF_JSON` into `dir` as `test-synth-x1.json`, return the path.
fn write_test_def(dir: &TempDir) -> std::path::PathBuf {
    let path = dir.path().join("test-synth-x1.json");
    std::fs::write(&path, TEST_DEF_JSON).expect("write test definition");
    path
}

// ---------------------------------------------------------------------------
// Undo-classifier: runtime-only messages must not pollute undo history
// ---------------------------------------------------------------------------

#[test]
fn rescan_definitions_is_skipped_for_undo() {
    assert!(
        matches!(
            classify(&Message::ExternalInstrument(Eim::RescanDefinitions)),
            UndoAction::Skip
        ),
        "RescanDefinitions must not be recorded in undo history"
    );
}

#[test]
fn reveal_user_definitions_folder_is_skipped_for_undo() {
    assert!(
        matches!(
            classify(&Message::ExternalInstrument(
                Eim::RevealUserDefinitionsFolder
            )),
            UndoAction::Skip
        ),
        "RevealUserDefinitionsFolder must not be recorded in undo history"
    );
}

// ---------------------------------------------------------------------------
// Re-scan: a new file dropped in the user folder appears after re-scan
// ---------------------------------------------------------------------------

#[test]
fn rescan_picks_up_newly_added_user_definition() {
    let (mut app, _task) = Resonance::new_for_test();

    // The bundled Moog Muse definition is always present.
    let initial_ids = app.test_device_registry_ids();
    assert!(
        initial_ids.contains(&"moog-muse".to_string()),
        "bundled definition present at startup"
    );
    assert!(
        !initial_ids.contains(&"test-synth-x1".to_string()),
        "test definition not yet present"
    );

    // Write a new definition to a temp directory and rescan.
    let dir = TempDir::new().expect("temp dir");
    write_test_def(&dir);
    app.test_rescan_definitions_from(dir.path());

    let after_ids = app.test_device_registry_ids();
    assert!(
        after_ids.contains(&"test-synth-x1".to_string()),
        "newly added user definition appears after re-scan; got: {after_ids:?}"
    );
    // Bundled definition is still present (last-wins keeps both).
    assert!(
        after_ids.contains(&"moog-muse".to_string()),
        "bundled definition still present after re-scan"
    );
}

#[test]
fn rescan_rebuilds_device_picker_cache() {
    let (mut app, _task) = Resonance::new_for_test();

    // Before re-scan: test synth not offered in the pick-list.
    assert!(
        !app.test_device_choice_ids().contains(&"test-synth-x1".to_string()),
        "test synth not in cache before re-scan"
    );

    // Drop definition into temp dir and rescan.
    let dir = TempDir::new().expect("temp dir");
    write_test_def(&dir);
    app.test_rescan_definitions_from(dir.path());

    // After re-scan: picker cache is rebuilt.
    let choices = app.test_device_choice_ids();
    assert!(
        choices.contains(&"test-synth-x1".to_string()),
        "picker cache rebuilt after re-scan; got: {choices:?}"
    );
    assert!(
        choices.contains(&"moog-muse".to_string()),
        "bundled definition still in picker cache after re-scan"
    );
}

#[test]
fn rescan_with_empty_dir_preserves_bundled_definitions() {
    let (mut app, _task) = Resonance::new_for_test();

    // Rescan against an empty temp dir — bundled defs must survive.
    let dir = TempDir::new().expect("temp dir");
    app.test_rescan_definitions_from(dir.path());

    let ids = app.test_device_registry_ids();
    assert!(
        ids.contains(&"moog-muse".to_string()),
        "bundled definition preserved when user dir is empty"
    );
}

#[test]
fn rescan_user_definition_shadows_bundled_with_same_id() {
    let (mut app, _task) = Resonance::new_for_test();

    // Write a user definition with the same id as the bundled Moog Muse.
    let override_json = r#"{
      "id": "moog-muse",
      "manufacturer": "Test Override",
      "model": "Muse Override",
      "schema_version": 1,
      "params": [],
      "patches": []
    }"#;
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("moog-muse.json");
    std::fs::write(&path, override_json).expect("write override");

    app.test_rescan_definitions_from(dir.path());

    // Only one entry for the id (last-wins).
    let ids = app.test_device_registry_ids();
    assert_eq!(
        ids.iter().filter(|id| id.as_str() == "moog-muse").count(),
        1,
        "last-wins: only one entry for the shared id"
    );
}
