//! Media-browser navigation + audition control (doc #175, todo #599).
//!
//! These drive the real reducer (`update::browser`) through the app's
//! dispatcher to pin the message contract: filesystem navigation +
//! per-folder filter + favourite/recent management + Files/Pool tab, and
//! the audition preview transport (play / stop / scrub / loop / sync /
//! auto-play). Everything here is transient — none of it is undoable or
//! persisted in the project — so we also assert the undo classifier skips
//! every browser message.
//!
//! Favourite / recent toggles mirror to user settings (`settings.json`).
//! To keep that hermetic we point `XDG_CONFIG_HOME` at a throwaway temp
//! dir for the whole test binary before constructing any app, so the
//! developer's real config is never touched.

use std::path::{Path, PathBuf};
use std::sync::Once;

use resonance_app::message::{BrowserMessage, Message};
use resonance_app::state::{BrowserState, BrowserTab, FolderScan};
use resonance_app::undo::{classify, UndoAction};
use resonance_app::update::browser::scan_folder;
use resonance_app::Resonance;
use resonance_audio::__test_support::Receiver;
use resonance_audio::types::AudioCommand;
use resonance_common::audio_probe::{AudioFileEntry, AudioFormat, AudioInfo};

static REDIRECT_CONFIG: Once = Once::new();

/// Point `XDG_CONFIG_HOME` at a temp dir so favourite/recent persistence
/// writes land in a throwaway location rather than the real config dir.
fn isolate_config() {
    REDIRECT_CONFIG.call_once(|| {
        let dir = std::env::temp_dir().join("resonance-browser-test-config");
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("XDG_CONFIG_HOME", &dir);
    });
}

fn app() -> Resonance {
    isolate_config();
    let (app, _task) = Resonance::new();
    app
}

/// Build an app with a captured engine so audition-command emission can be
/// asserted. Uses `test_dispatch`, which bypasses the startup / bounce
/// gates, so no active project is needed.
fn app_capturing() -> (Resonance, Receiver<AudioCommand>) {
    isolate_config();
    let (mut app, _task) = Resonance::new();
    let rx = app.test_capture_engine();
    (app, rx)
}

fn send(app: &mut Resonance, m: BrowserMessage) {
    app.test_dispatch(Message::Browser(m));
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn audio_entry(path: &str) -> AudioFileEntry {
    AudioFileEntry {
        path: path.to_string(),
        info: AudioInfo {
            format: AudioFormat::Wav,
            channels: 2,
            sample_rate: 48_000,
            frames: 96_000,
            duration_secs: 2.0,
        },
    }
}

// -- Tab switching -----------------------------------------------------

#[test]
fn default_tab_is_files() {
    let app = app();
    assert_eq!(app.test_browser().tab, BrowserTab::Files);
}

#[test]
fn select_tab_switches_between_files_and_pool() {
    let mut app = app();
    send(&mut app, BrowserMessage::SelectTab(BrowserTab::Pool));
    assert_eq!(app.test_browser().tab, BrowserTab::Pool);
    send(&mut app, BrowserMessage::SelectTab(BrowserTab::Files));
    assert_eq!(app.test_browser().tab, BrowserTab::Files);
}

// -- Folder navigation + scan caching ----------------------------------

#[test]
fn open_folder_sets_current_folder_and_marks_scanning() {
    let mut app = app();
    let folder = PathBuf::from("/tmp/some/loops");
    send(&mut app, BrowserMessage::OpenFolder(folder.clone()));

    let b = app.test_browser();
    assert_eq!(b.current_folder.as_deref(), Some(folder.as_path()));
    assert!(b.scanning, "an off-thread scan is in flight after navigate");
    assert!(b.scan.files.is_empty(), "previous rows are cleared on nav");
}

#[test]
fn open_folder_records_recent_most_recent_first() {
    let mut app = app();
    send(&mut app, BrowserMessage::OpenFolder(PathBuf::from("/a")));
    send(&mut app, BrowserMessage::OpenFolder(PathBuf::from("/b")));

    let recent = &app.test_pool().recent_folders;
    assert_eq!(recent.first(), Some(&PathBuf::from("/b")));
    assert!(recent.contains(&PathBuf::from("/a")));
}

#[test]
fn open_folder_clears_the_previous_filter() {
    let mut app = app();
    send(&mut app, BrowserMessage::OpenFolder(PathBuf::from("/a")));
    send(&mut app, BrowserMessage::SetFilter("kick".into()));
    assert_eq!(app.test_browser().filter, "kick");

    send(&mut app, BrowserMessage::OpenFolder(PathBuf::from("/b")));
    assert_eq!(app.test_browser().filter, "", "filter resets on navigate");
}

#[test]
fn scan_completed_applies_for_current_folder() {
    let mut app = app();
    let folder = PathBuf::from("/tmp/loops");
    send(&mut app, BrowserMessage::OpenFolder(folder.clone()));

    let scan = FolderScan {
        folders: vec![PathBuf::from("/tmp/loops/sub")],
        files: vec![audio_entry("/tmp/loops/kick.wav")],
    };
    send(
        &mut app,
        BrowserMessage::ScanCompleted {
            folder: folder.clone(),
            scan: scan.clone(),
        },
    );

    let b = app.test_browser();
    assert!(!b.scanning, "scanning flag cleared when the scan lands");
    assert_eq!(b.scan.folders, scan.folders);
    assert_eq!(b.scan.files.len(), 1);
}

#[test]
fn stale_scan_for_a_left_folder_is_dropped() {
    let mut app = app();
    send(&mut app, BrowserMessage::OpenFolder(PathBuf::from("/a")));
    send(&mut app, BrowserMessage::OpenFolder(PathBuf::from("/b")));

    // A late result for `/a` arrives after we've navigated to `/b`.
    send(
        &mut app,
        BrowserMessage::ScanCompleted {
            folder: PathBuf::from("/a"),
            scan: FolderScan {
                folders: Vec::new(),
                files: vec![audio_entry("/a/old.wav")],
            },
        },
    );

    let b = app.test_browser();
    assert!(b.scan.files.is_empty(), "stale rows are not shown");
    assert!(b.scanning, "the current folder's scan is still pending");
}

// -- Filtering ---------------------------------------------------------

#[test]
fn filter_matches_file_name_case_insensitively() {
    let mut b = BrowserState {
        scan: FolderScan {
            folders: Vec::new(),
            files: vec![
                audio_entry("/loops/Kick_01.wav"),
                audio_entry("/loops/snare.wav"),
                audio_entry("/loops/HiHat.wav"),
            ],
        },
        ..BrowserState::default()
    };

    b.filter = "kick".into();
    let matched: Vec<_> = b.filtered_files().map(|e| e.path.as_str()).collect();
    assert_eq!(matched, vec!["/loops/Kick_01.wav"]);

    b.filter = String::new();
    assert_eq!(b.filtered_files().count(), 3, "empty filter passes all");
}

// -- Breadcrumb --------------------------------------------------------

#[test]
fn breadcrumb_is_root_first_ancestor_chain() {
    let b = BrowserState {
        current_folder: Some(PathBuf::from("/home/me/loops")),
        ..BrowserState::default()
    };
    let crumbs = b.breadcrumb();
    assert_eq!(crumbs.first(), Some(&PathBuf::from("/")));
    assert_eq!(crumbs.last(), Some(&PathBuf::from("/home/me/loops")));
    // Every crumb is an ancestor of the current folder, in nesting order.
    assert!(crumbs.windows(2).all(|w| w[1].starts_with(&w[0])));
}

#[test]
fn breadcrumb_is_empty_before_any_navigation() {
    assert!(BrowserState::default().breadcrumb().is_empty());
}

// -- Favourites --------------------------------------------------------

#[test]
fn toggle_favourite_pins_then_unpins() {
    let mut app = app();
    let folder = PathBuf::from("/tmp/kits");

    send(&mut app, BrowserMessage::ToggleFavourite(folder.clone()));
    assert!(app.test_pool().is_favourite(&folder), "pinned on first toggle");

    send(&mut app, BrowserMessage::ToggleFavourite(folder.clone()));
    assert!(!app.test_pool().is_favourite(&folder), "unpinned on second");
}

// -- Audition transport ------------------------------------------------

#[test]
fn select_without_autoplay_only_highlights() {
    let (mut app, rx) = app_capturing();
    let path = PathBuf::from("/loops/kick.wav");
    send(&mut app, BrowserMessage::Select(Some(path.clone())));

    assert!(app.test_browser().audition.is_selected(&path));
    assert!(app.test_browser().audition.playing.is_none());
    assert!(drain(&rx).is_empty(), "no engine command without auto-play");
}

#[test]
fn select_with_autoplay_previews_immediately() {
    let (mut app, rx) = app_capturing();
    send(&mut app, BrowserMessage::ToggleAutoPlay);
    assert!(app.test_browser().audition.auto_play);

    let path = PathBuf::from("/loops/kick.wav");
    send(&mut app, BrowserMessage::Select(Some(path.clone())));

    assert!(app.test_browser().audition.is_playing(&path));
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::AuditionFile { path: p, start_frame: 0 }] if *p == path
    ));
}

#[test]
fn play_then_stop_round_trips_engine_and_state() {
    let (mut app, rx) = app_capturing();
    let path = PathBuf::from("/loops/loop.wav");

    send(&mut app, BrowserMessage::Play(path.clone()));
    assert!(app.test_browser().audition.is_playing(&path));
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::AuditionFile { path: p, start_frame: 0 }] if *p == path
    ));

    send(&mut app, BrowserMessage::Stop);
    assert!(app.test_browser().audition.playing.is_none());
    assert_eq!(app.test_browser().audition.position_frame, 0);
    assert!(matches!(drain(&rx).as_slice(), [AudioCommand::StopAudition]));
}

#[test]
fn stop_when_idle_is_a_silent_no_op() {
    let (mut app, rx) = app_capturing();
    send(&mut app, BrowserMessage::Stop);
    assert!(drain(&rx).is_empty(), "no StopAudition when nothing plays");
}

#[test]
fn scrub_seeks_by_restarting_the_preview_at_the_frame() {
    let (mut app, rx) = app_capturing();
    let path = PathBuf::from("/loops/loop.wav");
    send(&mut app, BrowserMessage::Play(path.clone()));
    let _ = drain(&rx);

    send(&mut app, BrowserMessage::Scrub(24_000));
    assert_eq!(app.test_browser().audition.position_frame, 24_000);
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::AuditionFile { path: p, start_frame: 24_000 }] if *p == path
    ));
}

#[test]
fn scrub_with_nothing_selected_does_nothing() {
    let (mut app, rx) = app_capturing();
    send(&mut app, BrowserMessage::Scrub(1_000));
    assert!(drain(&rx).is_empty());
    assert_eq!(app.test_browser().audition.position_frame, 0);
}

#[test]
fn toggle_loop_pushes_audition_options() {
    let (mut app, rx) = app_capturing();
    send(&mut app, BrowserMessage::ToggleLoop);

    assert!(app.test_browser().audition.loop_enabled);
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::SetAuditionOptions { loop_enabled: true, sync_to_tempo: false }]
    ));
}

#[test]
fn toggle_sync_pushes_audition_options() {
    let (mut app, rx) = app_capturing();
    send(&mut app, BrowserMessage::ToggleSync);

    assert!(app.test_browser().audition.sync_to_tempo);
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::SetAuditionOptions { loop_enabled: false, sync_to_tempo: true }]
    ));
}

#[test]
fn toggle_auto_play_is_pure_ui_no_engine() {
    let (mut app, rx) = app_capturing();
    send(&mut app, BrowserMessage::ToggleAutoPlay);
    assert!(app.test_browser().audition.auto_play);
    assert!(drain(&rx).is_empty(), "auto-play never talks to the engine");
}

#[test]
fn select_none_clears_selection_and_stops_preview() {
    let (mut app, rx) = app_capturing();
    let path = PathBuf::from("/loops/loop.wav");
    send(&mut app, BrowserMessage::Play(path));
    let _ = drain(&rx);

    send(&mut app, BrowserMessage::Select(None));
    assert!(app.test_browser().audition.selected.is_none());
    assert!(app.test_browser().audition.playing.is_none());
    assert!(matches!(drain(&rx).as_slice(), [AudioCommand::StopAudition]));
}

// -- Off-thread scan helper --------------------------------------------

#[test]
fn scan_folder_lists_sorted_subfolders_and_skips_non_audio() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir(root.join("zeta")).unwrap();
    std::fs::create_dir(root.join("alpha")).unwrap();
    std::fs::write(root.join("notes.txt"), b"not audio").unwrap();

    let scan = scan_folder(root);

    let names: Vec<_> = scan
        .folders
        .iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
        .collect();
    assert_eq!(names, vec!["alpha", "zeta"], "subfolders sorted by path");
    assert!(scan.files.is_empty(), "the .txt file is not an audio row");
}

#[test]
fn scan_folder_on_missing_dir_is_empty() {
    let scan = scan_folder(Path::new("/definitely/not/a/real/dir/xyz"));
    assert!(scan.folders.is_empty() && scan.files.is_empty());
}

// -- Undo classification ------------------------------------------------

#[test]
fn every_browser_message_is_skipped_by_undo() {
    let msgs = [
        BrowserMessage::SelectTab(BrowserTab::Pool),
        BrowserMessage::OpenFolder(PathBuf::from("/a")),
        BrowserMessage::ScanCompleted {
            folder: PathBuf::from("/a"),
            scan: FolderScan::default(),
        },
        BrowserMessage::SetFilter("x".into()),
        BrowserMessage::ToggleFavourite(PathBuf::from("/a")),
        BrowserMessage::Select(Some(PathBuf::from("/a/x.wav"))),
        BrowserMessage::Select(None),
        BrowserMessage::Play(PathBuf::from("/a/x.wav")),
        BrowserMessage::Stop,
        BrowserMessage::Scrub(10),
        BrowserMessage::ToggleLoop,
        BrowserMessage::ToggleSync,
        BrowserMessage::ToggleAutoPlay,
    ];
    for m in msgs {
        assert!(
            matches!(classify(&Message::Browser(m.clone())), UndoAction::Skip),
            "browser message must be transient: {m:?}"
        );
    }
}
