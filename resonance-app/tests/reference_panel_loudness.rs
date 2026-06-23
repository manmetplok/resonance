//! Golden-image snapshot for the **comparative loudness readout** at the
//! bottom of the populated Reference & A/B panel (todo #701 / design doc
//! #184/#198).
//!
//! With a reference loaded and active and a fresh `ABMeterSnapshot` folded
//! in, the readout shows the dual mix/reference loudness-bar pair on a
//! shared LUFS scale (with the target line) above the Mix / Ref / Δ table
//! of integrated, short-term and momentary LUFS, the true-peak max and the
//! LRA. Text selectors assert the rows/columns are present independently of
//! pixels, then a golden snapshot locks the layout.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::reference::ReferenceMessage;
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioEvent, ReferenceId};
use resonance_metering::MeterSnapshot;

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

/// A gentle ramped waveform overview so the panel above the readout draws a
/// real shape rather than a flat line.
fn demo_peaks() -> Vec<(f32, f32)> {
    (0..96)
        .map(|i| {
            let t = i as f32 / 96.0;
            let amp = 0.25 + 0.65 * (t * std::f32::consts::PI).sin();
            (-amp, amp)
        })
        .collect()
}

/// A realistic mix snapshot: a touch quiet versus the -14 LUFS target, with
/// a true-peak comfortably below clipping.
fn mix_meter() -> MeterSnapshot {
    MeterSnapshot {
        momentary_lufs: -13.2,
        short_term_lufs: -14.8,
        integrated_lufs: -16.1,
        true_peak_max_dbtp: -1.3,
        lra_lu: 6.4,
        ..Default::default()
    }
}

/// A louder, more compressed reference master: hotter integrated loudness,
/// a true-peak just over 0 dBTP (so its cell lights BAD pink) and a tighter
/// LRA, giving every Δ cell something to report.
fn reference_meter() -> MeterSnapshot {
    MeterSnapshot {
        momentary_lufs: -9.1,
        short_term_lufs: -9.7,
        integrated_lufs: -9.4,
        true_peak_max_dbtp: 0.2,
        lra_lu: 4.1,
        ..Default::default()
    }
}

/// Open the Mixer view with the reference rail showing, a single loaded
/// reference selected active, and a fresh A/B meter snapshot folded in so
/// the comparative loudness readout renders live values.
fn app_with_metered_reference() -> Resonance {
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
    let _ = app.update(Message::Reference(ReferenceMessage::SetActive(ReferenceId(1))));

    // Fold in a comparative meter snapshot: mix vs reference.
    app.test_handle_engine_event(AudioEvent::ABMeterSnapshot {
        mix: mix_meter(),
        reference: Some(reference_meter()),
    });
    app
}

#[test]
fn reference_panel_loudness_readout() {
    let app = app_with_metered_reference();

    let mut ui = simulator(&app);
    ui.find("LOUDNESS")
        .expect("the comparative loudness readout is shown");
    for row in ["Integrated", "Short-term", "Momentary", "True-peak", "LRA"] {
        ui.find(row)
            .unwrap_or_else(|_| panic!("readout shows the {row} row"));
    }
    ui.find("MIX").expect("the readout shows the Mix column");
    ui.find("REF").expect("the readout shows the Ref column");

    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image("tests/snapshots/reference_panel_loudness_readout.png")
            .expect("matches_image i/o"),
        "snapshot diverged from golden"
    );
}
