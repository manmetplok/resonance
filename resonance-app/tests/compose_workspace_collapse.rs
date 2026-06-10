//! Golden-image snapshots for the **Compose workspace group banners**
//! (SECTION / TRACKS) in their collapsed states.
//!
//! The whitespace pass turned both banners into click targets that fold
//! the lanes under them via `ComposeMessage::ToggleWorkspaceGroup` —
//! runtime UI state on `ComposeState` (`section_lanes_collapsed` /
//! `track_lanes_collapsed`), defaulting to open. Three states are
//! locked in:
//!
//! 1. **SECTION collapsed** — the banner stays visible (caret ▸) but
//!    the scale stripe, global lanes row, and chord lane are gone; the
//!    TRACKS banner and its lanes move up.
//! 2. **TRACKS collapsed** — every vocal / synth / drum lane is gone;
//!    the section lanes stay.
//! 3. **both collapsed** — just the two banner rows above the
//!    (now-empty) workspace.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::compose::{ComposeMessage, WorkspaceGroup};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

/// Window size matches the app's default & minimum window per the
/// design guidelines.
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

/// Build the demo app pinned to the Compose tab.
fn build_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Compose);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    app
}

fn toggle(app: &mut Resonance, group: WorkspaceGroup) {
    let _ = app.update(Message::Compose(ComposeMessage::ToggleWorkspaceGroup(
        group,
    )));
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

/// SECTION banner collapsed — scale stripe + chord lane hidden, banner
/// itself still visible with the ▸ caret.
#[test]
fn compose_section_banner_collapsed() {
    let mut app = build_app();
    toggle(&mut app, WorkspaceGroup::Section);
    snapshot_to(&app, "tests/snapshots/compose_section_banner_collapsed.png");
}

/// TRACKS banner collapsed — all vocal / synth / drum lanes hidden.
#[test]
fn compose_tracks_banner_collapsed() {
    let mut app = build_app();
    toggle(&mut app, WorkspaceGroup::Tracks);
    snapshot_to(&app, "tests/snapshots/compose_tracks_banner_collapsed.png");
}

/// Both banners collapsed — only the two banner rows remain in the
/// workspace column.
#[test]
fn compose_both_banners_collapsed() {
    let mut app = build_app();
    toggle(&mut app, WorkspaceGroup::Section);
    toggle(&mut app, WorkspaceGroup::Tracks);
    snapshot_to(&app, "tests/snapshots/compose_both_banners_collapsed.png");
}
