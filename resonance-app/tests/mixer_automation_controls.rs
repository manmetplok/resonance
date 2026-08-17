//! Render-level checks for the mixer's per-channel automation controls
//! (todo #383, arch doc #162 §3). These drive the real `view()` tree
//! through the iced simulator and assert the lane header (parameter
//! picker + Read toggle) actually renders — a deterministic,
//! GPU-independent companion to the golden snapshots (which diverge in
//! this environment).

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_audio::types::AudioEvent;
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};

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

fn build_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Mixer);
    let (mut app, _task) = Resonance::new_for_test();
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    app
}

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view())
}

/// With no lanes, no strip shows a Read toggle. Once a lane exists for a
/// channel, its strip surfaces the lane header's READ toggle — proving
/// the header (parameter picker + Read toggle) is gated on lane presence:
/// a lane can be "pointed at" a target and the control then appears.
#[test]
fn read_toggle_appears_only_once_a_lane_exists() {
    let mut app = build_app();

    // No automation yet → no Read toggle anywhere.
    {
        let mut ui = simulator(&app);
        assert!(
            ui.find("READ").is_err(),
            "no Read toggle should render before any lane is added"
        );
    }

    // Point the master's lane at its gain — the master strip is always
    // rendered, so this is a stable target.
    let lane = AutomationLane::new(
        1,
        AutomationTarget::MasterGain,
        vec![Breakpoint::new(0, 0.5, CurveKind::Linear)],
    );
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged { lane });

    let mut ui = simulator(&app);
    ui.find("READ")
        .expect("the lane header's Read toggle should render once a lane exists");
}
