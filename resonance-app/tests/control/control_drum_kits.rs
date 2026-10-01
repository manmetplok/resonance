//! `drum_kits.*` — the per-user drum-kit library over the control API
//! (drums-plugin-rework.md §8), against a temporary library root handed
//! to each app with `test_set_drum_kit_library_roots` (no environment
//! variable, so the tests run in parallel).

use std::path::{Path, PathBuf};

use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_common::drumkit_library::{self, write_sidecar, Sidecar, MANIFEST_FILE, SOURCE_PLOK};
use resonance_common::library_marks::MarksStore;
use resonance_control::methods::drum_kits::{
    DrumKitEntry, DrumKitList, DrumKitSource, DrumKitStatus,
};
use resonance_control::ErrorKind;

use crate::common::call;

/// A small kit manifest: `pieces` pieces, two mic setups, 3 layers × 2
/// round robins each (the sample files are not needed: a scan never opens
/// them). `meta` is the optional `_meta` object.
fn write_kit(manifest_dir: &Path, piece_names: &[&str], meta: Option<serde_json::Value>) {
    std::fs::create_dir_all(manifest_dir).unwrap();
    let mut obj = serde_json::Map::new();
    for piece in piece_names {
        let mut setups = serde_json::Map::new();
        for (key, pos, brand, mic) in [
            ("01_KickIn_e901", "KickIn", "Sennheiser", "e901"),
            ("19_OHsAB_KM184", "OHsAB", "Neumann", "KM184"),
        ] {
            let mut rounds = serde_json::Map::new();
            for rr in 1..=2 {
                let mut vels = serde_json::Map::new();
                for v in 1..=3 {
                    vels.insert(
                        format!("Vel{v:02}"),
                        format!("{piece} {key} {rr} {v}.wav").into(),
                    );
                }
                rounds.insert(format!("RR{rr:02}"), vels.into());
            }
            setups.insert(
                key.into(),
                serde_json::json!({
                    "brand": brand, "channel": &key[..2], "mic": mic, "position": pos,
                    "rounds": rounds,
                }),
            );
        }
        obj.insert((*piece).into(), setups.into());
    }
    if let Some(meta) = meta {
        obj.insert("_meta".into(), meta);
    }
    std::fs::write(
        manifest_dir.join(MANIFEST_FILE),
        serde_json::to_string_pretty(&obj).unwrap(),
    )
    .unwrap();
}

/// A fresh root with two kits — "Drummica" (a plok.org download with its
/// sidecar) and "Garage" (copied in by hand) — and an app pointed at it.
/// No project is open: the library is the user's, not the project's.
fn app(tag: &str) -> (Resonance, PathBuf) {
    let base =
        std::env::temp_dir().join(format!("resonance-drum-kits-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let kits = base.join("kits");
    write_kit(
        &kits.join("Drummica/drummica"),
        &["SD Kick mit Teppich", "SD Snare Normal", "SD Hat Closed"],
        None,
    );
    write_sidecar(
        &kits.join("Drummica"),
        &Sidecar {
            source: SOURCE_PLOK.into(),
            index_name: Some("Drummica".into()),
            description: Some("Acoustic studio kit".into()),
            size_bytes: Some(9_126_805_504),
            ..Sidecar::default()
        },
    )
    .unwrap();
    write_kit(
        &kits.join("Garage"),
        &["Kick", "Snare"],
        Some(serde_json::json!({ "name": "Garage Kit" })),
    );
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_drum_kit_library_roots(kits, base.join("marks"));
    (app, base.join("marks"))
}

fn list(app: &mut Resonance, params: serde_json::Value) -> DrumKitList {
    call(app, "drum_kits.list", params)
        .result()
        .expect("drum_kits.list succeeds")
}

fn names(list: &DrumKitList) -> Vec<&str> {
    list.kits.iter().map(|k| k.name.as_str()).collect()
}

#[test]
fn a_test_app_never_points_at_the_users_kit_library() {
    let (app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let roots = app.test_drum_kit_library_roots();
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    for root in [roots.kits.unwrap(), roots.marks.unwrap()] {
        assert!(
            root.starts_with(std::env::temp_dir()),
            "{} is not a temp dir",
            root.display()
        );
        assert!(home.as_os_str().is_empty() || !root.starts_with(home.join(".local")));
    }
}

#[test]
fn drum_kits_list_reports_filters_and_limits() {
    let (mut app, marks_dir) = app("list");
    let revision = app.revision();

    let all = list(&mut app, serde_json::json!({}));
    assert_eq!(all.total, 2);
    assert_eq!(all.matched, 2);
    assert_eq!(names(&all), vec!["Drummica", "Garage Kit"], "slot order");
    let drummica = &all.kits[0];
    assert_eq!(drummica.slot, Some(0));
    assert_eq!(all.kits[1].slot, Some(1));
    assert_eq!(drummica.id.len(), 64, "a sha256 content id");
    assert_eq!(drummica.source, DrumKitSource::Plok);
    assert_eq!(all.kits[1].source, DrumKitSource::Local);
    assert_eq!(drummica.description.as_deref(), Some("Acoustic studio kit"));
    assert_eq!(drummica.pieces, 3);
    assert_eq!(drummica.piece_names.len(), 3);
    assert_eq!(drummica.mic_setups, 2);
    assert_eq!(drummica.layers, 3);
    assert_eq!(drummica.rr, 2);
    assert_eq!(drummica.size_bytes, Some(9_126_805_504));
    assert_eq!(drummica.status, DrumKitStatus::Ok);
    assert!(drummica.loaded_in.is_empty(), "no project, no users");
    assert!(all.library_generation > 0);

    let by_query = list(&mut app, serde_json::json!({ "query": "garage" }));
    assert_eq!(names(&by_query), vec!["Garage Kit"]);
    let by_source = list(&mut app, serde_json::json!({ "source": "plok" }));
    assert_eq!(names(&by_source), vec!["Drummica"]);
    let by_is = list(&mut app, serde_json::json!({ "query": "is:local" }));
    assert_eq!(names(&by_is), vec!["Garage Kit"]);
    let limited = list(&mut app, serde_json::json!({ "limit": 1 }));
    assert_eq!(limited.kits.len(), 1);
    assert_eq!(limited.matched, 2, "matched counts past the limit");

    // A favourite set elsewhere (the plugin, another process) sorts first.
    MarksStore::open(&marks_dir)
        .unwrap()
        .set_favorite(&drumkit_library::mark_key(&all.kits[1].id), true)
        .unwrap();
    let starred = list(&mut app, serde_json::json!({}));
    assert_eq!(starred.kits[0].name, "Garage Kit");
    assert!(starred.kits[0].favorite);
    let only = list(&mut app, serde_json::json!({ "favorites_only": true }));
    assert_eq!(names(&only), vec!["Garage Kit"]);
    assert_eq!(
        app.revision(),
        revision,
        "reading the library is not a project edit"
    );
}

#[test]
fn drum_kits_list_answers_with_no_project_and_an_empty_library() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let empty = list(&mut app, serde_json::json!({}));
    assert_eq!(empty.total, 0);
    assert!(empty.kits.is_empty());
}

#[test]
fn drum_kits_list_names_the_tracks_that_play_a_kit() {
    use resonance_audio::types::{AudioEvent, ParamInfo, TrackType};
    let (mut app, _) = app("users");
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/tmp/control-drum-kits.rprj"));
    app.test_add_track(1, TrackType::Instrument);
    app.test_add_track(2, TrackType::Instrument);
    for (track_id, instance_id, slot) in [(1, 40, 1.0), (2, 41, -1.0)] {
        app.test_apply_engine_event(AudioEvent::PluginAdded {
            track_id,
            instance_id,
            plugin_name: "Resonance Drums".to_owned(),
            clap_plugin_id: "com.resonance.drums".to_owned(),
            clap_file_path: "/plugins/drums.clap".to_owned(),
            params: vec![ParamInfo {
                id: resonance_plugin::stable_hash("kit_select"),
                name: "Kit".to_owned(),
                min_value: -2.0,
                max_value: 999.0,
                current_value: slot,
                stepped: true,
                ..Default::default()
            }],
            has_gui: false,
            has_sidechain_input: false,
            output_port_count: 1,
            output_port_names: vec!["Main".to_owned()],
        });
    }
    let all = list(&mut app, serde_json::json!({}));
    let garage = all.kits.iter().find(|k| k.slot == Some(1)).unwrap();
    assert_eq!(garage.loaded_in.len(), 1, "{:?}", garage.loaded_in);
    assert_eq!(garage.loaded_in[0].track_id.0, 1);
    let drummica = all.kits.iter().find(|k| k.slot == Some(0)).unwrap();
    assert!(
        drummica.loaded_in.is_empty(),
        "track 2 plays the built-in kit (-1)"
    );
}

#[test]
fn drum_kits_set_marks_writes_the_shared_store_only() {
    let (mut app, marks_dir) = app("marks");
    let revision = app.revision();
    let all = list(&mut app, serde_json::json!({}));
    let id = all.kits[0].id.clone();
    let updated: DrumKitEntry = call(
        &mut app,
        "drum_kits.set_marks",
        serde_json::json!({ "id": &id[..10], "favorite": true, "tags": ["Live Room", "rock"] }),
    )
    .result()
    .expect("a unique id prefix addresses the kit");
    assert_eq!(updated.id, id);
    assert!(updated.favorite);
    assert_eq!(
        updated.tags,
        vec!["live-room", "rock"],
        "tags are normalised"
    );
    // The plugin's store sees it: same file, same key.
    let store = MarksStore::open(&marks_dir).unwrap();
    let marks = store.marks(&drumkit_library::mark_key(&id));
    assert!(marks.favorite);
    assert_eq!(marks.tags, vec!["live-room", "rock"]);

    // Tags replace; favorite alone leaves tags alone.
    let cleared: DrumKitEntry = call(
        &mut app,
        "drum_kits.set_marks",
        serde_json::json!({ "id": id, "tags": [] }),
    )
    .result()
    .unwrap();
    assert!(cleared.tags.is_empty());
    assert!(cleared.favorite, "not mentioned, not changed");

    let error = call(
        &mut app,
        "drum_kits.set_marks",
        serde_json::json!({ "id": id }),
    )
    .error
    .expect("nothing to set is refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    let error = call(
        &mut app,
        "drum_kits.set_marks",
        serde_json::json!({ "id": "ffffffffffff", "favorite": true }),
    )
    .error
    .expect("an unknown id is refused");
    assert_eq!(error.kind(), ErrorKind::NotFound);

    assert_eq!(
        app.revision(),
        revision,
        "the library and its marks are the user's, not the project's: no revision bump"
    );
}
