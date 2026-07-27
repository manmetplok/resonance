use std::path::{Path, PathBuf};

use resonance_common::device_definition::{
    DeviceDefinition, DeviceParam, MidiBinding, SCHEMA_VERSION,
};
use resonance_common::device_registry::*;

/// Build a fresh, empty temp directory unique to a test, removing any leftover.
fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("resonance_test_device_registry_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Minimal valid definition with one CC parameter, parameterized by id/model so
/// tests can tell a bundled vs. user definition apart.
fn definition(id: &str, model: &str) -> DeviceDefinition {
    DeviceDefinition {
        id: id.to_string(),
        manufacturer: "Acme".to_string(),
        model: model.to_string(),
        schema_version: SCHEMA_VERSION,
        params: vec![DeviceParam {
            id: "cutoff".to_string(),
            name: "Cutoff".to_string(),
            group: None,
            binding: MidiBinding::Cc { cc: 74 },
            min: 0,
            max: 127,
            default: Some(64),
            curve: Default::default(),
        }],
        patches: vec![],
    }
}

fn write_def(dir: &Path, file: &str, def: &DeviceDefinition) {
    DeviceDefinitionRegistry::save_to_path(def, &dir.join(file)).unwrap();
}

#[test]
fn user_definition_overrides_bundled_by_id() {
    let bundled = fresh_dir("override_bundled");
    let user = fresh_dir("override_user");

    // Same id "synth-x" in both sources; the user one is the shadowing copy.
    write_def(&bundled, "synth-x.json", &definition("synth-x", "Bundled X"));
    write_def(&bundled, "synth-y.json", &definition("synth-y", "Bundled Y"));
    write_def(&user, "synth-x.json", &definition("synth-x", "User X"));

    let reg = DeviceDefinitionRegistry::scan(&bundled, &user);

    // Both ids present, no duplicate for the overridden id.
    assert_eq!(reg.list().len(), 2);
    assert!(reg.errors().is_empty());
    // User definition wins for the shared id; the bundled-only id survives.
    assert_eq!(reg.get("synth-x").unwrap().model, "User X");
    assert_eq!(reg.get("synth-y").unwrap().model, "Bundled Y");
    assert!(reg.get("missing").is_none());

    let _ = std::fs::remove_dir_all(&bundled);
    let _ = std::fs::remove_dir_all(&user);
}

#[test]
fn malformed_file_is_skipped_and_collected() {
    let bundled = fresh_dir("malformed_bundled");
    let user = fresh_dir("malformed_user");

    write_def(&bundled, "good.json", &definition("good", "Good"));
    // Not JSON at all.
    std::fs::write(bundled.join("garbage.json"), b"this is not json").unwrap();
    // Valid JSON but fails validation: max 200 exceeds a CC's 0..=127 range.
    let mut invalid = definition("invalid", "Invalid");
    invalid.params[0].max = 200;
    std::fs::write(
        user.join("invalid.json"),
        invalid.to_json().unwrap().as_bytes(),
    )
    .unwrap();
    // Non-.json files are ignored entirely (not even counted as errors).
    std::fs::write(user.join("notes.txt"), b"ignore me").unwrap();

    let reg = DeviceDefinitionRegistry::scan(&bundled, &user);

    // Only the good definition loads; the two bad .json files are skipped.
    assert_eq!(reg.list().len(), 1);
    assert_eq!(reg.get("good").unwrap().model, "Good");
    assert_eq!(reg.errors().len(), 2);
    let bad: Vec<_> = reg
        .errors()
        .iter()
        .map(|e| e.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(bad.contains(&"garbage.json".to_string()));
    assert!(bad.contains(&"invalid.json".to_string()));

    let _ = std::fs::remove_dir_all(&bundled);
    let _ = std::fs::remove_dir_all(&user);
}

#[test]
fn load_save_round_trip() {
    let dir = fresh_dir("round_trip");
    let path = dir.join("acme-synth.json");

    let def = definition("acme-synth", "Synth One");
    DeviceDefinitionRegistry::save_to_path(&def, &path).unwrap();

    let loaded = DeviceDefinitionRegistry::load_from_path(&path).unwrap();
    assert_eq!(loaded, def);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_creates_parent_dirs_and_refuses_invalid() {
    let dir = fresh_dir("save_parents");
    // Parent subdirectory does not exist yet; save must create it.
    let path = dir.join("nested/deeper/dev.json");

    let def = definition("dev", "Dev");
    DeviceDefinitionRegistry::save_to_path(&def, &path).unwrap();
    assert!(path.exists());

    // An invalid definition is rejected before anything is written.
    let mut bad = definition("bad", "Bad");
    bad.params[0].min = 100;
    bad.params[0].max = 10; // min > max
    let bad_path = dir.join("bad.json");
    assert!(DeviceDefinitionRegistry::save_to_path(&bad, &bad_path).is_err());
    assert!(!bad_path.exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scan_missing_directories_is_empty_not_error() {
    let base = std::env::temp_dir().join("resonance_test_device_registry_absent_xyz");
    let _ = std::fs::remove_dir_all(&base);
    let bundled = base.join("bundled");
    let user = base.join("user");

    let reg = DeviceDefinitionRegistry::scan(&bundled, &user);
    assert!(reg.list().is_empty());
    assert!(reg.errors().is_empty());
}

#[test]
fn user_definitions_dir_is_under_resonance() {
    if let Some(dir) = user_definitions_dir() {
        assert!(dir.ends_with("resonance/device_definitions"));
    }
}
