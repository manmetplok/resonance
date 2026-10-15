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

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::reference::ReferenceMessage;
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance};
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
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
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

/// Switch loudness matching on and fold in the engine's echo: the -9.4
/// LUFS reference matched to a mix measured at `mix_lufs` (`None` = the
/// mix had no measurement, so nothing was matched).
fn match_loudness(app: &mut Resonance, mix_lufs: Option<f32>) {
    let _ = app.update(Message::Reference(ReferenceMessage::ToggleLoudnessMatch));
    app.test_handle_engine_event(AudioEvent::RefLoudnessMatchChanged {
        enabled: true,
        offset_db: mix_lufs.map(|m| m - -9.4).unwrap_or(0.0),
        mix_lufs,
    });
}

#[test]
fn loudness_match_shows_source_and_target_loudness() {
    let mut app = app_with_metered_reference();
    match_loudness(&mut app, Some(-16.1));

    let mut ui = simulator(&app);
    ui.find("-6.7 dB").expect("the applied offset is shown");
    ui.find("-9.4 \u{2192} -16.1 LUFS")
        .expect("the readout names the reference and the mix loudness it was matched to");
}

#[test]
fn loudness_match_without_a_mix_measurement_says_so() {
    let mut app = app_with_metered_reference();
    match_loudness(&mut app, None);

    let mut ui = simulator(&app);
    ui.find("+0.0 dB").expect("nothing is applied");
    ui.find("No mix loudness yet \u{2014} play, then re-toggle")
        .expect("the panel says why the offset is zero");
}

#[test]
fn reference_panel_loudness_readout() {
    let mut app = app_with_metered_reference();
    // Matching on, so the golden shows a non-zero offset and its readout.
    match_loudness(&mut app, Some(-16.1));

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
    common::assert_golden(&snap, "tests/snapshots/reference_panel_loudness_readout.png");
}
