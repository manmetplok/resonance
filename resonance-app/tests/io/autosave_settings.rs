//! Serialization / default coverage for persisted [`AppSettings`]
//! (autosave config, epic #32 / doc #171). Most of this covers the
//! serde contract the on-disk file depends on: sensible defaults, a
//! lossless round-trip, and forward compatibility when fields or whole
//! sections are absent — without touching the real `config_dir()`.
//!
//! The corrupt-file test below does exercise real disk I/O, but through
//! `settings::load_from` / `persist_to` against a scratch directory, the
//! same `_from`/`_to` escape hatch `registry.rs` and `midi_map.rs` use
//! to keep their own disk-touching tests off the machine's real files.

use resonance_app::settings::{load_from, persist_to, AppSettings, AutosaveSettings};

#[test]
fn autosave_defaults_match_spec() {
    let d = AutosaveSettings::default();
    assert!(d.enabled, "autosave is on by default");
    assert_eq!(d.interval_secs, 30, "default interval is 30 s");
    assert_eq!(d.backup_retention, 10, "default retention keeps 10 backups");
}

#[test]
fn app_settings_default_wraps_autosave_default() {
    assert_eq!(AppSettings::default().autosave, AutosaveSettings::default());
}

#[test]
fn round_trip_preserves_custom_values() {
    let original = AppSettings {
        autosave: AutosaveSettings {
            enabled: false,
            interval_secs: 120,
            backup_retention: 3,
        },
        ..AppSettings::default()
    };
    let json = serde_json::to_string(&original).expect("serialize");
    let restored: AppSettings = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored, original);
}

#[test]
fn empty_object_yields_all_defaults() {
    // An older settings.json (or a hand-cleared one) with no sections
    // must load as the full default document rather than failing.
    let parsed: AppSettings = serde_json::from_str("{}").expect("deserialize empty object");
    assert_eq!(parsed, AppSettings::default());
}

#[test]
fn missing_fields_fall_back_to_defaults() {
    // Forward compatibility: a file written before a field existed (here
    // only `interval_secs` is present) keeps the user's value for that
    // field and defaults the rest, instead of erroring on the gaps.
    let parsed: AppSettings =
        serde_json::from_str(r#"{"autosave":{"interval_secs":45}}"#).expect("deserialize partial");
    assert_eq!(parsed.autosave.interval_secs, 45);
    assert!(parsed.autosave.enabled, "absent `enabled` defaults to true");
    assert_eq!(
        parsed.autosave.backup_retention, 10,
        "absent `backup_retention` defaults to 10"
    );
}

#[test]
fn persist_then_load_round_trips_through_disk() {
    let dir = std::env::temp_dir().join(format!(
        "resonance_test_settings_roundtrip_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let file = dir.join("settings.json");

    let settings = AppSettings {
        autosave: AutosaveSettings {
            enabled: false,
            interval_secs: 45,
            backup_retention: 4,
        },
        ..AppSettings::default()
    };
    persist_to(&file, &settings);
    assert_eq!(load_from(&file), settings);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corrupt_settings_file_loads_default_and_is_quarantined() {
    let dir = std::env::temp_dir().join(format!(
        "resonance_test_settings_corrupt_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("settings.json");

    let original = b"{ this is not valid json";
    std::fs::write(&file, original).unwrap();

    let loaded = load_from(&file);
    assert_eq!(
        loaded,
        AppSettings::default(),
        "a corrupt file loads as the default settings, not a crash"
    );

    // The corrupt file is preserved rather than silently lost to the
    // next save.
    assert!(
        !file.exists(),
        "the corrupt file is moved aside, not left where the loader will find it again"
    );
    let corrupt_path = dir.join("settings.json.corrupt");
    assert!(corrupt_path.exists(), "original bytes preserved as .corrupt");
    assert_eq!(std::fs::read(&corrupt_path).unwrap(), original);

    let _ = std::fs::remove_dir_all(&dir);
}
