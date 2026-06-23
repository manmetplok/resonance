//! Golden-image snapshots for the **populated Reference & A/B panel**
//! (todo #700 / design doc #184/#198).
//!
//! With a reference loaded and selected active, the panel shows the
//! reference list, the Mix/Reference A/B switch, the waveform overview
//! (playhead + marker tick), the loudness-match toggle and the level
//! trim. Two snapshots lock both monitor sources:
//!
//! * **Mix-active** — the A segment lit lavender (the default monitor).
//! * **Reference-active** — the B segment lit amber after switching.
//!
//! Text selectors assert the controls are present independently of pixels,
//! then a golden snapshot locks the layout.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::reference::ReferenceMessage;
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_audio::types::{ABSource, AudioEvent, ReferenceId};

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

/// A gentle ramped waveform overview so the snapshot draws a real shape
/// rather than a flat line.
fn demo_peaks() -> Vec<(f32, f32)> {
    (0..96)
        .map(|i| {
            let t = i as f32 / 96.0;
            let amp = 0.25 + 0.65 * (t * std::f32::consts::PI).sin();
            (-amp, amp)
        })
        .collect()
}

/// Open the Mixer view with the reference rail showing and a single loaded
/// reference selected active, its cursor parked mid-track with one marker.
fn app_with_active_reference() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Mixer);
    let (mut app, _task) = Resonance::new();
    demo::seed_minimal_drum_track_no_busses(&mut app);
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    let _ = app.update(Message::Ui(UiMessage::ToggleReferencePanel));

    app.test_reference_push_pending("/refs/Reference Master.wav");
    app.test_handle_engine_event(AudioEvent::ReferenceLoaded {
        id: ReferenceId(1),
        name: "Reference Master".to_string(),
        path: "/refs/Reference Master.wav".to_string(),
        integrated_lufs: -9.4,
        waveform_peaks: demo_peaks(),
        length_samples: 480_000,
    });
    // Park the cursor mid-track and drop a marker so the waveform draws a
    // playhead and a tick.
    app.test_handle_engine_event(AudioEvent::RefPositionChanged {
        ref_id: ReferenceId(1),
        position_samples: 240_000,
    });
    app.test_handle_engine_event(AudioEvent::RefMarkerAdded {
        ref_id: ReferenceId(1),
        marker_id: 1,
        position_samples: 120_000,
        label: "Drop".to_string(),
    });

    // Select it active so the A/B detail controls render.
    let _ = app.update(Message::Reference(ReferenceMessage::SetActive(ReferenceId(1))));
    app
}

#[test]
fn reference_panel_populated_mix_active() {
    let app = app_with_active_reference();

    let mut ui = simulator(&app);
    ui.find("Reference Master")
        .expect("the loaded reference is listed");
    ui.find("Match loudness")
        .expect("populated panel shows the loudness-match toggle");
    ui.find("Trim").expect("populated panel shows the trim control");
    ui.find("Drop").expect("the comparison marker chip is shown");

    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image("tests/snapshots/reference_panel_populated_mix_active.png")
            .expect("matches_image i/o"),
        "snapshot diverged from golden"
    );
}

#[test]
fn reference_panel_populated_reference_active() {
    let mut app = app_with_active_reference();

    // Switch the monitor to the reference: the B segment lights amber.
    let _ = app.update(Message::Reference(ReferenceMessage::SetAbSource(
        ABSource::Reference,
    )));
    assert_eq!(app.test_reference().ab_source, ABSource::Reference);

    let mut ui = simulator(&app);
    ui.find("Reference Master")
        .expect("the loaded reference is listed");
    ui.find("Add marker")
        .expect("populated panel offers an add-marker action");

    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image("tests/snapshots/reference_panel_populated_reference_active.png")
            .expect("matches_image i/o"),
        "snapshot diverged from golden"
    );
}
