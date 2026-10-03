//! Project-lifecycle UX (code review 2026-10-02, batch U1):
//!
//! - UX-01: a GUI Open / New over unsaved changes asks Save / Don't save /
//!   Cancel instead of discarding them, and "Save" goes through Save As on
//!   an untitled project before carrying on.
//! - UX-03 / STATE2-04: an untitled project records undo — its clips and
//!   snapshots are anchored to the session's scratch dir.
//! - UX-11: the transport's CPU readout polls the engine's published load.
//! - UX-12: undo / redo say what they did.
//! - UX-14: deleting a user track preset takes an armed confirm.
//! - UX-15: uncommitted BPM text reverts on a press off the field.

use std::path::PathBuf;

use resonance_app::commands::CommandId;
use resonance_app::message::{Message, ProjectIoMessage, SwitchChoice, TrackMessage, UiMessage};
use resonance_app::state::{Overlay, ProjectSwitch};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent};

fn active_app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app
}

/// A titled, dirty project at a fresh temp path.
fn dirty_titled(tag: &str) -> (Resonance, PathBuf) {
    let mut app = active_app();
    let dir = std::env::temp_dir().join(format!(
        "resonance-lifecycle-{tag}-{}.rproj",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    app.test_set_project_path(dir.clone());
    app.test_set_dirty(true);
    (app, dir)
}

fn track_count(app: &Resonance) -> usize {
    app.test_registry().tracks.len()
}

/// A capturing app with an active project, for edits that need the
/// engine's echo (a track add lands on `TrackAdded`).
fn capture_app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    (app, rx)
}

/// Answer the track adds / removes the app sent, as the engine does.
fn echo_tracks(app: &mut Resonance, rx: &Receiver<AudioCommand>) {
    let cmds: Vec<AudioCommand> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    for cmd in cmds {
        match cmd {
            AudioCommand::AddTrack { id, .. } => {
                app.test_apply_engine_event(AudioEvent::TrackAdded { track_id: id })
            }
            AudioCommand::RemoveTrack { track_id } => {
                app.test_apply_engine_event(AudioEvent::TrackRemoved { track_id })
            }
            _ => {}
        }
    }
}

fn new_untitled(app: &mut Resonance) {
    let _ = app.update(Message::Ui(UiMessage::NewEmptyProject));
    app.test_apply_engine_event(AudioEvent::AllCleared);
    assert_eq!(app.test_project_path(), None);
}

// ---------------- UX-01: confirm before replacing unsaved work ----------------

#[test]
fn open_over_unsaved_changes_asks_first_and_keeps_the_project() {
    let (mut app, dir) = dirty_titled("open-asks");
    let token = app.test_pending_open_token();
    let target = std::env::temp_dir().join("resonance-lifecycle-other.rproj");

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
        target.display().to_string(),
    ))));

    assert_eq!(app.root_overlay(), Some(Overlay::ConfirmProjectSwitch));
    assert_eq!(app.test_confirm_switch(), Some(&ProjectSwitch::Open(target)));
    assert_eq!(app.test_pending_open_token(), token, "no open started");
    assert_eq!(app.test_project_path(), Some(dir.as_path()));
    assert!(app.test_dirty());
}

#[test]
fn cancel_keeps_the_project_and_closes_the_dialog() {
    let (mut app, dir) = dirty_titled("cancel");
    let token = app.test_pending_open_token();
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
        "/tmp/resonance-lifecycle-cancel-target.rproj".into(),
    ))));
    // Esc closes it like the Cancel button.
    let esc = app.root_overlay().unwrap().dismiss_message(&app).unwrap();
    let _ = app.update(esc);

    assert_eq!(app.root_overlay(), None);
    assert_eq!(app.test_pending_open_token(), token);
    assert_eq!(app.test_project_path(), Some(dir.as_path()));
    assert!(app.test_dirty());
}

#[test]
fn dont_save_carries_out_the_open() {
    let (mut app, _dir) = dirty_titled("discard");
    let token = app.test_pending_open_token();
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
        "/tmp/resonance-lifecycle-discard-target.rproj".into(),
    ))));
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::SwitchChoice(SwitchChoice::Discard)));

    assert_eq!(app.root_overlay(), None);
    assert_ne!(app.test_pending_open_token(), token, "the open started");
}

#[test]
fn save_then_opens_once_the_save_lands() {
    let (mut app, dir) = dirty_titled("save-then-open");
    let token = app.test_pending_open_token();
    let target = PathBuf::from("/tmp/resonance-lifecycle-save-target.rproj");
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
        target.display().to_string(),
    ))));
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::SwitchChoice(SwitchChoice::Save)));

    assert_eq!(app.root_overlay(), None);
    assert_eq!(app.test_save_in_flight().map(|(p, _)| p), Some(dir.clone()));
    assert_eq!(app.test_switch_after_save(), Some(&ProjectSwitch::Open(target)));
    assert_eq!(app.test_pending_open_token(), token, "not before the save lands");

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(Ok(()), false)));
    assert_eq!(app.test_switch_after_save(), None);
    assert_ne!(app.test_pending_open_token(), token, "the open started after the save");
}

#[test]
fn a_failed_save_drops_the_switch() {
    let (mut app, dir) = dirty_titled("save-fails");
    let token = app.test_pending_open_token();
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
        "/tmp/resonance-lifecycle-fail-target.rproj".into(),
    ))));
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::SwitchChoice(SwitchChoice::Save)));
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(
        Err("disk full".into()),
        false,
    )));

    assert_eq!(app.test_switch_after_save(), None);
    assert_eq!(app.test_pending_open_token(), token);
    assert_eq!(app.test_project_path(), Some(dir.as_path()));
}

#[test]
fn new_over_unsaved_changes_asks_instead_of_refusing() {
    let (mut app, dir) = dirty_titled("new-asks");
    assert!(
        CommandId::NewProject.availability(&app).is_yes(),
        "New is offered over unsaved changes: it asks, it doesn't dead-end"
    );
    app.test_run_shortcut(CommandId::NewProject);

    assert_eq!(app.root_overlay(), Some(Overlay::ConfirmProjectSwitch));
    assert_eq!(app.test_confirm_switch(), Some(&ProjectSwitch::NewEmpty));
    assert!(!app.test_error_message_is_set(), "no 'save first' banner");
    assert_eq!(app.test_project_path(), Some(dir.as_path()));

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::SwitchChoice(SwitchChoice::Discard)));
    app.test_apply_engine_event(AudioEvent::AllCleared);
    assert_eq!(app.test_project_path(), None, "a fresh untitled project");
    assert!(!app.test_dirty());
}

#[test]
fn save_on_an_untitled_project_goes_through_save_as_and_its_cancel_cancels() {
    let mut app = active_app();
    new_untitled(&mut app);
    let _ = app.update(Message::Track(TrackMessage::AddTrack));
    assert!(app.test_dirty());

    app.test_run_shortcut(CommandId::NewProject);
    assert_eq!(app.root_overlay(), Some(Overlay::ConfirmProjectSwitch));
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::SwitchChoice(SwitchChoice::Save)));
    // The Save As dialog is up (an rfd task); nothing was replaced.
    assert_eq!(app.test_switch_after_save(), Some(&ProjectSwitch::NewEmpty));
    assert!(app.test_save_in_flight().is_none(), "no path yet: Save As first");

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::SavePathSelected(None)));
    assert_eq!(app.test_switch_after_save(), None);
    assert!(app.test_dirty(), "the work is still open");
}

#[test]
fn a_clean_project_switches_without_asking() {
    let mut app = active_app();
    app.test_set_project_path(PathBuf::from("/tmp/resonance-lifecycle-clean.rproj"));
    let token = app.test_pending_open_token();
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
        "/tmp/resonance-lifecycle-clean-target.rproj".into(),
    ))));
    assert_eq!(app.root_overlay(), None);
    assert_ne!(app.test_pending_open_token(), token);
}

// ---------------- UX-03 / STATE2-04: untitled undo ----------------

#[test]
fn an_untitled_project_records_undo() {
    let (mut app, rx) = capture_app();
    new_untitled(&mut app);
    echo_tracks(&mut app, &rx);
    let before = track_count(&app);

    let _ = app.update(Message::Track(TrackMessage::AddTrack));
    echo_tracks(&mut app, &rx);
    assert_eq!(track_count(&app), before + 1);
    assert!(app.test_can_undo(), "an untitled edit is in the history");

    let _ = app.update(Message::Undo);
    echo_tracks(&mut app, &rx);
    assert_eq!(track_count(&app), before, "undo removed the track");
    assert_eq!(app.test_history_notice(), Some("Undid add track"));

    let _ = app.update(Message::Redo);
    echo_tracks(&mut app, &rx);
    assert_eq!(track_count(&app), before + 1);
    assert_eq!(app.test_history_notice(), Some("Redid add track"));
}

#[test]
fn an_untitled_project_anchors_the_engine_to_its_scratch_dir() {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    let _ = app.update(Message::Ui(UiMessage::NewEmptyProject));
    while rx.try_recv().is_ok() {}
    app.test_apply_engine_event(AudioEvent::AllCleared);

    let anchor = app.test_untitled_anchor().expect("anchored").to_path_buf();
    assert!(anchor.is_dir(), "the scratch dir exists");
    let mut dirs = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        if let AudioCommand::SetProjectDir(dir) = cmd {
            dirs.push(dir);
        }
    }
    assert_eq!(
        dirs.last(),
        Some(&anchor),
        "recordings, imports and undo WAVs land in the scratch dir, not the cwd"
    );
}

#[test]
fn the_first_save_carries_the_scratch_clip_wavs_into_the_bundle() {
    let mut app = active_app();
    new_untitled(&mut app);
    let scratch = app.test_untitled_anchor().unwrap().to_path_buf();
    // A clip an untitled undo entry still names, persisted before a delete.
    std::fs::create_dir_all(scratch.join("audio")).unwrap();
    std::fs::write(scratch.join("audio/clip_4242.wav"), b"RIFF").unwrap();

    let bundle = std::env::temp_dir().join(format!(
        "resonance-lifecycle-carry-{}.rproj",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&bundle);
    std::fs::create_dir_all(&bundle).unwrap();
    app.test_set_project_path(bundle.clone());
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(Ok(()), false)));

    assert!(bundle.join("audio/clip_4242.wav").exists());
    assert!(!scratch.exists(), "the superseded scratch dir is removed");
    assert_eq!(app.test_untitled_anchor(), None);
    let _ = std::fs::remove_dir_all(&bundle);
}

// ---------------- UX-11: CPU readout ----------------

#[test]
fn the_cpu_readout_polls_the_published_load() {
    let mut app = active_app();
    let _ = app.update(Message::Tick);
    assert_eq!(app.test_cpu_load(), None, "no cycle measured yet");

    app.test_publish_dsp_load(0.42, 0.6);
    let _ = app.update(Message::Tick);
    let load = app.test_cpu_load().expect("polled");
    assert!((load.smoothed - 0.42).abs() < 1e-6);
    assert!((load.peak - 0.6).abs() < 1e-6);
}

// ---------------- UX-12: history feedback ----------------

#[test]
fn the_palette_names_the_edit_undo_would_step_over() {
    let (mut app, rx) = capture_app();
    new_untitled(&mut app);
    let _ = app.update(Message::Track(TrackMessage::AddTrack));
    echo_tracks(&mut app, &rx);
    let _ = app.update(Message::Ui(UiMessage::OpenPalette(
        resonance_app::palette::PaletteMode::Commands,
    )));
    let _ = app.update(Message::Ui(UiMessage::Palette(
        resonance_app::palette::PaletteMsg::Query("undo".into()),
    )));
    let names: Vec<String> = app
        .test_palette()
        .expect("palette open")
        .rows()
        .map(|r| r.name.clone())
        .collect();
    assert!(
        names.iter().any(|n| n == "Undo add track"),
        "rows: {names:?}"
    );
}

#[test]
fn the_history_notice_expires() {
    let mut app = active_app();
    new_untitled(&mut app);
    let _ = app.update(Message::Track(TrackMessage::AddTrack));
    let _ = app.update(Message::Undo);
    assert!(app.test_history_notice().is_some());
    app.test_expire_history_notice(std::time::Instant::now() + std::time::Duration::from_secs(5));
    assert_eq!(app.test_history_notice(), None);
}

// ---------------- UX-14: preset delete confirm ----------------

#[test]
fn a_preset_delete_is_armed_first_and_closing_the_menu_disarms_it() {
    let mut app = active_app();
    let _ = app.update(Message::Ui(UiMessage::OpenAddTrackMenu));
    let _ = app.update(Message::Ui(UiMessage::ArmPresetDelete(Some("Lead".into()))));
    assert_eq!(app.test_preset_delete_armed(), Some("Lead"));
    let _ = app.update(Message::Ui(UiMessage::ArmPresetDelete(None)));
    assert_eq!(app.test_preset_delete_armed(), None, "Keep disarms");

    let _ = app.update(Message::Ui(UiMessage::ArmPresetDelete(Some("Lead".into()))));
    let _ = app.update(Message::Ui(UiMessage::CloseAddTrackMenu));
    assert_eq!(app.test_preset_delete_armed(), None, "a confirm never outlives its menu");
}

// ---------------- UX-15: BPM blur ----------------

#[test]
fn uncommitted_bpm_text_reverts_on_a_press_off_the_field() {
    use resonance_app::message::TransportMessage;
    let mut app = active_app();
    let bpm = app.test_transport_bpm();
    let _ = app.update(Message::Transport(TransportMessage::SetBpmText("95".into())));
    assert!(app.test_bpm_editing());

    // A press on the field itself keeps the text.
    let _ = app.update(Message::Ui(UiMessage::BpmFieldHovered(true)));
    let _ = app.update(Message::Ui(UiMessage::BpmFieldPointer));
    assert_eq!(app.test_bpm_input(), "95");

    // A press elsewhere reverts it to the tempo the song plays at.
    let _ = app.update(Message::Ui(UiMessage::BpmFieldHovered(false)));
    let _ = app.update(Message::Ui(UiMessage::BpmFieldPointer));
    assert_eq!(app.test_bpm_input(), format!("{bpm:.1}"));
    assert!(!app.test_bpm_editing());
    assert_eq!(app.test_transport_bpm(), bpm, "nothing was committed");
}

#[test]
fn committing_bpm_ends_the_edit() {
    use resonance_app::message::TransportMessage;
    let mut app = active_app();
    let _ = app.update(Message::Transport(TransportMessage::SetBpmText("95".into())));
    let _ = app.update(Message::Transport(TransportMessage::CommitBpm));
    assert!(!app.test_bpm_editing());
    assert_eq!(app.test_transport_bpm(), 95.0);
}

// ---------------- goldens ----------------

const WINDOW: (f32, f32) = (1440.0, 900.0);

fn sim_settings() -> iced::Settings {
    use resonance_app::theme;
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(theme::ICON_FONT_BYTES.into());
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    }
}

fn snapshot(app: &Resonance, golden: &str) {
    let mut ui = iced_test::simulator::Simulator::with_size(
        sim_settings(),
        iced::Size::new(WINDOW.0, WINDOW.1),
        app.view(),
    );
    let snap = ui
        .snapshot(&resonance_app::theme::resonance_theme())
        .expect("snapshot should render");
    crate::common::assert_golden(&snap, golden);
}

/// A demo project on the Arrange tab, titled "Demo Song", with no edits.
fn demo_app() -> Resonance {
    let (mut app, _task) =
        Resonance::new_for_test_on(resonance_app::state::ViewMode::Arrange);
    resonance_app::demo::seed_demo_content(&mut app);
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/tmp/resonance-golden/Demo Song.rproj"));
    app.test_set_dirty(false);
    app
}

/// UX-01: Ctrl+O over unsaved changes — the Save / Don't Save / Cancel
/// dialog names both projects.
#[test]
fn project_switch_confirm_golden() {
    let mut app = demo_app();
    app.test_set_dirty(true);
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
        "/tmp/resonance-golden/Live Set.rproj".into(),
    ))));
    assert_eq!(app.root_overlay(), Some(Overlay::ConfirmProjectSwitch));
    snapshot(&app, "tests/snapshots/project_switch_confirm.png");
}

/// UX-11 / UX-12 / UX-16: the chrome right after an undo (the "Undid …"
/// notice), and the transport with a near-budget CPU load (WARM) and the
/// SIG control under the pointer (hover fill).
#[test]
fn transport_cpu_warm_and_undo_notice_golden() {
    use resonance_app::message::TransportMessage;
    let mut app = demo_app();
    let _ = app.update(Message::Transport(TransportMessage::ToggleLoop));
    let _ = app.update(Message::Undo);
    assert_eq!(app.test_history_notice(), Some("Undid loop"));
    app.test_publish_dsp_load(0.82, 0.9);
    let _ = app.update(Message::Tick);
    snapshot(&app, "tests/snapshots/transport_cpu_warm_undo_notice.png");
}

/// UX-03: an untitled project's chrome says "not saved", never "saved".
#[test]
fn untitled_chrome_says_not_saved_golden() {
    let mut app = active_app();
    new_untitled(&mut app);
    snapshot(&app, "tests/snapshots/untitled_chrome_not_saved.png");
}

/// UX-14: a user preset's delete button armed — the row turns into an
/// inline "Delete …?" confirm with Keep / Delete.
#[test]
fn add_track_menu_preset_delete_armed_golden() {
    let mut app = demo_app();
    let preset = |name: &str| -> resonance_app::presets::TrackPreset {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "track_type": "instrument",
            "volume": 1.0,
            "pan": 0.0,
            "mono": false,
        }))
        .expect("preset json")
    };
    app.test_set_user_presets(vec![preset("Warm Pad"), preset("Fuzz Bass")]);
    let _ = app.update(Message::Ui(UiMessage::OpenAddTrackMenu));
    let _ = app.update(Message::Ui(UiMessage::ArmPresetDelete(Some("Fuzz Bass".into()))));
    snapshot(&app, "tests/snapshots/add_track_menu_preset_delete_armed.png");
}
