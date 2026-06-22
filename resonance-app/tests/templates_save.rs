//! Save-As-Template write path (todo #666, impl-plan doc #197).
//! Separate file as required — no inline #[cfg(test)] modules.
//!
//! These pin the `write_template` capture path end-to-end against a
//! controlled templates root (a tempdir): the folder it produces is listed
//! by `scan_user_templates`' classifier with the right summary, loads back
//! through the normal project loader, honours the two capture toggles, and
//! never clobbers an earlier same-named template.

use resonance_app::project::{
    load_project, ProjectBus, ProjectFile, ProjectPlugin, ProjectTrack, PROJECT_FORMAT_VERSION,
};
use resonance_app::state::{InstrumentIcon, InstrumentType, SignatureEvent, TempoEvent};
use resonance_app::update::project_io::{
    scan_templates_in, write_template, TemplateCaptureOptions, TemplateEntry,
};

/// A project carrying a track plugin, a bus plugin, a master plugin, and a
/// non-trivial tempo map — enough to exercise every toggle and summary
/// field.
fn project_with_content() -> ProjectFile {
    let mut project = ProjectFile {
        version: PROJECT_FORMAT_VERSION,
        bpm: 128.0,
        time_sig_num: 3,
        time_sig_den: 4,
        ..Default::default()
    };

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
            plugin_name: "Track Plugin".to_string(),
            clap_plugin_id: "track.id".to_string(),
            clap_file_path: "track.clap".to_string(),
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

    project.master_plugins = vec![ProjectPlugin {
        instance_id: 3,
        plugin_name: "Master Plugin".to_string(),
        clap_plugin_id: "master.id".to_string(),
        clap_file_path: "master.clap".to_string(),
        state_file: "plugins/plugin_3.bin".to_string(),
    }];

    project.tempo_events = vec![
        TempoEvent { bar: 0, bpm: 128.0 },
        TempoEvent { bar: 8, bpm: 140.0 },
    ];
    project.signature_events = vec![SignatureEvent {
        bar: 4,
        numerator: 6,
        denominator: 8,
    }];

    project
}

/// State blobs for the three plugins above, so the write path has files to
/// drop into `plugins/`.
fn plugin_states() -> Vec<(u64, Vec<u8>)> {
    vec![(1, vec![1, 1, 1]), (2, vec![2, 2]), (3, vec![3])]
}

#[test]
fn write_template_produces_scannable_folder_with_summary() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = write_template(
        tmp.path(),
        "My Song Starter",
        "A nice starting point",
        project_with_content(),
        &plugin_states(),
        &[],
        TemplateCaptureOptions::capture_all(),
        1_700_000_000,
    )
    .unwrap();

    // The folder name is a slug of the template name.
    assert_eq!(folder.file_name().unwrap(), "my-song-starter");
    assert!(folder.join("project.json").exists());
    assert!(folder.join("template.json").exists());

    // The scanner lists it as a single valid template with the metadata and
    // a summary computed from the captured project.
    let entries = scan_templates_in(tmp.path());
    assert_eq!(entries.len(), 1);
    let template = match &entries[0] {
        TemplateEntry::Valid(t) => t,
        TemplateEntry::Stale(s) => panic!("expected valid template, got stale: {s:?}"),
    };
    assert_eq!(template.name, "My Song Starter");
    assert_eq!(template.description, "A nice starting point");
    assert_eq!(template.schema_version, PROJECT_FORMAT_VERSION);
    assert_eq!(template.created_secs, Some(1_700_000_000));
    assert_eq!(template.summary.track_count, 1);
    assert_eq!(template.summary.bus_count, 1);
    // track plugin + bus plugin + master plugin
    assert_eq!(template.summary.plugin_count, 3);
    assert_eq!(template.summary.tempo_bpm, 128.0);
    assert_eq!(template.summary.time_sig, "3/4");
}

#[test]
fn write_template_round_trips_through_load_project() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = write_template(
        tmp.path(),
        "Roundtrip",
        "",
        project_with_content(),
        &plugin_states(),
        &[],
        TemplateCaptureOptions::capture_all(),
        1_700_000_000,
    )
    .unwrap();

    // The template folder uses the normal project shape, so the standard
    // loader reads it back without complaint.
    let loaded = load_project(&folder).expect("template folder loads as a project");
    assert_eq!(loaded.file.bpm, 128.0);
    assert_eq!(loaded.file.tracks.len(), 1);
    assert_eq!(loaded.file.busses.len(), 1);
    assert_eq!(loaded.file.master_plugins.len(), 1);
    // The three referenced plugin-state blobs were written and read back.
    assert_eq!(loaded.plugin_states.len(), 3);
}

#[test]
fn excluding_tempo_map_clears_events_but_keeps_base_tempo() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = write_template(
        tmp.path(),
        "No Tempo Map",
        "",
        project_with_content(),
        &plugin_states(),
        &[],
        TemplateCaptureOptions {
            include_markers_and_tempo: false,
            include_master_chain: true,
        },
        0,
    )
    .unwrap();

    let loaded = load_project(&folder).unwrap();
    assert!(loaded.file.tempo_events.is_empty());
    assert!(loaded.file.signature_events.is_empty());
    // The flat project BPM / signature still carry over — only the *change*
    // events were stripped.
    assert_eq!(loaded.file.bpm, 128.0);
    assert_eq!(loaded.file.time_sig_num, 3);
}

#[test]
fn excluding_master_chain_clears_master_plugins_and_summary() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = write_template(
        tmp.path(),
        "No Master Chain",
        "",
        project_with_content(),
        &plugin_states(),
        &[],
        TemplateCaptureOptions {
            include_markers_and_tempo: true,
            include_master_chain: false,
        },
        0,
    )
    .unwrap();

    let loaded = load_project(&folder).unwrap();
    assert!(loaded.file.master_plugins.is_empty());
    assert!(!loaded.file.master_fx_bypassed);
    // The dropped master plugin's state blob is not written, leaving only
    // the track + bus plugin states.
    assert_eq!(loaded.plugin_states.len(), 2);

    // The sidecar summary reflects the post-toggle project: 2 plugins, not 3.
    let entries = scan_templates_in(tmp.path());
    let template = match &entries[0] {
        TemplateEntry::Valid(t) => t,
        other => panic!("expected valid template, got {other:?}"),
    };
    assert_eq!(template.summary.plugin_count, 2);
}

#[test]
fn same_name_templates_get_distinct_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let opts = TemplateCaptureOptions::capture_all();

    let first = write_template(
        tmp.path(),
        "Starter",
        "",
        project_with_content(),
        &[],
        &[],
        opts,
        0,
    )
    .unwrap();
    let second = write_template(
        tmp.path(),
        "Starter",
        "",
        project_with_content(),
        &[],
        &[],
        opts,
        0,
    )
    .unwrap();

    assert_ne!(first, second);
    assert_eq!(first.file_name().unwrap(), "starter");
    assert_eq!(second.file_name().unwrap(), "starter-2");
    // Both survive on disk and both are scanned.
    assert!(first.exists() && second.exists());
    assert_eq!(scan_templates_in(tmp.path()).len(), 2);
}

#[test]
fn punctuation_only_name_falls_back_to_template_slug() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = write_template(
        tmp.path(),
        "!!! ???",
        "",
        ProjectFile::default(),
        &[],
        &[],
        TemplateCaptureOptions::capture_all(),
        0,
    )
    .unwrap();
    assert_eq!(folder.file_name().unwrap(), "template");
}
