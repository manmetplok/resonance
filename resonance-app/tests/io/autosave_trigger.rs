//! The periodic autosave trigger (ba todo #465; code review UPD-07: the
//! write path was merged but nothing ever emitted an autosave, so a crash
//! lost everything since the last Ctrl+S although the settings said
//! autosave was on).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use resonance_app::message::{Message, ProjectIoMessage, TrackMessage};
use resonance_app::settings::AutosaveSettings;
use resonance_app::state::ViewMode;
use resonance_app::update::project_io::{should_autosave, AutosaveGate};
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, TrackType};

const TRACK: u64 = 1;

// ---- Pure gate --------------------------------------------------------

fn ready_gate() -> AutosaveGate {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    AutosaveGate {
        enabled: true,
        dirty: true,
        changed_since_autosave: true,
        has_active_project: true,
        busy: false,
        save_in_flight: false,
        now,
        due_from: now - Duration::from_secs(60),
        interval: Duration::from_secs(30),
    }
}

#[test]
fn gate_fires_only_when_every_precondition_holds() {
    assert!(should_autosave(ready_gate()));
    let blocked = [
        AutosaveGate { enabled: false, ..ready_gate() },
        AutosaveGate { dirty: false, ..ready_gate() },
        AutosaveGate { changed_since_autosave: false, ..ready_gate() },
        AutosaveGate { has_active_project: false, ..ready_gate() },
        AutosaveGate { busy: true, ..ready_gate() },
        AutosaveGate { save_in_flight: true, ..ready_gate() },
    ];
    for g in blocked {
        assert!(!should_autosave(g), "{g:?}");
    }
}

#[test]
fn gate_waits_out_the_interval_and_survives_a_backwards_clock() {
    let g = ready_gate();
    assert!(!should_autosave(AutosaveGate { due_from: g.now - Duration::from_secs(29), ..g }));
    assert!(should_autosave(AutosaveGate { due_from: g.now - Duration::from_secs(30), ..g }));
    assert!(!should_autosave(AutosaveGate { due_from: g.now + Duration::from_secs(5), ..g }));
}

// ---- Wired into the tick ----------------------------------------------

fn project_dir() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "resonance_autosave_trigger_{}_{n}",
        std::process::id()
    ));
    std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
    let dir = root.join("song.rproj");
    std::fs::create_dir_all(&dir).expect("create project dir");
    dir
}

fn app(interval_secs: u32, enabled: bool) -> (Resonance, PathBuf) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let dir = project_dir();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.clone());
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_set_autosave_settings(AutosaveSettings {
        enabled,
        interval_secs,
        backup_retention: 10,
    });
    (app, dir)
}

fn edit(app: &mut Resonance, db: f32) {
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, db)));
}

fn finish_collect(app: &mut Resonance) {
    app.test_apply_engine_event(AudioEvent::ClipsSavedToProjectDir { clip_files: Vec::new() });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved { states: Vec::new() });
}

#[test]
fn a_due_tick_on_a_dirty_project_starts_an_autosave() {
    let (mut app, dir) = app(0, true);
    edit(&mut app, -3.0);

    let _ = app.update(Message::Tick);

    assert_eq!(app.test_save_in_flight(), Some((dir.clone(), true)));
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn an_unchanged_project_is_not_autosaved_again() {
    let (mut app, dir) = app(0, true);
    edit(&mut app, -3.0);
    let _ = app.update(Message::Tick);
    finish_collect(&mut app);
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(Ok(()), true)));
    assert!(app.is_dirty(), "an autosave never cleans");

    let _ = app.update(Message::Tick);
    assert_eq!(app.test_save_in_flight(), None, "nothing changed since the snapshot");

    edit(&mut app, -6.0);
    let _ = app.update(Message::Tick);
    assert_eq!(app.test_save_in_flight(), Some((dir.clone(), true)));
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn the_interval_and_the_enabled_setting_are_honoured() {
    // Default 30 s: the first tick after the edit only starts the clock.
    let (mut app, dir) = app(30, true);
    edit(&mut app, -3.0);
    let _ = app.update(Message::Tick);
    assert_eq!(app.test_save_in_flight(), None);
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());

    let (mut app, dir) = app_disabled();
    edit(&mut app, -3.0);
    let _ = app.update(Message::Tick);
    assert_eq!(app.test_save_in_flight(), None, "autosave disabled");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

fn app_disabled() -> (Resonance, PathBuf) {
    app(0, false)
}

#[test]
fn a_clean_project_is_not_autosaved() {
    let (mut app, dir) = app(0, true);
    let _ = app.update(Message::Tick);
    assert_eq!(app.test_save_in_flight(), None);
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}
