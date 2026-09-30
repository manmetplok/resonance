//! `amp_models.*` — the per-user NAM model library over the control API
//! (nam-model-library.md §9.3), against a temporary library root handed
//! to each app with `test_set_amp_library_roots` (no environment variable,
//! so the tests run in parallel).

use std::path::{Path, PathBuf};

use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_common::library_marks::MarksStore;
use resonance_common::nam_library;
use resonance_control::methods::amp_models::{
    AmpModelEntry, AmpModelList, AmpModelSource, AmpModelStatus,
};
use resonance_control::ErrorKind;

use crate::common::call;

/// A fresh root with three models (two A1 captures and an LSTM, the third
/// a Tone3000 download with its sidecar) and an app pointed at it. No
/// project is open: the library is the user's, not the project's.
fn app(tag: &str) -> (Resonance, PathBuf) {
    let base = std::env::temp_dir().join(format!("resonance-amp-models-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let models = base.join("models");
    let dl = models.join(nam_library::TONE3000_DIR);
    std::fs::create_dir_all(&dl).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/resonance-amp/tests/fixtures");
    std::fs::copy(fixtures.join("a1/wavenet.nam"), dl.join("a_darkglass.nam")).unwrap();
    std::fs::copy(fixtures.join("lstm/lstm.nam"), dl.join("b_lstm.nam")).unwrap();
    std::fs::copy(
        fixtures.join("a1/wavenet_a1_standard.nam"),
        dl.join("c_friedman_48121.nam"),
    )
    .unwrap();
    nam_library::write_sidecar(
        &dl.join("c_friedman_48121.nam"),
        &nam_library::Sidecar {
            source: nam_library::SOURCE_TONE3000.into(),
            tone_id: Some(1934),
            model_id: Some(48121),
            tone_title: Some("Friedman BE-100".into()),
            author: Some("jsmith".into()),
            size: Some("standard".into()),
            ..nam_library::Sidecar::default()
        },
    )
    .unwrap();
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_amp_library_roots(models, base.join("marks"));
    (app, base.join("marks"))
}

fn list(app: &mut Resonance, params: serde_json::Value) -> AmpModelList {
    call(app, "amp_models.list", params)
        .result()
        .expect("amp_models.list succeeds")
}

fn names(list: &AmpModelList) -> Vec<&str> {
    list.models.iter().map(|m| m.name.as_str()).collect()
}

#[test]
fn a_test_app_never_points_at_the_users_library() {
    let (app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let roots = app.test_amp_library_roots();
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    for root in [roots.models.unwrap(), roots.marks.unwrap()] {
        assert!(root.starts_with(std::env::temp_dir()), "{} is not a temp dir", root.display());
        assert!(home.as_os_str().is_empty() || !root.starts_with(home.join(".local")));
    }
}

#[test]
fn amp_models_list_reports_filters_and_limits() {
    let (mut app, marks_dir) = app("list");
    let revision = app.revision();

    let all = list(&mut app, serde_json::json!({}));
    assert_eq!(all.total, 3);
    assert_eq!(all.matched, 3);
    assert_eq!(
        names(&all),
        vec!["Test Model", "Test LSTM", "Friedman BE-100 · standard"],
        "slot order: the downloads in today's sorted order"
    );
    let friedman = &all.models[2];
    assert_eq!(friedman.slot, Some(2));
    assert_eq!(friedman.author.as_deref(), Some("jsmith"));
    assert_eq!(
        friedman.source,
        AmpModelSource::Tone3000 {
            tone_id: 1934,
            model_id: 48121
        }
    );
    assert_eq!(friedman.id.len(), 64, "a sha256 content id");
    assert_eq!(all.models[0].gear_type.as_deref(), Some("amp"));
    assert_eq!(all.models[1].architecture, "LSTM");
    assert_eq!(all.models[0].status, AmpModelStatus::Ok);
    assert!(all.library_generation > 0);

    let by_query = list(&mut app, serde_json::json!({ "query": "friedman" }));
    assert_eq!(names(&by_query), vec!["Friedman BE-100 · standard"]);
    let by_source = list(&mut app, serde_json::json!({ "query": "is:tone3000" }));
    assert_eq!(by_source.models.len(), 1);
    let by_arch = list(&mut app, serde_json::json!({ "query": "architecture:lstm" }));
    assert_eq!(names(&by_arch), vec!["Test LSTM"], "the plugin's Arch facet, shared rows");
    let by_type = list(&mut app, serde_json::json!({ "tone_type": "clean" }));
    assert_eq!(names(&by_type), vec!["Test Model", "Test LSTM"]);
    let limited = list(&mut app, serde_json::json!({ "limit": 1 }));
    assert_eq!(limited.models.len(), 1);
    assert_eq!(limited.matched, 3, "matched counts past the limit");

    // A favourite set elsewhere (the plugin, another process) sorts first.
    MarksStore::open(&marks_dir)
        .unwrap()
        .set_favorite(&nam_library::mark_key(&friedman.id), true)
        .unwrap();
    let starred = list(&mut app, serde_json::json!({}));
    assert_eq!(starred.models[0].name, "Friedman BE-100 · standard");
    assert!(starred.models[0].favorite);
    let only = list(&mut app, serde_json::json!({ "favorites_only": true }));
    assert_eq!(only.models.len(), 1);
    assert_eq!(app.revision(), revision, "reading the library is not a project edit");
}

#[test]
fn a_model_added_after_the_first_list_shows_up_in_the_next() {
    let (mut app, _) = app("later");
    assert_eq!(list(&mut app, serde_json::json!({})).total, 3);
    let roots = app.test_amp_library_roots();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/resonance-amp/tests/fixtures");
    std::fs::copy(
        fixtures.join("a2/wavenet_a2_max.nam"),
        roots.models.unwrap().join("tone3000/d_new.nam"),
    )
    .unwrap();
    let again = list(&mut app, serde_json::json!({}));
    assert_eq!(again.total, 4);
    assert_eq!(again.models.last().unwrap().slot, Some(3), "appended, nothing else moved");
}

#[test]
fn amp_models_set_marks_writes_the_shared_store_only() {
    let (mut app, marks_dir) = app("marks");
    let revision = app.revision();
    let all = list(&mut app, serde_json::json!({}));
    let lstm_id = all.models[1].id.clone();
    let updated: AmpModelEntry = call(
        &mut app,
        "amp_models.set_marks",
        serde_json::json!({ "id": &lstm_id[..10], "favorite": true, "tags": ["Clean Lead", "lstm"] }),
    )
    .result()
    .expect("a unique id prefix addresses the model");
    assert_eq!(updated.id, lstm_id);
    assert!(updated.favorite);
    assert_eq!(updated.tags, vec!["clean-lead", "lstm"], "tags are normalised");
    // The plugin's store sees it: same file, same key.
    let store = MarksStore::open(&marks_dir).unwrap();
    let marks = store.marks(&nam_library::mark_key(&lstm_id));
    assert!(marks.favorite);
    assert_eq!(marks.tags, vec!["clean-lead", "lstm"]);
    assert_eq!(
        names(&list(&mut app, serde_json::json!({ "query": "tag:clean-lead" }))),
        vec!["Test LSTM"]
    );

    // Tags replace; favorite alone leaves tags alone.
    let cleared: AmpModelEntry = call(
        &mut app,
        "amp_models.set_marks",
        serde_json::json!({ "id": lstm_id, "tags": [] }),
    )
    .result()
    .unwrap();
    assert!(cleared.tags.is_empty());
    assert!(cleared.favorite, "not mentioned, not changed");

    let error = call(&mut app, "amp_models.set_marks", serde_json::json!({ "id": lstm_id }))
        .error
        .expect("nothing to set is refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    let error = call(
        &mut app,
        "amp_models.set_marks",
        serde_json::json!({ "id": "ffffffffffff", "favorite": true }),
    )
    .error
    .expect("an unknown id is refused");
    assert_eq!(error.kind(), ErrorKind::NotFound);

    assert_eq!(
        app.revision(),
        revision,
        "the library and its marks are the user's, not the project's: no revision bump \
         (and so no undo entry, which is one per revision)"
    );
}

/// Resonance Amp on a track, with its model selector as the engine reports
/// it (no `choices`: 1000 steps is past the choice walk).
fn with_amp_on_a_track(app: &mut Resonance) {
    use resonance_audio::types::{AudioEvent, ParamInfo, ScannedPlugin, TrackType};
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/tmp/control-amp-models.rprj"));
    app.test_add_track(1, TrackType::Audio);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![ScannedPlugin {
            clap_file_path: "/plugins/amp.clap".to_owned(),
            clap_plugin_id: "com.resonance.amp".to_owned(),
            name: "Resonance Amp".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: false,
            ..Default::default()
        }],
    });
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: 1,
        instance_id: 30,
        plugin_name: "Resonance Amp".to_owned(),
        clap_plugin_id: "com.resonance.amp".to_owned(),
        clap_file_path: "/plugins/amp.clap".to_owned(),
        params: vec![ParamInfo {
            id: resonance_plugin::stable_hash("file_select"),
            name: "Model Select".to_owned(),
            min_value: 0.0,
            max_value: 999.0,
            stepped: true,
            text: "Test Model".to_owned(),
            ..Default::default()
        }],
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
}

#[test]
fn a_model_name_on_the_amps_selector_resolves_app_side() {
    use resonance_audio::types::AudioCommand;
    let (mut app, _) = app("label");
    with_amp_on_a_track(&mut app);
    let rx = app.test_capture_engine();
    let set = |app: &mut Resonance, value: &str| {
        call(
            app,
            "track.set_plugin_param",
            serde_json::json!({
                "track_id": 1, "plugin_id": "com.resonance.amp",
                "param": "Model Select", "value": value,
            }),
        )
    };
    let ack = set(&mut app, "test lstm");
    assert!(ack.error.is_none(), "{:?}", ack.error);
    let cmds: Vec<AudioCommand> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ResolvePluginParamText { .. })),
        "no round trip through the plugin for the amp's own library"
    );
    let value = cmds.iter().find_map(|c| match c {
        AudioCommand::SetPluginParam { value, .. } => Some(*value),
        _ => None,
    });
    assert_eq!(value, Some(1.0), "Test LSTM is slot 1");

    let error = set(&mut app, "no such amp").error.expect("an unknown model is refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(error.message.contains("amp_models_list"), "{}", error.message);
}
