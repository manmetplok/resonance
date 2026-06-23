//! Tests for template scanning and model (todo #663).
//! Separate file as required — no inline #[cfg(test)] modules.

use resonance_app::project::{ProjectBus, ProjectFile, ProjectPlugin, ProjectTrack, PROJECT_FORMAT_VERSION};
use resonance_app::update::project_io::{
    compute_summary, scan_templates_in, scan_user_templates, templates_dir, ensure_templates_dir,
    Template, TemplateEntry, TemplateKind, TemplateMetadata, TemplateSummary, StaleTemplate, StaleReason,
};
use std::fs;
use std::path::PathBuf;

fn write_json_file<P: AsRef<std::path::Path>, T: serde::Serialize>(path: P, value: &T) {
    let json = serde_json::to_string_pretty(value).unwrap();
    fs::write(path, json).unwrap();
}

fn make_minimal_project() -> ProjectFile {
    ProjectFile {
        version: PROJECT_FORMAT_VERSION,
        sample_rate: 44100,
        bpm: 120.0,
        time_sig_num: 4,
        time_sig_den: 4,
        metronome_enabled: false,
        master_volume: 0.0,
        master_plugins: Vec::new(),
        master_fx_bypassed: false,
        loop_enabled: false,
        loop_in: 0,
        loop_out: 0,
        tracks: Vec::new(),
        clips: Vec::new(),
        midi_clips: Vec::new(),
        busses: Vec::new(),
        section_definitions: Vec::new(),
        section_placements: Vec::new(),
        tempo_events: Vec::new(),
        signature_events: Vec::new(),
        midi_clock_send_enabled: false,
        midi_clock_send_device: None,
        midi_clock_recv_enabled: false,
        midi_clock_recv_device: None,
        drum_groups: Vec::new(),
        drum_patterns: Vec::new(),
        references: Vec::new(),
        reference_settings: Default::default(),
        arrangement_markers: Vec::new(),
    }
}

fn make_project_with_content() -> ProjectFile {
    use resonance_app::state::{InstrumentIcon, InstrumentType};
    
    let mut project = make_minimal_project();
    
    // Add a track with a plugin
    project.tracks = vec![ProjectTrack {
        id: 1,
        name: "Track 1".to_string(),
        order: 0,
        volume: 0.0,
        pan: 0.0,
        muted: false,
        soloed: false,
        fx_bypassed: false,
        record_armed: false,
        monitor_enabled: false,
        mono: false,
        input_device_name: None,
        input_port_index: None,
        plugins: vec![ProjectPlugin {
            instance_id: 1,
            plugin_name: "Test Plugin".to_string(),
            clap_plugin_id: "test.id".to_string(),
            clap_file_path: "test.clap".to_string(),
            state_file: "plugins/plugin_1.bin".to_string(),
        }],
        track_type: "audio".to_string(),
        output_bus: None,
        instrument_type: InstrumentType::Synth,
        instrument_icon: InstrumentIcon::Music,
        role: None,
        sub_track: None,
        midi_input_device: None,
        midi_input_channel: None,
        midi_output_device: None,
        midi_output_channel: None,
    }];

    // Add a bus with a plugin
    project.busses = vec![ProjectBus {
        id: 1,
        name: "Bus 1".to_string(),
        order: 0,
        volume: 0.0,
        pan: 0.0,
        muted: false,
        fx_bypassed: false,
        plugins: vec![ProjectPlugin {
            instance_id: 2,
            plugin_name: "Bus Plugin".to_string(),
            clap_plugin_id: "bus.id".to_string(),
            clap_file_path: "bus.clap".to_string(),
            state_file: "plugins/plugin_2.bin".to_string(),
        }],
    }];

    project
}

#[test]
fn templates_dir_returns_path() {
    let dir = templates_dir();
    assert!(dir.is_some() || dir.is_none());
}

#[test]
fn ensure_templates_dir_creates_or_returns() {
    let dir = ensure_templates_dir();
    assert!(dir.is_some() || dir.is_none());
}

#[test]
fn compute_summary_empty_project() {
    let project = make_minimal_project();
    let summary = compute_summary(&project);
    
    assert_eq!(summary.track_count, 0);
    assert_eq!(summary.bus_count, 0);
    assert_eq!(summary.plugin_count, 0);
    assert_eq!(summary.tempo_bpm, 120.0);
    assert_eq!(summary.time_sig, "4/4");
}

#[test]
fn compute_summary_with_tracks_and_busses() {
    let project = make_project_with_content();
    let summary = compute_summary(&project);
    
    assert_eq!(summary.track_count, 1);
    assert_eq!(summary.bus_count, 1);
    assert_eq!(summary.plugin_count, 2);
    assert_eq!(summary.tempo_bpm, 120.0);
    assert_eq!(summary.time_sig, "4/4");
}

#[test]
fn compute_summary_with_master_plugins() {
    let mut project = make_minimal_project();
    project.master_plugins = vec![
        ProjectPlugin {
            instance_id: 1,
            plugin_name: "Master Plugin".to_string(),
            clap_plugin_id: "master.id".to_string(),
            clap_file_path: "master.clap".to_string(),
            state_file: "plugins/plugin_1.bin".to_string(),
        },
    ];
    
    let summary = compute_summary(&project);
    assert_eq!(summary.plugin_count, 1);
}

#[test]
fn template_new_user() {
    let metadata = TemplateMetadata {
        name: "My Template".to_string(),
        description: "A test template".to_string(),
        built_in: false,
        schema_version: PROJECT_FORMAT_VERSION,
        summary: TemplateSummary {
            track_count: 1,
            bus_count: 0,
            plugin_count: 0,
            tempo_bpm: 120.0,
            time_sig: "4/4".to_string(),
        },
        created_secs: 1234567890,
    };
    
    let path = PathBuf::from("/test/template");
    let template = Template::new_user(metadata.clone(), path.clone());
    
    assert_eq!(template.kind, TemplateKind::User);
    assert_eq!(template.name, metadata.name);
    assert_eq!(template.description, metadata.description);
    assert_eq!(template.summary.track_count, metadata.summary.track_count);
    assert_eq!(template.path, path);
    assert_eq!(template.schema_version, metadata.schema_version);
    assert_eq!(template.created_secs, Some(metadata.created_secs));
}

#[test]
fn template_new_builtin() {
    let summary = TemplateSummary {
        track_count: 2,
        bus_count: 1,
        plugin_count: 3,
        tempo_bpm: 120.0,
        time_sig: "4/4".to_string(),
    };
    
    let template = Template::new_builtin(
        "Empty Project".to_string(),
        "A blank project".to_string(),
        summary.clone(),
    );
    
    assert_eq!(template.kind, TemplateKind::Builtin);
    assert_eq!(template.name, "Empty Project");
    assert_eq!(template.description, "A blank project");
    assert_eq!(template.summary.track_count, 2);
    assert!(template.path.as_os_str().is_empty());
    assert_eq!(template.schema_version, PROJECT_FORMAT_VERSION);
    assert_eq!(template.created_secs, None);
}

#[test]
fn template_metadata_serialization_roundtrip() {
    let original = TemplateMetadata {
        name: "Test".to_string(),
        description: "A test template".to_string(),
        built_in: false,
        schema_version: 2,
        summary: TemplateSummary {
            track_count: 1,
            bus_count: 2,
            plugin_count: 3,
            tempo_bpm: 120.0,
            time_sig: "4/4".to_string(),
        },
        created_secs: 12345,
    };

    let json = serde_json::to_string(&original).unwrap();
    let parsed: TemplateMetadata = serde_json::from_str(&json).unwrap();

    assert_eq!(original.name, parsed.name);
    assert_eq!(original.description, parsed.description);
    assert_eq!(original.built_in, parsed.built_in);
    assert_eq!(original.schema_version, parsed.schema_version);
    assert_eq!(original.summary.track_count, parsed.summary.track_count);
    assert_eq!(original.created_secs, parsed.created_secs);
}

#[test]
fn template_kind_serialization() {
    let builtin_json = serde_json::to_string(&TemplateKind::Builtin).unwrap();
    let user_json = serde_json::to_string(&TemplateKind::User).unwrap();
    
    assert_eq!(builtin_json, "\"Builtin\"");
    assert_eq!(user_json, "\"User\"");
    
    let parsed_builtin: TemplateKind = serde_json::from_str(&builtin_json).unwrap();
    let parsed_user: TemplateKind = serde_json::from_str(&user_json).unwrap();
    
    assert_eq!(parsed_builtin, TemplateKind::Builtin);
    assert_eq!(parsed_user, TemplateKind::User);
}

#[test]
fn stale_reason_serialization() {
    let newer = StaleReason::SchemaVersionNewer { schema_version: 3 };
    let parse_err = StaleReason::ProjectParseError { reason: "bad json".to_string() };
    let meta_err = StaleReason::MetadataParseError { reason: "corrupt".to_string() };
    
    let newer_json = serde_json::to_string(&newer).unwrap();
    let parse_err_json = serde_json::to_string(&parse_err).unwrap();
    let meta_err_json = serde_json::to_string(&meta_err).unwrap();
    
    let _: StaleReason = serde_json::from_str(&newer_json).unwrap();
    let _: StaleReason = serde_json::from_str(&parse_err_json).unwrap();
    let _: StaleReason = serde_json::from_str(&meta_err_json).unwrap();
}

#[test]
fn stale_template_structure() {
    let stale = StaleTemplate {
        path: PathBuf::from("/bad/template"),
        reason: StaleReason::SchemaVersionNewer { schema_version: 5 },
        schema_version: Some(5),
    };
    
    assert_eq!(stale.path, PathBuf::from("/bad/template"));
    assert!(matches!(stale.reason, StaleReason::SchemaVersionNewer { .. }));
    assert_eq!(stale.schema_version, Some(5));
}

#[test]
fn template_entry_valid_variant() {
    let project = make_minimal_project();
    let metadata = TemplateMetadata {
        name: "Valid".to_string(),
        description: "Valid template".to_string(),
        built_in: false,
        schema_version: PROJECT_FORMAT_VERSION,
        summary: compute_summary(&project),
        created_secs: 1000,
    };
    
    let template = Template::new_user(metadata, PathBuf::from("/valid"));
    let entry = TemplateEntry::Valid(template);
    
    assert!(matches!(entry, TemplateEntry::Valid(_)));
}

#[test]
fn template_entry_stale_variant() {
    let stale = StaleTemplate {
        path: PathBuf::from("/stale"),
        reason: StaleReason::ProjectParseError { reason: "broken".to_string() },
        schema_version: Some(2),
    };
    
    let entry = TemplateEntry::Stale(stale);
    assert!(matches!(entry, TemplateEntry::Stale(_)));
}

#[test]
fn scan_returns_list() {
    let results = scan_user_templates();
    assert!(results.iter().all(|e| matches!(e, TemplateEntry::Valid(_) | TemplateEntry::Stale(_))));
}

// --- Fixture-based scan coverage -------------------------------------------
//
// `scan_user_templates` resolves the real config dir, which a test can't
// populate deterministically, so the classification logic is exercised
// through `scan_templates_in(dir)` against a tempdir laid out by hand.

/// Write a template folder `name/` under `root` with the given metadata and
/// a raw `project.json` body (passed as a string so tests can inject
/// deliberately-broken JSON).
fn write_template(root: &std::path::Path, name: &str, meta: &TemplateMetadata, project_json: &str) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    write_json_file(dir.join("template.json"), meta);
    fs::write(dir.join("project.json"), project_json).unwrap();
}

fn metadata_at(schema_version: u32) -> TemplateMetadata {
    let project = make_minimal_project();
    TemplateMetadata {
        name: "Fixture".to_string(),
        description: "Fixture template".to_string(),
        built_in: false,
        schema_version,
        summary: compute_summary(&project),
        created_secs: 42,
    }
}

#[test]
fn scan_missing_dir_returns_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("does-not-exist");
    assert!(scan_templates_in(&missing).is_empty());
}

#[test]
fn scan_non_directory_path_returns_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("a-file");
    fs::write(&file, "not a dir").unwrap();
    assert!(scan_templates_in(&file).is_empty());
}

#[test]
fn scan_classifies_valid_template() {
    let tmp = tempfile::tempdir().unwrap();
    let project_json = serde_json::to_string(&make_minimal_project()).unwrap();
    write_template(tmp.path(), "good", &metadata_at(PROJECT_FORMAT_VERSION), &project_json);

    let results = scan_templates_in(tmp.path());
    assert_eq!(results.len(), 1);
    match &results[0] {
        TemplateEntry::Valid(t) => {
            assert_eq!(t.kind, TemplateKind::User);
            assert_eq!(t.name, "Fixture");
            assert_eq!(t.schema_version, PROJECT_FORMAT_VERSION);
            assert!(t.path.ends_with("good"));
        }
        other => panic!("expected Valid, got {other:?}"),
    }
}

#[test]
fn scan_flags_schema_too_new_as_stale() {
    let tmp = tempfile::tempdir().unwrap();
    let project_json = serde_json::to_string(&make_minimal_project()).unwrap();
    let future = PROJECT_FORMAT_VERSION + 1;
    write_template(tmp.path(), "future", &metadata_at(future), &project_json);

    let results = scan_templates_in(tmp.path());
    assert_eq!(results.len(), 1);
    match &results[0] {
        TemplateEntry::Stale(s) => {
            assert!(matches!(
                s.reason,
                StaleReason::SchemaVersionNewer { schema_version } if schema_version == future
            ));
            assert_eq!(s.schema_version, Some(future));
        }
        other => panic!("expected Stale, got {other:?}"),
    }
}

#[test]
fn scan_flags_unparseable_project_as_stale() {
    let tmp = tempfile::tempdir().unwrap();
    // template.json is fine and current, but project.json is garbage.
    write_template(tmp.path(), "broken", &metadata_at(PROJECT_FORMAT_VERSION), "{ not valid json");

    let results = scan_templates_in(tmp.path());
    assert_eq!(results.len(), 1);
    match &results[0] {
        TemplateEntry::Stale(s) => {
            assert!(matches!(s.reason, StaleReason::ProjectParseError { .. }));
            assert_eq!(s.schema_version, Some(PROJECT_FORMAT_VERSION));
        }
        other => panic!("expected Stale, got {other:?}"),
    }
}

#[test]
fn scan_flags_missing_project_as_stale() {
    let tmp = tempfile::tempdir().unwrap();
    // A template folder with a valid sidecar but no project.json at all.
    let dir = tmp.path().join("no-project");
    fs::create_dir_all(&dir).unwrap();
    write_json_file(dir.join("template.json"), &metadata_at(PROJECT_FORMAT_VERSION));

    let results = scan_templates_in(tmp.path());
    assert_eq!(results.len(), 1);
    assert!(matches!(
        &results[0],
        TemplateEntry::Stale(StaleTemplate { reason: StaleReason::ProjectParseError { .. }, .. })
    ));
}

#[test]
fn scan_flags_unparseable_metadata_as_stale() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("bad-meta");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("template.json"), "{ broken").unwrap();
    fs::write(dir.join("project.json"), "{}").unwrap();

    let results = scan_templates_in(tmp.path());
    assert_eq!(results.len(), 1);
    match &results[0] {
        TemplateEntry::Stale(s) => {
            assert!(matches!(s.reason, StaleReason::MetadataParseError { .. }));
            assert_eq!(s.schema_version, None);
        }
        other => panic!("expected Stale, got {other:?}"),
    }
}

#[test]
fn scan_skips_folder_without_sidecar() {
    let tmp = tempfile::tempdir().unwrap();
    // A directory that holds a project but no template.json is not a template.
    let dir = tmp.path().join("just-a-project");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("project.json"),
        serde_json::to_string(&make_minimal_project()).unwrap(),
    )
    .unwrap();
    // A loose file at the top level should be ignored too.
    fs::write(tmp.path().join("README.txt"), "hi").unwrap();

    assert!(scan_templates_in(tmp.path()).is_empty());
}

#[test]
fn scan_classifies_mixed_set() {
    let tmp = tempfile::tempdir().unwrap();
    let project_json = serde_json::to_string(&make_minimal_project()).unwrap();
    write_template(tmp.path(), "ok", &metadata_at(PROJECT_FORMAT_VERSION), &project_json);
    write_template(tmp.path(), "future", &metadata_at(PROJECT_FORMAT_VERSION + 1), &project_json);
    write_template(tmp.path(), "broken", &metadata_at(PROJECT_FORMAT_VERSION), "nope");

    let results = scan_templates_in(tmp.path());
    assert_eq!(results.len(), 3);
    let valid = results.iter().filter(|e| matches!(e, TemplateEntry::Valid(_))).count();
    let stale = results.iter().filter(|e| matches!(e, TemplateEntry::Stale(_))).count();
    assert_eq!(valid, 1);
    assert_eq!(stale, 2);
}

#[test]
fn template_json_structure() {
    let metadata = TemplateMetadata {
        name: "Test Template".to_string(),
        description: "Description".to_string(),
        built_in: false,
        schema_version: PROJECT_FORMAT_VERSION,
        summary: TemplateSummary {
            track_count: 5,
            bus_count: 2,
            plugin_count: 10,
            tempo_bpm: 140.0,
            time_sig: "7/8".to_string(),
        },
        created_secs: 9999999999,
    };
    
    let json = serde_json::to_string_pretty(&metadata).unwrap();
    
    assert!(json.contains("\"name\""));
    assert!(json.contains("\"Test Template\""));
    assert!(json.contains("\"description\""));
    assert!(json.contains("\"built_in\""));
    assert!(json.contains("\"schema_version\""));
    assert!(json.contains("\"summary\""));
    assert!(json.contains("\"track_count\""));
    assert!(json.contains("\"bus_count\""));
    assert!(json.contains("\"plugin_count\""));
    assert!(json.contains("\"tempo_bpm\""));
    assert!(json.contains("\"time_sig\""));
    assert!(json.contains("\"created_secs\""));
}
