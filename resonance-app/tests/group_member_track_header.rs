//! Golden-image snapshot for a **group member track header** showing the
//! "via group" solo chip (epic #36, doc #200, todo #688).
//!
//! The "via group" chip is a small amber/outlined pill reading "S·grp" that
//! appears on a group member track when:
//! - the containing group has `macro_solo = true`
//! - the member's own `soloed` flag is `false`
//!
//! This snapshot locks in that:
//! 1. the "S·grp" WARM/amber pill renders correctly
//! 2. the M/S/monitor button row stays in place (chip sits left of the
//!    Fill spacer, layout unchanged vs the chip-absent case)

use iced::widget::container;
use iced::{Length, Size};
use iced_test::simulator::Simulator;
use resonance_app::{theme, Resonance};
use resonance_common::group_identity::GroupIdentityColor;
use resonance_common::track_group::TrackGroup;

use resonance_app::state::{InstrumentIcon, InstrumentType, TrackState};

/// Snapshot canvas — the track-header column width by a single track row
/// height, so the header renders at its real pitch.
const CANVAS: (f32, f32) = (theme::TRACK_HEADER_WIDTH, theme::TRACK_HEIGHT + 2.0);

/// Build the iced simulator `Settings` with the same font registrations
/// the production app uses.
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

fn snapshot_track_header(app: &Resonance, path: &str) {
    // Get the first (and only) track we added
    let track = app.test_registry().tracks.first().expect("test track must exist");
    
    // Wrap the header in a fixed-width container at the track-header column width
    let element = container(app.test_track_header_view(track))
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
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
}

/// Snapshot of a group member track header with the "via group" solo chip.
/// The track's own solo is OFF, but its containing group has macro_solo ON,
/// so the small amber "S·grp" pill appears to the left of the button row.
#[test]
fn group_member_track_header_via_group_solo() {
    let (mut app, _task) = Resonance::new();
    
    // Create a group with macro_solo enabled
    let group_id = 1000;
    let mut group = TrackGroup::new(group_id, "Drums", GroupIdentityColor::Drum);
    group.macro_solo = true;
    
    // Add the group to the registry using the mutable accessor
    app.test_track_groups_mut().add_group(group);
    
    // Create a track that is NOT soloed on its own
    let mut track = TrackState::new_instrument(1, 0);
    track.name = "Kick".to_string();
    track.id = 1001;
    track.soloed = false; // Own solo is OFF
    track.muted = false;
    track.instrument_icon = InstrumentIcon::Drum;
    track.instrument_type = InstrumentType::Drum;
    
    // Add the track to the registry
    app.test_push_track(track);
    
    // Add the track as a member of the group
    app.test_track_groups_mut().add_member(group_id, 1001);
    
    // Now the track should show the "via group" solo chip because:
    // - group.macro_solo = true
    // - track.soloed = false
    // - track is a member of the group
    snapshot_track_header(
        &app,
        "tests/snapshots/group_member_track_header_via_group_solo.png",
    );
}

/// Snapshot of a group member track header with the "via group" mute chip
/// (epic #36, doc #200, todo #687). The track's own mute is OFF, but its
/// containing group has `macro_mute` ON, so the small pink "M·grp" pill
/// appears to the left of the button row (left of the solo chip's slot).
#[test]
fn group_member_track_header_via_group_mute() {
    let (mut app, _task) = Resonance::new();

    // Create a group with macro_mute enabled.
    let group_id = 1000;
    let mut group = TrackGroup::new(group_id, "Drums", GroupIdentityColor::Drum);
    group.macro_mute = true;

    app.test_track_groups_mut().add_group(group);

    // Create a track that is NOT muted on its own.
    let mut track = TrackState::new_instrument(1, 0);
    track.name = "Kick".to_string();
    track.id = 1001;
    track.soloed = false;
    track.muted = false; // Own mute is OFF
    track.instrument_icon = InstrumentIcon::Drum;
    track.instrument_type = InstrumentType::Drum;

    app.test_push_track(track);
    app.test_track_groups_mut().add_member(group_id, 1001);

    // Now the track should show the "via group" mute chip because:
    // - group.macro_mute = true
    // - track.muted = false
    // - track is a member of the group
    snapshot_track_header(
        &app,
        "tests/snapshots/group_member_track_header_via_group_mute.png",
    );
}
