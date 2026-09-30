//! `amp_models.*` — the per-user NAM model library over the control API
//! (nam-model-library.md §9.3), against a temporary library root.
//!
//! The handler reads the same files the amp plugin does, located through
//! `RESONANCE_AMP_MODEL_DIR` / `RESONANCE_LIBRARY_DIR`. Environment
//! variables are process-wide and this binary runs its tests in parallel,
//! so every `amp_models` test shares ONE root, set up once, and runs its
//! phases in sequence inside a single `#[test]`. Nothing else in the
//! binary reads either variable.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_common::library_marks::{self, MarksStore};
use resonance_common::nam_library;
use resonance_control::methods::amp_models::{AmpModelList, AmpModelSource};

use crate::common::call;

/// The shared root: `<tmp>/models` with three models, `<tmp>/marks` for
/// the marks store.
fn roots() -> &'static (PathBuf, PathBuf) {
    static ROOTS: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
    ROOTS.get_or_init(|| {
        let base = std::env::temp_dir().join(format!("resonance-amp-models-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let models = base.join("models");
        let marks = base.join("marks");
        let dl = models.join(nam_library::TONE3000_DIR);
        std::fs::create_dir_all(&dl).unwrap();
        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/resonance-amp/tests/fixtures");
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
        std::env::set_var(nam_library::AMP_MODEL_DIR_ENV, &models);
        std::env::set_var(library_marks::LIBRARY_DIR_ENV, &marks);
        (models, marks)
    })
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
fn amp_models_list_and_marks_need_no_project_and_touch_no_revision() {
    let (_models, marks_dir) = roots();
    // No project open: the library is the user's, not the project's.
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let revision = app.revision();

    // -- amp_models.list -------------------------------------------------
    let all = list(&mut app, serde_json::json!({}));
    assert_eq!(all.total, 3);
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
    assert_eq!(all.models[0].status, "ok");
    assert!(all.library_generation > 0);

    let by_query = list(&mut app, serde_json::json!({ "query": "friedman" }));
    assert_eq!(names(&by_query), vec!["Friedman BE-100 · standard"]);
    let by_source = list(&mut app, serde_json::json!({ "query": "is:tone3000" }));
    assert_eq!(by_source.models.len(), 1);
    let by_type = list(&mut app, serde_json::json!({ "tone_type": "clean" }));
    assert_eq!(names(&by_type), vec!["Test Model", "Test LSTM"]);

    // A favourite set elsewhere (the plugin, another process) sorts first.
    MarksStore::open(marks_dir)
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
