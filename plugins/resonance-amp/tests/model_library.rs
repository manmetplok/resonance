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
use resonance_plugin::library_view::{BrowserModel, LibraryRows, Sort, SortKey};
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
    let lib = shared_for(Some(root.clone()), Some(root.join("marks")));
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
    let lib = shared_for(Some(root.clone()), Some(root.join("marks")));
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
    assert_eq!(rows.column(0, 0).as_deref(), Some("J. Smith"));
    assert_eq!(rows.column(0, 4).as_deref(), Some("A1"));
    assert_eq!(rows.column(0, 5).as_deref(), Some("48k"));
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
fn favourites_sort_first_and_marks_follow_the_content_id() {
    let lib = browse_root("marks");
    let id = |slot: u32| lib.read().by_slot(slot).unwrap().id.clone();
    lib.toggle_favorite(&id(3)).unwrap();
    lib.add_tag(&id(1), "Djent Rhythm").unwrap();
    let gen = lib.marks_generation();
    let rows = ModelRows::build(&lib.read(), Some(&lib.marks().snapshot()), (1, gen));
    let mut model = BrowserModel::new();
    model.refresh(&rows, 1);
    assert_eq!(view_titles(&model, &rows)[0], "Fuzz Face", "the favourite first");
    model.set_query("tag:djent-rhythm");
    model.refresh(&rows, 2);
    assert_eq!(view_titles(&model, &rows), vec!["Darkglass MT900"]);
    model.set_query("");
    model.set_favorites_only(true);
    model.refresh(&rows, 3);
    assert_eq!(view_titles(&model, &rows), vec!["Fuzz Face"]);

    // Marks are written to the shared store under amp-model:<sha256>.
    let key = nam_library::mark_key(&id(3));
    assert!(lib.marks().is_favorite(&key));
    assert!(key.starts_with("amp-model:"));
}

#[test]
fn a_project_restore_is_not_a_recent_pick() {
    let (root, lib) = seeded_root("recents");
    let b = root.join("tone3000/b.nam");
    let mut amp = ResonanceAmp::with_library(lib.clone());
    let blob = json!({ "params": {}, "model_path": b.to_string_lossy() });
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    let id = hash(&b).unwrap();
    assert_eq!(lib.marks_of(&id).last_used, None, "opening a project reorders nothing (D10)");
    lib.record_use(&id).unwrap();
    assert!(lib.marks_of(&id).last_used.is_some());
    assert_eq!(lib.marks_of(&id).use_count, 1);
}

// ---------------------------------------------------------------------------
// Delete (L5)
// ---------------------------------------------------------------------------

#[test]
fn delete_keeps_marks_and_live_instances_and_the_next_activation_is_missing() {
    let (root, lib) = seeded_root("delete");
    let b = root.join("tone3000/b.nam");
    nam_library::write_sidecar(
        &b,
        &nam_library::Sidecar {
            source: nam_library::SOURCE_TONE3000.into(),
            tone_id: Some(1934),
            model_id: Some(48121),
            tone_title: Some("Friedman BE-100".into()),
            size: Some("standard".into()),
            ..nam_library::Sidecar::default()
        },
    )
    .unwrap();
    lib.rescan().unwrap();
    let id = hash(&b).unwrap();
    lib.toggle_favorite(&id).unwrap();

    let mut amp = ResonanceAmp::with_library(lib.clone());
    let blob = json!({ "params": {}, "model_path": b.to_string_lossy() });
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    assert_eq!(lib.usage_count(&id), 1, "used in 1 open amp");
    let saved = state_of(&amp);

    // Delete: the file, its sidecar and its slot go; the marks stay.
    lib.mutate(|l| l.delete(&b)).unwrap();
    assert!(!b.exists());
    assert!(!nam_library::sidecar_path(&b).exists());
    assert!(lib.read().by_slot(1).is_none());
    assert!(lib.marks_of(&id).favorite, "the star survives for a re-download");

    // The live instance keeps playing, and keeps its reference.
    run_blocks(&mut amp, 32);
    assert_eq!(amp.model_status().state, ModelState::Loaded);
    assert_eq!(model_keys(&state_of(&amp)), model_keys(&saved));

    // A project reopened later resolves to missing, with Re-download's key.
    let mut reopened = ResonanceAmp::with_library(lib.clone());
    assert!(reopened.load_state(&serde_json::to_vec(&saved).unwrap()));
    assert!(reopened.initialize(SAMPLE_RATE, BLOCK as u32));
    match reopened.model_status().state {
        ModelState::Missing { name, source, .. } => {
            assert_eq!(name, "Friedman BE-100 · standard");
            assert_eq!(
                source,
                Some(Source::Tone3000 {
                    tone_id: 1934,
                    model_id: 48121
                })
            );
        }
        other => panic!("expected Missing, got {other:?}"),
    }

    // The same bytes coming back take slot 1 again and relink the project.
    std::fs::copy(fixture("a1/wavenet_a1_standard.nam"), &b).unwrap();
    lib.rescan().unwrap();
    assert_eq!(lib.read().slot_of(&id), Some(1));
    let mut again = ResonanceAmp::with_library(lib.clone());
    assert!(again.load_state(&serde_json::to_vec(&saved).unwrap()));
    assert!(again.initialize(SAMPLE_RATE, BLOCK as u32));
    assert_eq!(again.model_status().state, ModelState::Loaded);
    drop(amp);
    drop(reopened);
    assert_eq!(lib.usage_count(&id), 1, "dropped instances leave the count");
}

// ---------------------------------------------------------------------------
// The editor, headless
// ---------------------------------------------------------------------------

fn has(drawn: &[String], text: &str) -> bool {
    drawn.iter().any(|t| t.contains(text))
}

/// The Library overlay, read back from what it draws: the populated
/// Installed tab with its rows and columns, a selected row's detail pane
/// with its tags and usage, a delete armed on one row that does not
/// follow the selection to another, an empty search, and the minimum
/// editor size.
#[test]
fn the_library_overlay_shows_what_the_library_holds() {
    let lib = browse_root("render");
    let first = lib.read().by_slot(0).unwrap().id.clone();
    lib.toggle_favorite(&first).unwrap();
    lib.add_tag(&first, "rhythm").unwrap();
    let mut amp = ResonanceAmp::with_library(lib.clone());
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    // Something else "plays" the first model, for the usage count.
    lib.set_usage(u64::MAX, Some(&first));

    let mut ed = resonance_amp::editor::HeadlessEditor::new(&amp);
    let drawn = ed.frame();
    assert!(has(&drawn, "Library…"), "{drawn:?}");
    assert!(has(&drawn, "– / 4 in view"), "{drawn:?}");

    ed.open_library();
    ed.frame(); // an egui area's first frame only measures it
    let drawn = ed.frame();
    assert!(ed.is_library_open());
    for name in ["Friedman BE-100", "Darkglass MT900", "5150 Block Letter", "Fuzz Face"] {
        assert!(has(&drawn, name), "{name} is listed: {drawn:?}");
    }
    assert!(has(&drawn, "4 models"));
    assert!(has(&drawn, "tonekid"), "the author column");

    ed.select_in_view(0);
    let drawn = ed.frame();
    assert!(has(&drawn, "rhythm"), "the selected row's tags");
    assert!(has(&drawn, "used in 1 open amp"), "usage count: {drawn:?}");
    assert!(has(&drawn, "Delete…"));

    // Arm a delete on the first row, then select the second: no confirm
    // is offered there, and the armed delete is gone.
    ed.begin_delete_selected();
    let drawn = ed.frame();
    assert!(has(&drawn, "Delete \"Friedman BE-100"), "{drawn:?}");
    lib.set_usage(u64::MAX, None);
    assert!(has(&drawn, "Used by 1 open amp"));
    ed.select_in_view(1);
    let drawn = ed.frame();
    assert!(!drawn.iter().any(|t| t.starts_with("Delete \"")), "{drawn:?}");
    assert_eq!(ed.pending_delete(), None);

    ed.set_query("no such model at all");
    let drawn = ed.frame();
    assert!(!has(&drawn, "5150 Block Letter"), "filtered out of the list: {drawn:?}");

    // At the editor's minimum size everything still lays out, the filter
    // row wrapping instead of running off the panel.
    ed.set_query("");
    ed.set_size(760.0, 520.0);
    let drawn = ed.frame();
    assert!(has(&drawn, "Sort: Slot"), "{drawn:?}");
    assert!(has(&drawn, "Arch"));
}

#[test]
fn the_empty_library_and_the_tone3000_tab_render() {
    let root = temp_root("render-empty");
    let lib = shared_for(Some(root.clone()), Some(root.join("marks")));
    let mut amp = ResonanceAmp::with_library(lib);
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    let mut ed = resonance_amp::editor::HeadlessEditor::new(&amp);
    ed.open_library();
    ed.frame();
    let drawn = ed.frame();
    assert!(has(&drawn, "No models installed yet."), "{drawn:?}");
    assert!(has(&drawn, "Browse Tone3000"));
    ed.open_tone3000_tab();
    ed.frame();
    let drawn = ed.frame();
    assert!(has(&drawn, "TONE3000"), "{drawn:?}");
    assert!(has(&drawn, "disconnected"), "offline worker: no saved session was read");
}

#[test]
fn a_deleted_model_shows_deleted_and_a_missing_one_its_banner() {
    let (root, lib) = seeded_root("render-deleted");
    let b = root.join("tone3000/b.nam");
    let mut amp = ResonanceAmp::with_library(lib.clone());
    let blob = json!({ "params": {}, "model_path": b.to_string_lossy() });
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    let mut ed = resonance_amp::editor::HeadlessEditor::new(&amp);
    lib.mutate(|l| l.delete(&b)).unwrap();
    let drawn = ed.frame();
    assert!(has(&drawn, "b (deleted)"), "{drawn:?}");

    let mut missing = ResonanceAmp::with_library(lib);
    let blob = json!({
        "params": {},
        "model_path": b.to_string_lossy(),
        "model_id": "ab".repeat(32),
        "model_name": "Friedman BE-100 · standard",
    });
    assert!(missing.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(missing.initialize(SAMPLE_RATE, BLOCK as u32));
    let mut ed = resonance_amp::editor::HeadlessEditor::new(&missing);
    let drawn = ed.frame();
    assert!(has(&drawn, "Missing model: \"Friedman BE-100 · standard\""), "{drawn:?}");
    assert!(has(&drawn, "Locate file…"));
    ed.locate_mismatch(root.join("tone3000/a.nam"));
    let drawn = ed.frame();
    assert!(has(&drawn, "Use this file anyway?"), "{drawn:?}");
    assert!(has(&drawn, "Use it"));
}

#[test]
fn stepping_browses_without_recording_a_use() {
    // Under "Recently used", recording each ◀/▶ step as a use re-sorted the
    // view under the stepping and bounced between two models.
    let root = temp_root("step-recent");
    for (i, f) in ["a1/wavenet.nam", "a1/wavenet_a1_standard.nam", "lstm/lstm.nam", "a2/wavenet_a2_max.nam"]
        .into_iter()
        .enumerate()
    {
        std::fs::copy(fixture(f), root.join(format!("tone3000/{i}.nam"))).unwrap();
    }
    let lib = shared_for(Some(root.clone()), Some(root.join("marks")));
    lib.rescan().unwrap();
    let mut amp = ResonanceAmp::with_library(lib.clone());
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    let mut ed = resonance_amp::editor::HeadlessEditor::new(&amp);
    ed.set_sort(SortKey::RecentlyUsed);
    let gen = lib.marks_generation();
    let mut visited = Vec::new();
    for _ in 0..4 {
        ed.step(1);
        ed.frame();
        let slot = amp.param(FILE_SELECT).get_plain() as u32;
        visited.push(slot);
        // What `process()` and the loader would do; that model plays.
        let want = lib.read().by_slot(slot).unwrap().id.clone();
        assert!(pump_until(&mut amp, |a| a.model_status().id.as_deref() == Some(want.as_str())));
    }
    assert_eq!(lib.marks_generation(), gen, "browsing wrote no marks");
    visited.dedup();
    assert_eq!(visited.len(), 4, "four steps visit four models: {visited:?}");
}

#[test]
fn a_detail_pane_redownload_neither_loads_nor_compares_with_what_plays() {
    let (root, lib) = seeded_root("redownload-only");
    let mut amp = ResonanceAmp::with_library(lib.clone());
    let a = root.join("tone3000/a.nam");
    let blob = json!({ "params": {}, "model_path": a.to_string_lossy() });
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    let ed = resonance_amp::editor::HeadlessEditor::new(&amp);
    let b = lib.read().by_slot(1).unwrap().clone();
    let before = amp.param(FILE_SELECT).get_plain();

    // Unchanged bytes: "unchanged", and this amp does not switch to b.
    (ed.detail_redownload_done(&b))(&b);
    assert_eq!(amp.param(FILE_SELECT).get_plain(), before);
    assert!(ed.redownload_notice().unwrap().contains("unchanged"));
    // Different bytes than THE ENTRY (not than what plays): says so.
    let mut changed = b.clone();
    changed.id = "cd".repeat(32);
    (ed.detail_redownload_done(&b))(&changed);
    assert!(ed.redownload_notice().unwrap().contains("differs"));
    assert_eq!(amp.param(FILE_SELECT).get_plain(), before);
}

#[test]
fn an_external_models_slot_text_says_external() {
    let (_root, lib) = seeded_root("external-text");
    let outside = temp_root("external-file").join("mine.nam");
    std::fs::copy(fixture("lstm/lstm.nam"), &outside).unwrap();
    let mut amp = ResonanceAmp::with_library(lib);
    let blob = json!({ "params": { "file_select": 1.0 }, "model_path": outside.to_string_lossy() });
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    assert!(amp.model_status().external);
    assert_eq!(amp.param(FILE_SELECT).display(1.0), "External: mine");
    assert_eq!(amp.param(FILE_SELECT).display(0.0), "Test Model", "other slots read as before");
}

#[test]
fn empty_slot_requests_rescan_at_most_every_two_seconds() {
    let (_root, lib) = seeded_root("miss-throttle");
    assert!(lib.rescan_for_miss());
    assert!(!lib.rescan_for_miss(), "a second miss right after does not hash the library again");
}

// ---------------------------------------------------------------------------
// B2: a state loaded into an ACTIVE amp
// ---------------------------------------------------------------------------

#[test]
fn a_state_loaded_while_active_follows_its_model_id_not_its_slot() {
    let (root, lib) = seeded_root("active-id");
    let b = root.join("tone3000/b.nam");
    let b_id = hash(&b).unwrap();
    let mut amp = ResonanceAmp::with_library(lib);
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    amp.param(FILE_SELECT).set_plain(1.0);
    assert!(pump_until(&mut amp, |a| a.model_status().id.as_deref() == Some(b_id.as_str())));

    // A state that points file_select at slot 0 (a) but names b by id.
    let blob = json!({
        "params": { "file_select": 0.0 },
        "model_path": b.to_string_lossy(),
        "model_id": b_id,
    });
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    std::thread::sleep(Duration::from_millis(300));
    run_blocks(&mut amp, 64);
    std::thread::sleep(Duration::from_millis(300));
    run_blocks(&mut amp, 64);
    assert_eq!(amp.model_status().id.as_deref(), Some(b_id.as_str()), "b still plays");
    assert_eq!(model_keys(&state_of(&amp))["model_id"], json!(b_id), "and b is what is saved");
    assert_eq!(amp.param(FILE_SELECT).get_plain(), 1.0, "file_select re-derived from the id");
}

#[test]
fn a_state_whose_slot_is_empty_still_saves_what_plays() {
    let (root, lib) = seeded_root("active-empty-slot");
    let a = root.join("tone3000/a.nam");
    let b = root.join("tone3000/b.nam");
    let a_id = hash(&a).unwrap();
    let mut amp = ResonanceAmp::with_library(lib);
    let blob = json!({ "params": {}, "model_path": a.to_string_lossy() });
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    assert_eq!(amp.model_status().id.as_deref(), Some(a_id.as_str()));

    let blob = json!({
        "params": { "file_select": 500.0 },
        "model_path": b.to_string_lossy(),
        "model_id": hash(&b).unwrap(),
    });
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(pump_until(&mut amp, |a| a.model_status().name == "b"));
    // Whatever plays is what is saved — never a path to one model while
    // another one plays.
    let saved = model_keys(&state_of(&amp));
    assert_eq!(saved["model_path"], json!(b.to_string_lossy()));
    assert_eq!(amp.model_status().id, hash(&b));
}

#[test]
fn a_missing_state_loaded_while_active_stops_the_old_model() {
    let (root, lib) = seeded_root("active-missing");
    let a = root.join("tone3000/a.nam");
    let mut amp = ResonanceAmp::with_library(lib);
    let blob = json!({ "params": {}, "model_path": a.to_string_lossy() });
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    let gone = json!({ "params": {}, "model_path": "/gone/x.nam", "model_name": "X" });
    assert!(amp.load_state(&serde_json::to_vec(&gone).unwrap()));
    assert!(pump_until(&mut amp, |a| a.model_status().is_missing()));
    assert_eq!(model_keys(&state_of(&amp))["model_path"], json!("/gone/x.nam"));
    assert_eq!(amp.model_status().id, None, "nothing of a's plays under x's reference");
}

// ---------------------------------------------------------------------------
// B1: activation writes nothing
// ---------------------------------------------------------------------------

const CHILD_HOME: &str = "RESONANCE_AMP_B1_HOME";

/// Worker half of `activating_an_amp_writes_nothing_under_the_data_dir`,
/// run in a child process whose `XDG_DATA_HOME` / `HOME` point at an empty
/// temporary directory (set by the parent on the child only).
#[test]
#[ignore = "worker half of activating_an_amp_writes_nothing_under_the_data_dir"]
fn b1_child_activates_an_amp() {
    let Some(_home) = std::env::var_os(CHILD_HOME) else {
        return;
    };
    let mut amp = <ResonanceAmp as ResonancePlugin>::new();
    let blob = json!({
        "params": { "file_select": 3.0 },
        "model_path": fixture("a1/wavenet.nam").to_string_lossy(),
        "model_id": "ab".repeat(32),
    });
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    run_blocks(&mut amp, 16);
    let _ = amp.save_state();
    amp.reset();
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    run_blocks(&mut amp, 16);
}

#[test]
fn activating_an_amp_writes_nothing_under_the_data_dir() {
    let home = std::env::temp_dir().join(format!("resonance-amp-b1-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    // An installed download, so the library root exists: an activation that
    // scanned would index it (library.json, library.lock) and prune marks.
    let model = home.join("resonance/amp-models/tone3000/installed.nam");
    std::fs::create_dir_all(model.parent().unwrap()).unwrap();
    std::fs::copy(fixture("a1/wavenet.nam"), &model).unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "b1_child_activates_an_amp", "--test-threads=1"])
        .env(CHILD_HOME, &home)
        .env("XDG_DATA_HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("HOME", &home)
        .env_remove(nam_library::AMP_MODEL_DIR_ENV)
        .env_remove(resonance_common::library_marks::LIBRARY_DIR_ENV)
        .status()
        .unwrap();
    assert!(status.success(), "the child activation failed");
    let left: Vec<_> = walk(&home);
    assert_eq!(left, vec![model], "activation created files under the data dir");
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
                if out.is_empty() {
                    out.push(p);
                }
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn the_shared_library_is_one_per_root() {
    let root = temp_root("shared");
    let a = shared_for(Some(root.clone()), Some(root.join("marks")));
    let b = shared_for(Some(root.clone()), Some(root.join("marks")));
    assert!(Arc::ptr_eq(&a, &b));
    let other = shared_for(Some(temp_root("shared-other")), None);
    assert!(!Arc::ptr_eq(&a, &other));
    let _ = Library::empty();
}
