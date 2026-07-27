//! Golden-image snapshots for the **track-group (folder-track) header
//! row** — the 60 px presentational band rendered by
//! `view::track_header::group_header::view_group_header` (epic #36,
//! doc #200, todo #680).
//!
//! The header is an organisational + macro-control strip. Anatomy
//! left→right: caret · identity colour swatch · bold group name ·
//! `N trk` count badge · group level trim (slider + dB) · macro `M` /
//! `S` buttons, over a faint group-colour wash.
//!
//! This component is VIEW-only at this stage (the reducers behind the
//! controls land in #686–#689 and the row isn't wired into the live
//! timeline column yet), so the two states are snapshotted standalone
//! via the `test_group_header_view` accessor rather than through the
//! full Arrange view:
//!
//! 1. **expanded** — an expanded Drums group at unity level, macros off.
//! 2. **collapsed_macros** — a collapsed Vocals group with both macros
//!    engaged and a non-unity (−6 dB) level trim.

mod common;

use iced::widget::container;
use iced::{Length, Size};
use iced_test::simulator::Simulator;
use resonance_app::{theme, Resonance};
use resonance_common::group_identity::GroupIdentityColor;
use resonance_common::track_group::TrackGroup;

/// Snapshot canvas — the track-header column width by the group-row
/// height, so the row renders at its real pitch with a little vertical
/// slack for the hairline.
const CANVAS: (f32, f32) = (theme::TRACK_HEADER_WIDTH, 80.0);

/// Build the iced simulator `Settings` with the same font registrations
/// the production app uses — without these the simulator falls back to a
/// default sans and the goldens stop matching the user's reality.
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

fn snapshot_to(app: &Resonance, group: &TrackGroup, member_count: usize, path: &str) {
    // Wrap the row in a fixed-width, BG_1-filled container so the row
    // lays out at the real track-header column width (it's `Length::Fill`
    // internally, like the live column).
    let element = container(app.test_group_header_view(group, member_count))
        .width(theme::TRACK_HEADER_WIDTH)
        .height(Length::Shrink)
        .style(theme::base_bg);

    let mut ui = Simulator::with_size(
        sim_settings(),
        Size::new(CANVAS.0, CANVAS.1),
        element,
    );
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// Expanded Drums group: unity level, macros off, 4 members.
#[test]
fn group_header_expanded() {
    let (app, _task) = Resonance::new();
    let group = TrackGroup::new(1, "Drums", GroupIdentityColor::Drum);
    snapshot_to(&app, &group, 4, "tests/snapshots/group_header_expanded.png");
}

/// Collapsed Vocals group: both macros engaged, −6 dB level trim,
/// 3 members.
#[test]
fn group_header_collapsed_macros() {
    let (app, _task) = Resonance::new();
    let mut group = TrackGroup::new(2, "Vocals", GroupIdentityColor::Vocal);
    group.is_collapsed = true;
    group.macro_mute = true;
    group.macro_solo = true;
    group.macro_level = 0.5;
    snapshot_to(
        &app,
        &group,
        3,
        "tests/snapshots/group_header_collapsed_macros.png",
    );
}
