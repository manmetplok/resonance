//! Golden-image snapshot for the **bus inspector** — what the mixer's
//! right pane shows while a bus strip is selected.
//!
//! Two states are locked in:
//!
//! 1. **a plain bus** — CHAIN with the bus effects and the add picker,
//!    ROUTING (members / sends in / output), AUTOMATION and BUS.
//! 2. **a return bus with members** — the RETURN badge beside the name
//!    and a non-empty members list, which is the state the group
//!    actually exists to answer.

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, TrackMessage, UiMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance};
use resonance_audio::types::TrackOutput;

/// Window size matches the app's default & minimum window per the
/// design guidelines.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// The demo seed's first bus ("Bus 1 · Drums").
const DRUM_BUS: u64 = 100;

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

fn build_app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    app
}

fn select_bus(app: &mut Resonance, id: u64) {
    let _ = app.update(Message::Ui(UiMessage::SelectBus(Some(id))));
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// A plain bus with nothing routed into it: the members row reads
/// "0 · (none)" dimmed, and the chain shows the demo's compressor.
#[test]
fn mixer_inspector_bus_selected() {
    let mut app = build_app();
    select_bus(&mut app, DRUM_BUS);
    snapshot_to(&app, "tests/snapshots/mixer_inspector_bus.png");
}

/// The same bus with the drum tracks routed into it — the members list
/// is the question the bus strip itself cannot answer.
#[test]
fn mixer_inspector_bus_with_members() {
    let mut app = build_app();
    for track in [1_u64, 5] {
        let _ = app.update(Message::Track(TrackMessage::SetTrackOutput(
            track,
            TrackOutput::Bus(DRUM_BUS),
        )));
    }
    select_bus(&mut app, DRUM_BUS);
    snapshot_to(&app, "tests/snapshots/mixer_inspector_bus_members.png");
}
