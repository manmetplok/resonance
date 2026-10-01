//! Golden-image snapshots for the **mixer inspector's collapsible
//! groups** (CHAIN / SENDS / ROUTING / AUTOMATION / TRACK,
//! mixer-cleanup.md §3.1).
//!
//! The groups are collapsible via `UiMessage::ToggleMixerInspectorGroup`
//! — runtime UI state held in `MixerUiState::collapsed_inspector_groups`,
//! defaulting to all-open. Three states are locked in:
//!
//! 1. **all open** — the baseline with a track selected.
//! 2. **ROUTING + CHAIN collapsed** — only their header rows (caret
//!    flipped to ▸) remain; the other groups stay open.
//! 3. **SENDS + AUTOMATION + TRACK collapsed** — the lower groups fold
//!    while CHAIN and ROUTING stay open.

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::state::{MixerInspectorGroup, ViewMode};
use resonance_app::{demo, theme, Resonance};

/// Window size matches the app's default & minimum window per the
/// design guidelines.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// Build the iced simulator `Settings` with the same font registrations
/// the production app uses — without these the simulator falls back to
/// a default sans and goldens stop matching the user's reality.
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

/// Build the demo app on the Mixer tab. `seed_demo_content` selects a
/// track (Synth Bass), so the inspector renders the full group stack
/// rather than the empty placeholder.
fn build_app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    demo::seed_demo_content(&mut app);
    // Reach the Mixer through the real reducer, so the view-switch path
    // is exercised rather than assumed.
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    app
}

fn toggle(app: &mut Resonance, group: MixerInspectorGroup) {
    let _ = app.update(Message::Ui(UiMessage::ToggleMixerInspectorGroup(group)));
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(
        sim_settings(),
        Size::new(WINDOW.0, WINDOW.1),
        app.view(),
    );
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// Baseline: every inspector group open (the default).
#[test]
fn mixer_inspector_all_groups_open() {
    let app = build_app();
    snapshot_to(&app, "tests/snapshots/mixer_inspector_all_open.png");
}

/// ROUTING and CHAIN folded — only their header rows remain, the other
/// groups still open around them.
#[test]
fn mixer_inspector_routing_and_chain_collapsed() {
    let mut app = build_app();
    toggle(&mut app, MixerInspectorGroup::Routing);
    toggle(&mut app, MixerInspectorGroup::Chain);
    snapshot_to(
        &app,
        "tests/snapshots/mixer_inspector_routing_chain_collapsed.png",
    );
}

/// SENDS, AUTOMATION and TRACK folded — CHAIN and ROUTING stay open
/// above three header rows.
#[test]
fn mixer_inspector_lower_groups_collapsed() {
    let mut app = build_app();
    toggle(&mut app, MixerInspectorGroup::Sends);
    toggle(&mut app, MixerInspectorGroup::Automation);
    toggle(&mut app, MixerInspectorGroup::Track);
    snapshot_to(
        &app,
        "tests/snapshots/mixer_inspector_lower_groups_collapsed.png",
    );
}
