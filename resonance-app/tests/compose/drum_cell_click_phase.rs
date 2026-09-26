//! A click on a drum-lane cell must flip the pattern cell the canvas
//! *draws* at that position (code review VIEW-03).
//!
//! The canvas used to send `(global_step + phase) % cycle` — already a
//! pattern index — and the `TogglePadStep` handler added the phase again,
//! so with a non-zero phase the edit landed `phase` steps to the right of
//! the click. This drives the real canvas `update` with a synthetic press
//! and feeds its message through the real reducer.

use iced::widget::canvas::Program;
use iced::{mouse, Point, Rectangle, Size};

use resonance_app::compose::messages::DrumGroupsMessage;
use resonance_app::compose::ComposeMessage;
use resonance_app::message::Message;
use resonance_app::state::{TrackState, ViewMode};
use resonance_app::view::compose::drumroll::{BarSpanView, ComposeDrumCanvas};
use resonance_app::view::compose::tracks::NAME_COLUMN_WIDTH;
use resonance_app::{demo, Resonance};

// Canvas geometry, mirrored from `view/compose/drumroll/canvas.rs`.
const PAD_LABEL_WIDTH: f32 = 76.0;
const STEP_HEADER_HEIGHT: f32 = 16.0;
const GROUP_HEAD_HEIGHT: f32 = 22.0;
const PAD_ROW_HEIGHT: f32 = 18.0;

fn drum_groups(app: &mut Resonance, msg: DrumGroupsMessage) {
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(msg)));
}

#[test]
fn clicking_the_first_cell_with_phase_4_flips_pattern_index_4() {
    let (mut app, _) = Resonance::new_for_test_on(ViewMode::Compose);
    demo::seed_demo_content(&mut app);
    let group_id = app.compose_state().drum_patterns[0].groups[0].id;
    drum_groups(&mut app, DrumGroupsMessage::SetGroupGrid { group_id, grid: 4 });
    drum_groups(&mut app, DrumGroupsMessage::SetGroupCycle { group_id, cycle: 16 });
    drum_groups(&mut app, DrumGroupsMessage::SetGroupPhase { group_id, phase: 4 });

    let groups = app.compose_state().drum_patterns[0].groups.clone();
    let group = &groups[0];
    assert_eq!((group.grid, group.cycle, group.phase), (4, 16, 4));
    let before = group.pads[0].pattern.clone();

    let track = TrackState::new_instrument(1, 0);
    let canvas = ComposeDrumCanvas {
        track: &track,
        groups: &groups,
        selected_group_id: None,
        track_selected: false,
        bar_spans: vec![BarSpanView {
            bar_start: 0,
            bar_end: 1,
            pattern_color: [0, 0, 0],
            pattern_groups: &groups,
            is_fill: false,
        }],
        section_bars: 1,
        visible_x: resonance_app::view::compose::visible_x_window(None),
    };

    let bounds = Rectangle::new(Point::ORIGIN, Size::new(1000.0, 400.0));
    let step_area_x = NAME_COLUMN_WIDTH + 8.0 + PAD_LABEL_WIDTH + 8.0;
    let first_pad_y = 2.0 + STEP_HEADER_HEIGHT + 4.0 + GROUP_HEAD_HEIGHT + PAD_ROW_HEIGHT / 2.0;
    // First cell of bar 1, first pad row.
    let cursor = mouse::Cursor::Available(Point::new(step_area_x + 1.0, first_pad_y));
    let press = iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
    let mut state = Default::default();
    let msg = canvas
        .update(&mut state, &press, bounds, cursor)
        .and_then(|a| a.into_inner().0)
        .expect("a cell press publishes a message");
    assert!(
        matches!(
            msg,
            Message::Compose(ComposeMessage::DrumGroups(DrumGroupsMessage::TogglePadStep {
                pad_index: 0,
                ..
            }))
        ),
        "expected a TogglePadStep on pad 0"
    );
    let _ = app.update(msg);

    let after = &app.compose_state().drum_patterns[0].groups[0].pads[0].pattern;
    let flipped: Vec<usize> = (0..before.len()).filter(|&i| before[i] != after[i]).collect();
    // The canvas draws pattern index `(0 + phase) % cycle` = 4 in the
    // clicked cell, so that is the one the click must flip.
    assert_eq!(flipped, vec![4], "the click must flip the cell it drew");
}
