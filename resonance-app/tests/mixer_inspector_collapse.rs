//! Golden-image snapshots for the **mixer inspector's collapsible
//! groups** (SIGNAL / ROUTING / CHAIN).
//!
//! The whitespace pass made the three inspector groups collapsible via
//! `UiMessage::ToggleMixerInspectorGroup` — runtime UI state held in
//! `MixerUiState::collapsed_inspector_groups`, defaulting to all-open.
//! Three states are locked in:
//!
//! 1. **all open** — the post-redesign baseline with a track selected.
//! 2. **ROUTING + CHAIN collapsed** — only their header rows (caret
//!    flipped to ▸) remain; the SIGNAL tiles stay visible.
//! 3. **SIGNAL collapsed** — the live PEAK/RMS/PAN/OUT tiles fold away
//!    while ROUTING + CHAIN stay open.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::state::{MixerInspectorGroup, ViewMode};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

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
/// track (Synth Bass), so the inspector renders the full SIGNAL /
/// ROUTING / CHAIN stack rather than the empty placeholder.
fn build_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Mixer);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    // Belt-and-braces in case another test in this binary set
    // STARTUP_TAB to something else first (OnceLock makes our set a
    // no-op then).
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
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
}

/// Baseline: every inspector group open (the default).
#[test]
fn mixer_inspector_all_groups_open() {
    let app = build_app();
    snapshot_to(&app, "tests/snapshots/mixer_inspector_all_open.png");
}

/// ROUTING and CHAIN folded — only their header rows remain, SIGNAL
/// tiles still visible above.
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

/// SIGNAL folded — the PEAK/RMS/PAN/OUT tiles disappear while ROUTING
/// and CHAIN stay open underneath.
#[test]
fn mixer_inspector_signal_collapsed() {
    let mut app = build_app();
    toggle(&mut app, MixerInspectorGroup::Signal);
    snapshot_to(&app, "tests/snapshots/mixer_inspector_signal_collapsed.png");
}
