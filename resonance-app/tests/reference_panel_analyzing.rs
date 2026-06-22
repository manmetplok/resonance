//! Golden-image snapshot for the **Reference & A/B panel — Analyzing state**
//! (todo #699 / design doc #184/#198).
//!
//! Drives a reference part-way through its offline analysis (a pending load
//! plus a `ReferenceAnalysisProgress` event) so the panel routes to its
//! **Analyzing** body: the four-stage checklist, the determinate progress
//! bar, and the Cancel action. Text selectors lock the body's presence
//! independently of pixels, then a golden snapshot locks its layout.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioEvent, ReferenceAnalysisStage, ReferenceId};

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

#[test]
fn reference_panel_analyzing() {
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

    // Simulate a dispatched load (queues its path) followed by the engine's
    // first analysis-progress event, which registers a provisional entry
    // mid-analysis — exactly the Analyzing state we want to render. Stop at
    // `MeasuringLufs` so the checklist shows one stage done, one in progress.
    app.test_reference_push_pending("/refs/Reference Master.wav");
    app.test_handle_engine_event(AudioEvent::ReferenceAnalysisProgress {
        id: ReferenceId(1),
        stage: ReferenceAnalysisStage::MeasuringLufs,
    });

    let mut ui = simulator(&app);
    ui.find("Analyzing Reference Master\u{2026}")
        .expect("analyzing panel shows the per-file title");
    ui.find("Measuring loudness")
        .expect("analyzing panel shows the stage checklist");
    ui.find("Cancel")
        .expect("analyzing panel offers a cancel action");

    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image("tests/snapshots/reference_panel_analyzing.png")
            .expect("matches_image i/o"),
        "snapshot diverged from golden"
    );
}
