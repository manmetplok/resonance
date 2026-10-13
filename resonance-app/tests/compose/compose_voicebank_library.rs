//! The voicebank library overlay (vocal-mcp-control-and-quality.md §5,
//! D15): opened from the vocal lane's Library… button, tabs `Installed |
//! plok.org`, a download that only starts after its terms are accepted,
//! progress with Cancel, Update and Delete with a confirmation — and the
//! golden images of those states.
//!
//! Each test serves its own index from a loopback server and installs into
//! its own folders, so nothing touches the network or the user's data.

use crate::common;
use crate::common::plok_server::{sha256_hex, zip_dir, PlokServer};

use std::time::Duration;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::compose::vocal_svs::registry::InstallPhase;
use resonance_app::compose::vocal_svs::VoicebankRoots;
use resonance_app::compose::{ComposeMessage, LaneGeneratorKind, RailPanelKey, SelectedLane};
use resonance_app::message::{Message, VoicebankLibraryMessage as M};
use resonance_app::state::voicebank_library::{LibraryTab, RowStatus};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, write_voicebank_fixture, Resonance, VoicebankFixture};
use serde_json::json;

const WINDOW: (f32, f32) = (1440.0, 900.0);
/// The demo's Lead Vocal track.
const LEAD_VOCAL: u64 = 6;
const SETTLE: Duration = Duration::from_secs(20);

fn sim_settings() -> iced::Settings {
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

fn snapshot(app: &Resonance, path: &str, expect_text: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    assert!(ui.find(expect_text).is_ok(), "the overlay shows {expect_text:?}");
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

fn send(app: &mut Resonance, m: M) {
    let _ = app.update(Message::VoicebankLibrary(m));
}

/// A zipped single-voice fixture bank with its own vocoder.
fn bank_zip(name: &str) -> Vec<u8> {
    let tmp = tempfile::tempdir().unwrap();
    let spec = VoicebankFixture {
        name: Some(name.to_owned()),
        ..VoicebankFixture::default()
    };
    write_voicebank_fixture(tmp.path(), &spec);
    zip_dir(tmp.path(), name)
}

/// An index with two banks: Aurora (installable) and Lilia (installed
/// here from the dev fixture, so it reads Installed).
fn serve() -> (PlokServer, Vec<u8>) {
    let server = PlokServer::start();
    let aurora = bank_zip("Aurora");
    let index = json!({
        "voicebanks": [
            {
                "id": "aurora",
                "name": "Aurora",
                "version": "v2.1",
                "languages": ["en", "ja"],
                "speakers": ["soft", "power"],
                "size": "412 MB",
                "sha256": sha256_hex(&aurora),
                "url": server.url("/upstream/Aurora_v2.1.zip"),
                "terms_url": "https://example.invalid/aurora/terms",
                "credit": "Aurora from Example Project",
                "commercial": "permission",
                "added": "2026-10-07"
            },
            {
                "id": "lilia",
                "name": "LIEE Lilia",
                "version": "MM 2.8",
                "languages": ["en"],
                "size": "609 MB",
                "url": server.url("/upstream/lilia.zip"),
                "terms_url": "https://example.invalid/liee/terms",
                "credit": "LIEE",
                "commercial": "free_with_credit"
            }
        ]
    });
    server.put("/index.json", serde_json::to_vec(&index).unwrap());
    server.put("/upstream/Aurora_v2.1.zip", aurora.clone());
    (server, aurora)
}

/// The demo on the Compose tab with the Lead Vocal lane selected (its
/// draft and melody cards folded) and switched to Lilia, on private
/// voicebank folders holding a Lilia stand-in, with the library pointed at
/// `server`. Returns the lane.
fn demo_with_library(root: &std::path::Path, server: &PlokServer) -> (Resonance, u64) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    demo::seed_demo_content(&mut app);
    app.test_set_active_project(true);
    app.test_set_voicebank_roots(VoicebankRoots {
        banks: Some(root.join("voicebanks")),
        vocoders: Some(root.join("vocoders")),
        dev_models: Vec::new(),
    });
    write_voicebank_fixture(
        &root.join("voicebanks/lilia"),
        &VoicebankFixture::shipped("lilia"),
    );
    app.test_rescan_voicebanks();
    app.test_set_voicebank_index(&server.url("/index.json"));
    let _ = app.update(Message::Compose(ComposeMessage::SelectLane(
        SelectedLane::Instrument(LEAD_VOCAL),
    )));
    for key in [RailPanelKey::VocalDraft(LEAD_VOCAL), RailPanelKey::VocalMelody(LEAD_VOCAL)] {
        let _ = app.update(Message::Compose(ComposeMessage::ToggleRailPanel(key)));
    }
    let def = app
        .compose_state()
        .selected_placement()
        .expect("demo selects a placement")
        .definition_id;
    // A new lane sings TIGER, which is not installed here: put the lane on
    // the installed Lilia.
    let _ = app.update(Message::Compose(ComposeMessage::LaneInspector {
        definition_id: def,
        track_id: LEAD_VOCAL,
        msg: resonance_app::compose::messages::LaneInspectorMsg::SetVocalVoicebank(
            resonance_music_theory::VoicebankId::new("lilia"),
        ),
    }));
    (app, def)
}

fn open(app: &mut Resonance, def: u64) {
    send(
        app,
        M::Open {
            definition_id: def,
            track_id: LEAD_VOCAL,
        },
    );
    assert!(app.test_settle_voicebank_library(SETTLE), "the index fetch settles");
}

fn lane_bank(app: &Resonance, def: u64) -> String {
    match &app.compose_state().find_definition(def).unwrap().lane_generators[&LEAD_VOCAL].kind {
        LaneGeneratorKind::Vocal(p) => p.voicebank.as_str().to_owned(),
        _ => panic!("a vocal lane"),
    }
}

#[test]
fn library_button_opens_the_overlay_on_the_lane_bank() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, _) = serve();
    let (mut app, def) = demo_with_library(tmp.path(), &server);
    // Tall enough that the rail shows the voice card without scrolling.
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, 1600.0), app.view());
    ui.click("Library\u{2026}").expect("the lane inspector has a Library… button");
    let messages: Vec<Message> = ui.into_messages().collect();
    for message in messages {
        let _ = app.update(message);
    }
    assert!(app.test_settle_voicebank_library(SETTLE));
    let o = app.test_voicebank_overlay().expect("the overlay is open");
    assert_eq!(o.definition_id, def);
    assert_eq!(o.tab, LibraryTab::Installed);
    assert_eq!(o.installed.selected(), Some("voicebank:lilia"), "the lane's bank");
}

#[test]
fn a_download_starts_only_after_its_terms_are_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, _) = serve();
    let (mut app, def) = demo_with_library(tmp.path(), &server);
    open(&mut app, def);
    send(&mut app, M::SetTab(LibraryTab::Plok));
    send(&mut app, M::Select("voicebank:aurora".into()));
    let row = |app: &Resonance| {
        app.test_voicebank_overlay().unwrap().plok_rows.by_id("aurora").cloned().unwrap()
    };
    assert_eq!(row(&app).status, RowStatus::NotInstalled);

    // Download shows the terms; nothing is fetched yet.
    send(&mut app, M::Download("aurora".into()));
    assert_eq!(app.test_voicebank_overlay().unwrap().accept.as_deref(), Some("aurora"));
    assert!(server.requests("/upstream/Aurora_v2.1.zip").is_empty());

    // Declining drops it.
    send(&mut app, M::DeclineTerms);
    assert!(app.test_voicebank_overlay().unwrap().accept.is_none());
    assert!(app.test_settle_voicebank_library(SETTLE));
    assert!(server.requests("/upstream/Aurora_v2.1.zip").is_empty());

    // Accepting downloads and installs it.
    send(&mut app, M::Download("aurora".into()));
    send(&mut app, M::AcceptTerms);
    assert!(app.test_settle_voicebank_library(SETTLE));
    assert_eq!(server.requests("/upstream/Aurora_v2.1.zip").len(), 1);
    assert!(app.test_installed_voicebanks().contains(&"aurora".to_owned()));
    assert_eq!(row(&app).status, RowStatus::Installed);

    // Use sets the lane's bank, as one undoable lane edit.
    send(&mut app, M::Use("aurora".into()));
    assert!(app.test_voicebank_overlay().is_none(), "Use closes the overlay");
    assert_eq!(lane_bank(&app, def), "aurora");
}

#[test]
fn delete_asks_first_and_removes_the_user_install() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, _) = serve();
    let (mut app, def) = demo_with_library(tmp.path(), &server);
    open(&mut app, def);
    send(&mut app, M::Delete("lilia".into()));
    assert_eq!(
        app.test_voicebank_overlay().unwrap().confirm_delete.as_deref(),
        Some("lilia")
    );
    send(&mut app, M::CancelDelete);
    assert!(tmp.path().join("voicebanks/lilia").is_dir());
    send(&mut app, M::Delete("lilia".into()));
    send(&mut app, M::ConfirmDelete);
    assert!(!tmp.path().join("voicebanks/lilia").exists());
    assert!(!app.test_installed_voicebanks().contains(&"lilia".to_owned()));
}

#[test]
fn cancel_stops_an_install() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, _) = serve();
    let (mut app, def) = demo_with_library(tmp.path(), &server);
    open(&mut app, def);
    send(&mut app, M::SetTab(LibraryTab::Plok));
    // A frozen install shows Cancel; Cancel raises its flag.
    app.test_show_voicebank_installing(
        "aurora",
        InstallPhase::Downloading {
            what: "Aurora".into(),
            done: 1,
            total: 4,
        },
    );
    send(&mut app, M::Cancel("aurora".into()));
    let s = app.test_voicebank_library().state();
    assert!(s.job("aurora").unwrap().cancel.load(std::sync::atomic::Ordering::SeqCst));
}

/// A running install's progress patches its row's status in place: the
/// rows are not rebuilt, the browsers do not re-filter, and no disk is
/// read. Typing in the search field re-filters without a rebuild either.
#[test]
fn progress_and_typing_patch_rather_than_rebuild_the_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, _) = serve();
    let (mut app, def) = demo_with_library(tmp.path(), &server);
    open(&mut app, def);
    send(&mut app, M::SetTab(LibraryTab::Plok));
    app.test_show_voicebank_installing(
        "aurora",
        InstallPhase::Downloading {
            what: "Aurora".into(),
            done: 1,
            total: 4,
        },
    );
    let generations = |app: &Resonance| {
        let o = app.test_voicebank_overlay().unwrap();
        (o.rows_generation, o.status_generation)
    };
    let (rows, statuses) = generations(&app);
    app.test_set_voicebank_install_phase(
        "aurora",
        InstallPhase::Downloading {
            what: "Aurora".into(),
            done: 3,
            total: 4,
        },
    );
    assert_eq!(generations(&app), (rows, statuses + 1));
    let row = app
        .test_voicebank_overlay()
        .unwrap()
        .plok_rows
        .by_id("aurora")
        .cloned()
        .unwrap();
    assert!(
        matches!(row.status, RowStatus::Installing { fraction: Some(f), .. } if (f - 0.675).abs() < 1e-3),
        "{:?}",
        row.status
    );
    // A tick with nothing new does nothing.
    app.test_poll_voicebank_library();
    assert_eq!(generations(&app), (rows, statuses + 1));

    // Typing filters the browser over the same rows.
    send(&mut app, M::Search("aur".into()));
    assert_eq!(generations(&app).0, rows);
    let o = app.test_voicebank_overlay().unwrap();
    assert_eq!(o.plok.view_len(), 1, "the query filtered the view");
    send(&mut app, M::Select("voicebank:aurora".into()));
    assert_eq!(generations(&app).0, rows);
}

// ---------------------------------------------------------------------------
// Golden images
// ---------------------------------------------------------------------------

#[test]
fn voicebank_library_installed_tab() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, _) = serve();
    let (mut app, def) = demo_with_library(tmp.path(), &server);
    open(&mut app, def);
    snapshot(
        &app,
        "tests/snapshots/compose_voicebank_library_installed.png",
        "Use on this lane",
    );
}

#[test]
fn voicebank_library_plok_tab() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, _) = serve();
    let (mut app, def) = demo_with_library(tmp.path(), &server);
    open(&mut app, def);
    send(&mut app, M::SetTab(LibraryTab::Plok));
    send(&mut app, M::Select("voicebank:aurora".into()));
    snapshot(
        &app,
        "tests/snapshots/compose_voicebank_library_plok.png",
        "Download\u{2026}",
    );
}

#[test]
fn voicebank_library_accept_terms() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, _) = serve();
    let (mut app, def) = demo_with_library(tmp.path(), &server);
    open(&mut app, def);
    send(&mut app, M::SetTab(LibraryTab::Plok));
    send(&mut app, M::Select("voicebank:aurora".into()));
    send(&mut app, M::Download("aurora".into()));
    snapshot(
        &app,
        "tests/snapshots/compose_voicebank_library_accept_terms.png",
        "Accept & download",
    );
}

#[test]
fn voicebank_library_downloading() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, _) = serve();
    let (mut app, def) = demo_with_library(tmp.path(), &server);
    open(&mut app, def);
    send(&mut app, M::SetTab(LibraryTab::Plok));
    send(&mut app, M::Select("voicebank:aurora".into()));
    app.test_show_voicebank_installing(
        "aurora",
        InstallPhase::Downloading {
            what: "Aurora".into(),
            done: 180 * 1024 * 1024,
            total: 412 * 1024 * 1024,
        },
    );
    snapshot(
        &app,
        "tests/snapshots/compose_voicebank_library_downloading.png",
        "Cancel",
    );
}

#[test]
fn voicebank_library_confirm_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, _) = serve();
    let (mut app, def) = demo_with_library(tmp.path(), &server);
    open(&mut app, def);
    send(&mut app, M::Delete("lilia".into()));
    snapshot(
        &app,
        "tests/snapshots/compose_voicebank_library_confirm_delete.png",
        "Keep",
    );
}
