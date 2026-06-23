//! Golden-image snapshot for the **Reference & A/B panel — Error state**
//! (todo #702 / design doc #184/#198).
//!
//! Two failure surfaces are covered. First, a project restored with a
//! now-missing reference file keeps a `Missing` entry and renders an inline
//! BAD-tinted card (filename, "File not found", Dismiss + Choose another) —
//! golden-snapshotted. Second, a load that failed before any entry existed
//! (`ReferenceLoadFailed`) takes the whole panel via `last_error` —
//! selector-checked. Text selectors lock each body independently of pixels.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::project::{ProjectFile, ProjectReference, ProjectReferenceSettings};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_audio::types::AudioEvent;

/// Default & minimum window size per the design guidelines, matching the
/// other `iced_test` integration tests in this crate.
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

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view())
}

/// Open the Mixer view with the reference rail toggled on, on a freshly
/// seeded app — the common preamble for both error scenarios below.
fn open_reference_panel() -> Resonance {
    // Pin the startup tab to Mixer so `view()` lands on `view_mixer`,
    // where the reference rail and its chrome toggle live.
    let _ = STARTUP_TAB.set(ViewMode::Mixer);

    let (mut app, _task) = Resonance::new();
    demo::seed_minimal_drum_track_no_busses(&mut app);

    // Belt-and-braces in case another test in this binary already set the
    // OnceLock to a different tab.
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    // The rail is hidden until the chrome "REF" toggle is pressed.
    let _ = app.update(Message::Ui(UiMessage::ToggleReferencePanel));
    app
}

#[test]
fn reference_panel_missing_entry() {
    let mut app = open_reference_panel();

    // Restore a project whose only reference file has gone missing: the
    // restore keeps a `Missing` entry (name + path preserved) so the panel
    // routes to its populated body and renders the inline error card.
    let file = ProjectFile {
        references: vec![ProjectReference {
            path: "/no/such/Reference Master.wav".into(),
            name: "Reference Master".into(),
            integrated_lufs: -10.0,
            markers: vec![],
        }],
        reference_settings: ProjectReferenceSettings {
            active: Some(0),
            ..ProjectReferenceSettings::default()
        },
        ..ProjectFile::default()
    };
    app.test_restore_references(&file);

    let mut ui = simulator(&app);
    ui.find("Reference Master")
        .expect("error card shows the reference filename");
    ui.find("File not found")
        .expect("missing entry explains the failure");
    ui.find("Dismiss").expect("error card offers Dismiss");
    ui.find("Choose another\u{2026}")
        .expect("error card offers Choose another");

    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image("tests/snapshots/reference_panel_error.png")
            .expect("matches_image i/o"),
        "snapshot diverged from golden"
    );
}

#[test]
fn reference_panel_load_failed_takes_whole_panel() {
    let mut app = open_reference_panel();

    // A load that failed before an entry was registered surfaces through
    // `last_error`; with no entries to attach it to, it owns the panel.
    app.test_handle_engine_event(AudioEvent::ReferenceLoadFailed {
        path: "/refs/broken.wav".into(),
        reason: "unsupported codec".into(),
    });

    let mut ui = simulator(&app);
    ui.find("Couldn\u{2019}t load reference")
        .expect("load-failure body shows its heading");
    ui.find("Dismiss")
        .expect("load-failure body offers Dismiss");
    ui.find("Choose another\u{2026}")
        .expect("load-failure body offers Choose another");
}
