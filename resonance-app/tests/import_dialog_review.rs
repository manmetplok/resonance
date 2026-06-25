//! Golden-image snapshot for the **Import modal — Review stage**
//! (design doc #158, todo #506).
//!
//! The Review stage is the populated default of the Import modal: a
//! summary band surfacing what the parser found (SMF format, track count,
//! PPQ, length, tempo range), then the "Tracks to import" list of
//! per-track checkbox rows — kind swatch, inline rename, channel chip, a
//! note-count + pitch-range readout, and a slot reserved for the mini
//! piano-roll preview (todo #511). All / None toggles + a live selected
//! count sit above the list, and selected rows carry the `ACCENT_DIM`
//! wash + filled checkbox shared with the export modal's track rows.
//!
//! The sibling export modal locks its stages with golden PNGs
//! (`export_dialog_shell.rs`); this mirrors that so a layout/style/render
//! regression in the ~430 lines of Review view code is caught. The state
//! is reached through the real `ImportMessage` reducer path: `Open` →
//! `FileChosen` → `ParseCompleted(Ok(..))` with a conductor/tempo row
//! plus instrument, drum and vocal tracks and a full `ImportSummary`.
//!
//! Goldens diverge in this local env, so the golden is authored here and
//! blessed in CI (as with the recent vocal-rail snapshot).

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{ImportMessage, Message};
use resonance_app::state::{
    ImportStage, ImportSummary, ImportTrackKind, ParsedImport, TrackImportRow, ViewMode,
};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

/// Window size matches the app's default & minimum window per the design
/// guidelines, same as the other `iced_test` snapshots.
const WINDOW: (f32, f32) = (1440.0, 900.0);

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

/// A real, importable track row (selected by default).
fn track(
    name: &str,
    channel: u8,
    kind: ImportTrackKind,
    note_count: usize,
    pitch_min: u8,
    pitch_max: u8,
) -> TrackImportRow {
    TrackImportRow {
        selected: true,
        name: name.to_string(),
        channel,
        kind,
        note_count,
        pitch_min: Some(pitch_min),
        pitch_max: Some(pitch_max),
        is_conductor: false,
        preview: Vec::new(),
    }
}

/// The notes-less Conductor / tempo track, rendered disabled with the
/// "tempo & meter only" note.
fn conductor() -> TrackImportRow {
    TrackImportRow {
        selected: false,
        name: "Conductor".to_string(),
        channel: 0,
        kind: ImportTrackKind::Instrument,
        note_count: 0,
        pitch_min: None,
        pitch_max: None,
        is_conductor: true,
        preview: Vec::new(),
    }
}

/// A Format-1 parse with a tempo range (120–140 BPM), one disabled
/// conductor row and one of each importable kind so every swatch + the
/// selected-row wash and the channel/readout chips are exercised.
fn parsed() -> ParsedImport {
    let rows = vec![
        conductor(),
        track("Lead Synth", 0, ImportTrackKind::Instrument, 184, 48, 84),
        track("Drum Kit", 9, ImportTrackKind::Drum, 512, 36, 51),
        track("Lilia (Vocal)", 1, ImportTrackKind::Vocal, 96, 55, 72),
    ];
    let total_notes = rows.iter().map(|r| r.note_count).sum();
    ParsedImport {
        summary: ImportSummary {
            file_name: "ballad_in_6-8.mid".to_string(),
            smf_format: Some(1),
            track_count: rows.len(),
            ppq: Some(480),
            length_bars: Some(64),
            total_notes,
            file_tempo_bpm: Some(132.0),
            tempo_bpm_min: Some(120.0),
            tempo_bpm_max: Some(140.0),
            tempo_conflict: false,
        },
        rows,
    }
}

/// Demo app pinned to the Compose tab with the Import modal driven to its
/// Review stage through the real reducer, so the snapshot has
/// representative content dimmed behind the overlay.
fn build_app_with_import_review() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Compose);
    let (mut app, _task) = Resonance::new();
    // Seeds tracks/clips and flips `has_active_project` so the import
    // overlay isn't gated by the startup modal.
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Import(ImportMessage::Open));
    let _ = app.update(Message::Import(ImportMessage::FileChosen(
        "/tmp/ballad_in_6-8.mid".into(),
    )));
    let _ = app.update(Message::Import(ImportMessage::ParseCompleted(Ok(parsed()))));
    app
}

/// Review stage: summary band + the four-row tracks-to-import list (one
/// disabled conductor, three selected instrument/drum/vocal rows).
#[test]
fn import_dialog_review_stage() {
    let app = build_app_with_import_review();
    let dialog = app
        .test_import_dialog()
        .expect("modal open after Open → FileChosen → ParseCompleted");
    assert_eq!(
        dialog.stage,
        ImportStage::Review,
        "a conflict-free parse should land on the Review stage",
    );
    assert_eq!(dialog.rows.len(), 4, "conductor + three importable rows");
    assert_eq!(
        dialog.importable_count(),
        3,
        "the conductor row is not importable",
    );

    let mut ui = Simulator::with_size(
        sim_settings(),
        Size::new(WINDOW.0, WINDOW.1),
        app.view(),
    );
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image("tests/snapshots/import_dialog_review.png")
            .expect("matches_image i/o"),
        "snapshot diverged from golden: tests/snapshots/import_dialog_review.png",
    );
}
