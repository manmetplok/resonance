//! The amp over the model library (nam-model-library.md §5, §11): how a
//! saved reference resolves (path, moved and relinked, missing with the
//! reference kept verbatim, hash mismatch), state v2 with v1 load, stable
//! slots through `file_select` (its text both ways, an empty slot never
//! loads), and the editor-side library actions that need no window.
//!
//! Every test builds its own library root and hands it to the instance
//! through `ResonanceAmp::with_library`, so nothing reads the user's data
//! dir or a process-global env var, and the tests run in parallel.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use resonance_amp::library::{shared_for, SharedLibrary};
use resonance_amp::library_rows::{step_in_view, view_counter, ModelRows};
use resonance_plugin::library_view::{BrowserModel, Sort, SortKey};
use resonance_amp::model_ref::{resolve_model, ModelRef, ModelState, Resolved};
use resonance_amp::ResonanceAmp;
use resonance_common::nam_library::{self, Library, Source};
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
use serde_json::{json, Value};

const SAMPLE_RATE: f32 = 48_000.0;
const BLOCK: usize = 256;
const FILE_SELECT: usize = 0;
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("resonance-amp-lib-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("tone3000")).unwrap();
    dir
}

/// A root with the two A1 fixtures as downloads: `a.nam` (slot 0, the
/// small `wavenet.nam`) and `b.nam` (slot 1, `wavenet_a1_standard.nam`).
fn seeded_root(tag: &str) -> (PathBuf, Arc<SharedLibrary>) {
    let root = temp_root(tag);
    std::fs::copy(fixture("a1/wavenet.nam"), root.join("tone3000/a.nam")).unwrap();
    std::fs::copy(fixture("a1/wavenet_a1_standard.nam"), root.join("tone3000/b.nam")).unwrap();
    let lib = shared_for(Some(root.clone()));
    lib.rescan().unwrap();
    (root, lib)
}

fn hash(p: &Path) -> Option<String> {
    nam_library::hash_file(p).ok()
}

fn run_blocks(plugin: &mut ResonanceAmp, blocks: usize) {
    let mut left = vec![0.0_f32; BLOCK];
    let mut right = vec![0.0_f32; BLOCK];
    for _ in 0..blocks {
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, BLOCK, &mut ev, None);
    }
}

fn state_of(plugin: &ResonanceAmp) -> Value {
    serde_json::from_slice(&plugin.save_state()).unwrap()
}

/// The non-param, non-preset keys of a saved state: the model reference.
fn model_keys(state: &Value) -> serde_json::Map<String, Value> {
    state
        .as_object()
        .unwrap()
        .iter()
        .filter(|(k, _)| k.starts_with("model_"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn pump_until(plugin: &mut ResonanceAmp, done: impl Fn(&ResonanceAmp) -> bool) -> bool {
    let deadline = Instant::now() + LOAD_TIMEOUT;
    while Instant::now() < deadline {
        run_blocks(plugin, 4);
        if done(plugin) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

// ---------------------------------------------------------------------------
// resolve_model
// ---------------------------------------------------------------------------

#[test]
fn a_path_that_exists_loads_and_finds_its_entry() {
    let (root, lib) = seeded_root("resolve-ok");
    let lib = lib.read();
    let b = root.join("tone3000/b.nam");
    let v1 = ModelRef {
        path: b.to_string_lossy().into_owned(),
        ..ModelRef::default()
    };
    match resolve_model(&v1, &lib, hash) {
        Resolved::Load { path, entry } => {
            assert_eq!(path, b);
            assert_eq!(entry.unwrap().slot, Some(1));
        }
        other => panic!("expected Load, got {other:?}"),
    }
    let v2 = ModelRef {
        id: hash(&b),
        ..v1.clone()
    };
    assert!(matches!(resolve_model(&v2, &lib, hash), Resolved::Load { .. }));
    assert_eq!(resolve_model(&ModelRef::default(), &lib, hash), Resolved::Nothing);
}

#[test]
fn a_moved_file_is_found_by_its_content_id() {
    let (root, lib) = seeded_root("resolve-moved");
    let b = root.join("tone3000/b.nam");
    let reference = ModelRef {
        path: "/gone/elsewhere/b.nam".into(),
        id: hash(&b),
        name: Some("B".into()),
        source: None,
    };
    let resolved = resolve_model(&reference, &lib.read(), hash);
    match resolved {
        Resolved::Relinked { entry } => assert_eq!(entry.path, b),
        other => panic!("expected Relinked, got {other:?}"),
    }
}

#[test]
fn an_unknown_model_is_missing_with_its_name() {
    let (_root, lib) = seeded_root("resolve-missing");
    let reference = ModelRef {
        path: "/gone/Friedman_BE100_standard_48121.nam".into(),
        id: Some("00".repeat(32)),
        name: Some("Friedman BE-100 · standard".into()),
        source: Some(json!({ "tone3000": { "tone_id": 1934, "model_id": 48121 } })),
    };
    assert_eq!(
        resolve_model(&reference, &lib.read(), hash),
        Resolved::Missing {
            name: "Friedman BE-100 · standard".into(),
            path: "/gone/Friedman_BE100_standard_48121.nam".into(),
            source: Some(Source::Tone3000 {
                tone_id: 1934,
                model_id: 48121
            }),
            file_changed: false,
        }
    );
    // A v1 reference names the file stem.
    let v1 = ModelRef {
        path: "/gone/marshall_jcm800.nam".into(),
        ..ModelRef::default()
    };
    assert!(matches!(
        resolve_model(&v1, &lib.read(), hash),
        Resolved::Missing { name, .. } if name == "marshall_jcm800"
    ));
}

#[test]
fn different_bytes_at_the_path_are_not_the_saved_model() {
    let (root, lib) = seeded_root("resolve-mismatch");
    let a = root.join("tone3000/a.nam");
    // The saved id is one the library does not have: missing, with the
    // "file changed" flag the banner offers "use it anyway" for.
    let reference = ModelRef {
        path: a.to_string_lossy().into_owned(),
        id: Some("ab".repeat(32)),
        ..ModelRef::default()
    };
    assert!(matches!(
        resolve_model(&reference, &lib.read(), hash),
        Resolved::Missing { file_changed: true, .. }
    ));
    // The saved id is in the library under another path: relink there.
    let b = root.join("tone3000/b.nam");
    let reference = ModelRef {
        id: hash(&b),
        ..reference
    };
    assert!(matches!(
        resolve_model(&reference, &lib.read(), hash),
        Resolved::Relinked { entry } if entry.path == b
    ));
}

// ---------------------------------------------------------------------------
// State v2 through an instance
// ---------------------------------------------------------------------------

#[test]
fn a_v1_state_loads_and_gains_the_v2_keys() {
    let (root, lib) = seeded_root("v1");
    let b = root.join("tone3000/b.nam");
    let mut amp = ResonanceAmp::with_library(lib);
    let blob = json!({ "params": { "file_select": 0.0 }, "model_path": b.to_string_lossy() });
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    let keys = model_keys(&state_of(&amp));
    assert_eq!(keys["model_path"], json!(b.to_string_lossy()));
    assert_eq!(keys["model_id"], json!(hash(&b).unwrap()));
    assert_eq!(keys["model_name"], json!("b"));
    assert_eq!(keys["model_source"], json!("external"));
    assert_eq!(amp.param(FILE_SELECT).get_plain(), 1.0, "the slot follows the reference");
    assert_eq!(amp.model_status().state, ModelState::Loaded);
}

#[test]
fn v2_keys_round_trip() {
    let (_root, lib) = seeded_root("v2-roundtrip");
    let reference = json!({
        "model_path": "/gone/x.nam",
        "model_id": "cd".repeat(32),
        "model_name": "Some Amp · feather",
        "model_source": { "tone3000": { "tone_id": 1, "model_id": 2 } },
    });
    let mut blob = reference.clone();
    blob["params"] = json!({});
    let mut amp = ResonanceAmp::with_library(lib);
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert_eq!(Value::Object(model_keys(&state_of(&amp))), reference);
}

#[test]
fn a_missing_model_re_saves_byte_identical_extra_state() {
    let (_root, lib) = seeded_root("missing-resave");
    for reference in [
        json!({ "model_path": "/gone/marshall_jcm800.nam" }),
        json!({
            "model_path": "/gone/x.nam",
            "model_id": "cd".repeat(32),
            "model_name": "Some Amp",
            // A source this build does not understand is kept as-is too.
            "model_source": { "future-store": { "handle": [1, 2, 3] } },
        }),
    ] {
        let mut blob = reference.clone();
        blob["params"] = json!({});
        let mut amp = ResonanceAmp::with_library(lib.clone());
        assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
        assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
        assert!(amp.model_status().is_missing());
        run_blocks(&mut amp, 16);
        let saved = Value::Object(model_keys(&state_of(&amp)));
        assert_eq!(
            serde_json::to_vec(&saved).unwrap(),
            serde_json::to_vec(&reference).unwrap(),
            "the reference must survive a save verbatim"
        );
    }
}

#[test]
fn a_moved_model_relinks_at_activation_and_rewrites_the_path() {
    let (root, lib) = seeded_root("relink");
    let b = root.join("tone3000/b.nam");
    let id = hash(&b).unwrap();
    let blob = json!({
        "params": {},
        "model_path": "/old/machine/b.nam",
        "model_id": id,
        "model_name": "B",
    });
    let mut amp = ResonanceAmp::with_library(lib);
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    let status = amp.model_status();
    assert_eq!(status.state, ModelState::Loaded);
    assert!(status.notice.unwrap().starts_with("Relinked"));
    let keys = model_keys(&state_of(&amp));
    assert_eq!(keys["model_path"], json!(b.to_string_lossy()));
    assert_eq!(keys["model_id"], json!(id));
    assert_eq!(amp.param(FILE_SELECT).get_plain(), 1.0);
}

// ---------------------------------------------------------------------------
// file_select as a stable slot
// ---------------------------------------------------------------------------

#[test]
fn file_select_text_names_the_slot_both_ways() {
    let (_root, lib) = seeded_root("text");
    let amp = ResonanceAmp::with_library(lib.clone());
    let p = amp.param(FILE_SELECT);
    assert_eq!(p.display(0.0), "Test Model", "the name from the file's metadata");
    assert_eq!(p.display(1.0), "b");
    assert_eq!(p.display(7.0), "(empty)");
    assert_eq!(p.parse("b"), Some(1.0), "a model name selects its slot");
    assert_eq!(p.parse("B"), Some(1.0), "case-insensitive");
    let id = lib.read().by_slot(1).unwrap().id.clone();
    assert_eq!(p.parse(&id[..10]), Some(1.0), "an id prefix selects its slot");
    assert_eq!(p.parse("slot 7"), Some(7.0));
    assert_eq!(p.parse("no such model"), None);
}

#[test]
fn file_select_text_says_missing_for_the_missing_models_slot() {
    let (_root, lib) = seeded_root("text-missing");
    let mut amp = ResonanceAmp::with_library(lib);
    let blob = json!({
        "params": { "file_select": 5.0 },
        "model_path": "/gone/x.nam",
        "model_name": "Friedman BE-100",
    });
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    assert_eq!(amp.param(FILE_SELECT).display(5.0), "Missing: Friedman BE-100");
    assert_eq!(amp.param(FILE_SELECT).display(0.0), "Test Model");
}

#[test]
fn an_empty_slot_never_loads_and_never_unloads() {
    let (root, lib) = seeded_root("empty-slot");
    let b = root.join("tone3000/b.nam");
    let mut amp = ResonanceAmp::with_library(lib);
    let blob = json!({ "params": {}, "model_path": b.to_string_lossy() });
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    amp.param(FILE_SELECT).set_plain(42.0);
    assert!(
        pump_until(&mut amp, |a| a.model_status().state == ModelState::EmptySlot(42)),
        "the loader reports the empty slot"
    );
    assert_eq!(
        model_keys(&state_of(&amp))["model_path"],
        json!(b.to_string_lossy()),
        "nothing was clamped onto the last model, and the playing one stays"
    );
    assert_eq!(amp.model_status().name, "b");
}

#[test]
fn adding_models_while_active_does_not_move_the_loaded_one() {
    let (root, lib) = seeded_root("add-while-active");
    let mut amp = ResonanceAmp::with_library(lib.clone());
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    amp.param(FILE_SELECT).set_plain(1.0);
    let b = root.join("tone3000/b.nam").to_string_lossy().into_owned();
    assert!(pump_until(&mut amp, |a| model_keys(&state_of(a))
        .get("model_path")
        .is_some_and(|p| *p == json!(b))));

    // A download that sorts first alphabetically used to shift every
    // index after it.
    std::fs::copy(fixture("lstm/lstm.nam"), root.join("tone3000/0_first.nam")).unwrap();
    lib.rescan().unwrap();
    run_blocks(&mut amp, 64);
    std::thread::sleep(Duration::from_millis(200));
    run_blocks(&mut amp, 64);
    assert_eq!(amp.param(FILE_SELECT).get_plain(), 1.0);
    assert_eq!(model_keys(&state_of(&amp))["model_path"], json!(b));
    assert_eq!(lib.read().by_slot(2).unwrap().name, "Test LSTM", "the new model appended");
    assert_eq!(amp.param(FILE_SELECT).display(1.0), "b");
}

// ---------------------------------------------------------------------------
// The Installed view (library_rows over the shared BrowserModel)
// ---------------------------------------------------------------------------

/// A root with four models of different gear types and names.
fn browse_root(tag: &str) -> Arc<SharedLibrary> {
    let root = temp_root(tag);
    let write = |name: &str, meta: &str, seed: u32| {
        let text = format!(
            r#"{{"version":"0.5.4","architecture":"WaveNet","config":{{}},"weights":[{seed}.0],"sample_rate":48000,"metadata":{meta}}}"#
        );
        std::fs::write(root.join("tone3000").join(name), text).unwrap();
    };
    write("a.nam", r#"{"name":"Friedman BE-100","modeled_by":"J. Smith","gear_type":"amp","tone_type":"crunch"}"#, 1);
    write("b.nam", r#"{"name":"Darkglass MT900","modeled_by":"Steve","gear_type":"amp_cab","tone_type":"clean"}"#, 2);
    write("c.nam", r#"{"name":"5150 Block Letter","modeled_by":"tonekid","gear_type":"amp","tone_type":"hi_gain"}"#, 3);
    write("d.nam", r#"{"name":"Fuzz Face","modeled_by":"tonekid","gear_type":"pedal","tone_type":"fuzz"}"#, 4);
    let lib = shared_for(Some(root));
    lib.rescan().unwrap();
    lib
}

fn view_titles(model: &BrowserModel, rows: &ModelRows) -> Vec<String> {
    model
        .view()
        .iter()
        .map(|&r| rows.rows[r].entry.name.clone())
        .collect()
}

#[test]
fn the_installed_view_searches_filters_and_sorts() {
    let lib = browse_root("view");
    let rows = ModelRows::build(&lib.read(), None, (1, 0));
    let mut model = BrowserModel::new();
    model.refresh(&rows, 1);
    assert_eq!(
        view_titles(&model, &rows),
        vec!["Friedman BE-100", "Darkglass MT900", "5150 Block Letter", "Fuzz Face"],
        "slot order by default"
    );

    model.set_query("tonekid");
    model.refresh(&rows, 1);
    assert_eq!(view_titles(&model, &rows), vec!["5150 Block Letter", "Fuzz Face"]);
    model.set_query("b.nam");
    model.refresh(&rows, 1);
    assert_eq!(view_titles(&model, &rows), vec!["Darkglass MT900"], "file names are searched");

    model.set_query("");
    model.toggle_facet("gear_type", "amp");
    model.refresh(&rows, 1);
    assert_eq!(view_titles(&model, &rows), vec!["Friedman BE-100", "5150 Block Letter"]);
    let types: Vec<(String, usize)> = model
        .facet_counts(&rows, "gear_type")
        .into_iter()
        .map(|c| (c.value, c.count))
        .collect();
    assert!(types.contains(&("pedal".to_string(), 1)));

    model.clear_facets();
    model.set_sort(Sort::by(SortKey::Title));
    model.refresh(&rows, 1);
    assert_eq!(
        view_titles(&model, &rows),
        vec!["5150 Block Letter", "Darkglass MT900", "Friedman BE-100", "Fuzz Face"]
    );
    assert_eq!(rows.rows[0].columns[0], "J. Smith");
    assert_eq!(rows.rows[0].columns[4], "A1");
    assert_eq!(rows.rows[0].columns[5], "48k");
}

#[test]
fn prev_next_walk_the_view_not_the_slot_order() {
    let lib = browse_root("step");
    let rows = ModelRows::build(&lib.read(), None, (1, 0));
    let id = |slot: u32| lib.read().by_slot(slot).unwrap().id.clone();
    let mut model = BrowserModel::new();
    model.set_sort(Sort::by(SortKey::Title));
    model.refresh(&rows, 1);
    // Title order: 5150 (slot 2), Darkglass (1), Friedman (0), Fuzz (3).
    assert_eq!(step_in_view(&model, &rows, Some(&id(2)), 1), Some(1));
    assert_eq!(step_in_view(&model, &rows, Some(&id(1)), 1), Some(0));
    assert_eq!(step_in_view(&model, &rows, Some(&id(3)), 1), None, "clamped at the end");
    assert_eq!(step_in_view(&model, &rows, None, 1), Some(2), "nothing loaded: enter at the top");
    assert_eq!(step_in_view(&model, &rows, None, -1), Some(3));
    assert_eq!(view_counter(&model, Some(&id(0))), "3 / 4 in view");

    model.toggle_facet("gear_type", "amp");
    model.refresh(&rows, 1);
    assert_eq!(view_counter(&model, Some(&id(1))), "– / 2 in view");
    assert_eq!(
        step_in_view(&model, &rows, Some(&id(1)), 1),
        Some(2),
        "a loaded model outside the view steps into it"
    );
}

#[test]
fn the_shared_library_is_one_per_root() {
    let root = temp_root("shared");
    let a = shared_for(Some(root.clone()));
    let b = shared_for(Some(root.clone()));
    assert!(Arc::ptr_eq(&a, &b));
    let other = shared_for(Some(temp_root("shared-other")));
    assert!(!Arc::ptr_eq(&a, &other));
    let _ = Library::empty();
}
