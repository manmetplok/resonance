//! Golden-image snapshot for **group identity rails** on track headers
//! (epic #36, doc #200, todo #681).
//!
//! The coloured rail is a 3px strip on the left edge of group member track
//! headers that ties members to their parent group. Nested groups add a
//! second, inset rail.
//!
//! This snapshot locks in that:
//! 1. A single-group member shows one 3px identity rail
//! 2. A nested member shows two stacked rails ordered outermost->innermost left-to-right
//! 3. Rail order is deterministic (by nesting depth, then by group id)

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

/// Snapshot of a single-group member track header with one identity rail.
/// The track is a member of a "Drums" group, so it shows a single 3px rail
/// on the left edge matching the Drums identity colour.
#[test]
fn group_identity_rail_single_member() {
    let (mut app, _task) = Resonance::new();
    
    // Create a group
    let group_id = 1000;
    let group = TrackGroup::new(group_id, "Drums", GroupIdentityColor::Drum);
    
    // Add the group to the registry
    app.test_track_groups_mut().add_group(group);
    
    // Create a track
    let mut track = TrackState::new_instrument(1, 0);
    track.name = "Kick".to_string();
    track.id = 1001;
    track.soloed = false;
    track.muted = false;
    track.instrument_icon = InstrumentIcon::Drum;
    track.instrument_type = InstrumentType::Drum;
    
    // Add the track to the registry
    app.test_push_track(track);
    
    // Add the track as a member of the group
    app.test_track_groups_mut().add_member(group_id, 1001);
    
    snapshot_track_header(
        &app,
        "tests/snapshots/group_identity_rail_single_member.png",
    );
}

/// Snapshot of a nested group member track header with two identity rails.
/// The track is a member of a "Vocals" group (Vocal colour) which is nested
/// inside a "Drums" group (Drum colour). Rails should appear left-to-right as:
/// Drums (outermost, depth 0) then Vocals (nested, depth 1).
#[test]
fn group_identity_rail_nested_member() {
    let (mut app, _task) = Resonance::new();
    
    // Create parent group (Drums, depth 0)
    let parent_id = 1000;
    let parent = TrackGroup::new(parent_id, "Drums", GroupIdentityColor::Drum);
    
    // Create child group (Vocals, depth 1, nested under parent)
    let child_id = 1001;
    let mut child = TrackGroup::new(child_id, "Vocals", GroupIdentityColor::Vocal);
    child.nesting_parent = Some(parent_id);
    
    // Add parent group (must be added first since child references it)
    app.test_track_groups_mut().add_group(parent.clone());
    
    // Add child group as a member of parent
    app.test_track_groups_mut().add_member(parent_id, child_id);
    app.test_track_groups_mut().add_group(child.clone());
    
    // Create a track
    let mut track = TrackState::new_instrument(1, 0);
    track.name = "Lead Vocal".to_string();
    track.id = 1002;
    track.soloed = false;
    track.muted = false;
    track.instrument_icon = InstrumentIcon::Microphone;
    track.instrument_type = InstrumentType::Synth;
    
    // Add the track to the registry
    app.test_push_track(track);
    
    // Add the track as a member of the child group
    app.test_track_groups_mut().add_member(child_id, 1002);
    
    // The track should now show two rails: Drums (parent, depth 0) then Vocals (child, depth 1)
    snapshot_track_header(
        &app,
        "tests/snapshots/group_identity_rail_nested_member.png",
    );
}
