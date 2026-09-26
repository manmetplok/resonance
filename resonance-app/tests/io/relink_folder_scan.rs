//! "Search a folder…" batch relink walks the tree off the UI thread
//! (review VIEW-29 / UPD-10).
//!
//! `start_batch_relink` used to run a recursive `read_dir` walk (depth 24,
//! every file) inside `update()`: pointing it at `~` or a sample library
//! froze the window, meters and transport. The walk now runs on a worker;
//! the reducer only records the scan and the imports start from its
//! `ScanFinished` result. It can be cancelled, and its result is
//! deterministic (breadth-first, name-sorted).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use resonance_app::message::{Message, RelinkMessage};
use resonance_app::state::{PoolAsset, ScanControl};
use resonance_app::update::relink::{scan_folder_for_names, scan_folder_for_names_with};
use resonance_app::Resonance;
use resonance_audio::types::AssetId;

fn temp_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("resonance-relink-scan-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("audio")).expect("create audio dir");
    dir
}

fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"x").unwrap();
}

fn missing_asset(id: AssetId, original_path: &str) -> PoolAsset {
    PoolAsset {
        id,
        project_relative_path: format!("audio/asset_{id}.wav"),
        original_path: original_path.to_string(),
        format: resonance_common::AudioFormat::Wav,
        channels: 2,
        source_sample_rate: 48_000,
        duration_frames: 0,
        thumbnail_peaks: Vec::new(),
        missing: true,
    }
}

fn app_with_missing(dir: &Path) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.to_path_buf());
    app.test_add_pool_asset(missing_asset(1, "/lost/kick.wav"));
    app.test_add_pool_asset(missing_asset(2, "/lost/snare.wav"));
    app.test_dispatch(Message::Relink(RelinkMessage::ShowModal));
    app
}

fn found(pairs: &[(&str, PathBuf)]) -> Option<std::collections::HashMap<String, PathBuf>> {
    Some(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}

#[test]
fn folder_chosen_defers_the_walk_to_a_task() {
    let dir = temp_dir("defer");
    let mut app = app_with_missing(&dir);
    let task = app.update(Message::Relink(RelinkMessage::FolderChosen(Some(
        dir.join("search"),
    ))));
    assert!(task.units() > 0, "the walk runs as a task");
    let relink = app.test_relink();
    assert!(relink.scanning(), "scan recorded for progress / cancel");
    assert!(
        relink.in_flight.is_empty(),
        "no import may start before the walk reports back"
    );

    // A second pick while scanning doesn't start a duplicate walk.
    let token = relink.scan.as_ref().unwrap().token;
    let task = app.update(Message::Relink(RelinkMessage::FolderChosen(Some(
        dir.join("other"),
    ))));
    assert_eq!(task.units(), 0);
    assert_eq!(app.test_relink().scan.as_ref().unwrap().token, token);

    // The result starts the imports for what it found.
    app.test_dispatch(Message::Relink(RelinkMessage::ScanFinished(
        token,
        found(&[("kick.wav", dir.join("search/kick.wav"))]),
    )));
    let relink = app.test_relink();
    assert!(!relink.scanning());
    assert!(relink.is_in_flight(1));
    assert!(!relink.is_in_flight(2), "unfound asset stays missing");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancelled_or_dismissed_scan_result_is_ignored() {
    let dir = temp_dir("cancel");
    let mut app = app_with_missing(&dir);
    app.test_dispatch(Message::Relink(RelinkMessage::FolderChosen(Some(
        dir.join("search"),
    ))));
    let scan = app.test_relink().scan.clone().unwrap();
    app.test_dispatch(Message::Relink(RelinkMessage::CancelScan));
    assert!(scan.control.cancel.load(Ordering::Relaxed), "worker told to stop");
    assert!(!app.test_relink().scanning());
    // A late result from the cancelled walk must not start imports.
    app.test_dispatch(Message::Relink(RelinkMessage::ScanFinished(
        scan.token,
        found(&[("kick.wav", dir.join("search/kick.wav"))]),
    )));
    assert!(app.test_relink().in_flight.is_empty());

    // Closing the modal cancels a running scan too.
    app.test_dispatch(Message::Relink(RelinkMessage::FolderChosen(Some(
        dir.join("search"),
    ))));
    let scan = app.test_relink().scan.clone().unwrap();
    app.test_dispatch(Message::Relink(RelinkMessage::DismissModal));
    assert!(scan.control.cancel.load(Ordering::Relaxed));
    app.test_dispatch(Message::Relink(RelinkMessage::ScanFinished(
        scan.token,
        found(&[("kick.wav", dir.join("search/kick.wav"))]),
    )));
    assert!(app.test_relink().in_flight.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scan_is_deterministic_shallowest_then_name_order() {
    let dir = temp_dir("determinism");
    let search = dir.join("search");
    touch(&search.join("b/deep/kick.wav"));
    touch(&search.join("z/kick.wav"));
    touch(&search.join("a/kick.wav"));
    touch(&search.join("m/Snare.wav"));
    touch(&search.join("snare.wav"));
    let names = vec!["kick.wav".to_string(), "snare.wav".to_string()];
    for _ in 0..3 {
        let found = scan_folder_for_names(&search, &names);
        assert_eq!(found.get("kick.wav"), Some(&search.join("a/kick.wav")));
        assert_eq!(found.get("snare.wav"), Some(&search.join("snare.wav")));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scan_reports_progress_and_stops_when_cancelled() {
    let dir = temp_dir("progress");
    let search = dir.join("search");
    touch(&search.join("a/x.wav"));
    touch(&search.join("b/c/y.wav"));
    let names = vec!["nope.wav".to_string()];

    let control = ScanControl::default();
    let found = scan_folder_for_names_with(&search, &names, &control);
    assert_eq!(found.map(|f| f.len()), Some(0));
    assert_eq!(control.dirs_scanned(), 4, "search, a, b, b/c");

    let control = ScanControl::default();
    control.cancel.store(true, Ordering::Relaxed);
    assert!(scan_folder_for_names_with(&search, &names, &control).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
