//! VIEW-21: the arrange vertical scroll must reach the bottom of the
//! content when automation lanes are expanded. The reducer used to cap
//! the offset at `tracks.len() * TRACK_HEIGHT`, which ignores automation,
//! take and group-header rows, so the last lanes were unreachable.

use resonance_app::message::{AutomationMessage, Message, ViewportMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, Resonance};
use resonance_common::{AutomationTarget, CurveKind};

const VIEWPORT_H: f32 = 400.0;

fn add_lane(app: &mut Resonance, target: AutomationTarget) {
    let _ = app.update(Message::Automation(AutomationMessage::AddBreakpoint {
        target,
        time_frames: 0,
        value: 0.5,
        curve: CurveKind::Linear,
    }));
}

fn content_height(app: &Resonance) -> f32 {
    app.test_arrange_header_offset() + app.test_arrange_row_layout().total_height()
}

#[test]
fn scroll_reaches_bottom_with_expanded_automation_lanes() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(VIEWPORT_H)));
    // The canvas reported its content height before any lane was expanded.
    let before = content_height(&app);
    let _ = app.update(Message::Viewport(ViewportMessage::TimelineContentSize(
        2000.0, before,
    )));

    let tracks: Vec<_> = app.test_registry().tracks.iter().map(|t| t.id).collect();
    for &t in &tracks {
        add_lane(&mut app, AutomationTarget::TrackGain(t));
        add_lane(&mut app, AutomationTarget::TrackPan(t));
        add_lane(&mut app, AutomationTarget::TrackMute(t));
        let _ = app.update(Message::Automation(AutomationMessage::ToggleTrackExpanded(t)));
    }
    let after = content_height(&app);
    assert!(after > before, "precondition: expanded lanes add rows");

    let _ = app.update(Message::Viewport(ViewportMessage::ScrollY(1.0e6)));
    assert_eq!(app.test_arrange_scroll_y(), after - VIEWPORT_H);

    let _ = app.update(Message::Viewport(ViewportMessage::ScrollToY(1.0e6)));
    assert_eq!(app.test_arrange_scroll_y(), after - VIEWPORT_H);
}

#[test]
fn scroll_clamps_to_zero_when_content_fits() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    demo::seed_demo_content(&mut app);
    let tall = content_height(&app) + 500.0;
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(tall)));
    let _ = app.update(Message::Viewport(ViewportMessage::ScrollY(250.0)));
    assert_eq!(app.test_arrange_scroll_y(), 0.0);
}
