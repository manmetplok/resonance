//! Serialization / default coverage for persisted [`AppSettings`]
//! (autosave config, epic #32 / doc #171). Disk I/O (`settings::load`
//! / `settings::persist`) touches the real `config_dir()` and is not
//! exercised here — these tests pin the serde contract the on-disk file
//! depends on: sensible defaults, a lossless round-trip, and forward
//! compatibility when fields or whole sections are absent.

use resonance_app::settings::{AppSettings, AutosaveSettings};

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
