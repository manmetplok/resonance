//! VIEW-23: a pan-knob drag keeps tracking after the cursor leaves the
//! 28 px knob. The drag math spans 140 px for the full -1..=1 range, but
//! `CursorMoved` used `cursor.position_in(bounds)`, which is `None`
//! outside the widget, so a drag stopped at about ±0.2.

use iced::widget::canvas::Program;
use iced::{mouse, Point, Rectangle, Size};
use resonance_app::view::knob::{pan_knob_program, KnobState, PAN_KNOB_SIZE};

/// The knob sits at (100, 300) in the window.
fn bounds() -> Rectangle {
    Rectangle::new(Point::new(100.0, 300.0), Size::new(PAN_KNOB_SIZE, PAN_KNOB_SIZE))
}

fn at(x: f32, y: f32) -> mouse::Cursor {
    mouse::Cursor::Available(Point::new(x, y))
}

fn published(
    knob: &impl Program<f32, State = KnobState>,
    state: &mut KnobState,
    event: iced::Event,
    cursor: mouse::Cursor,
) -> Option<f32> {
    knob.update(state, &event, bounds(), cursor)
        .and_then(|a| a.into_inner().0)
}

#[test]
fn drag_continues_outside_the_knob_bounds() {
    let knob = pan_knob_program(0.0, |v| v);
    let mut state = KnobState::default();
    let centre = (100.0 + PAN_KNOB_SIZE / 2.0, 300.0 + PAN_KNOB_SIZE / 2.0);

    let press = iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
    assert_eq!(published(&knob, &mut state, press, at(centre.0, centre.1)), None);

    // Drag 100 px straight up — far above the 28 px knob. 70 px is the
    // full half-range, so this pins the value at +1.0.
    let moved = iced::Event::Mouse(mouse::Event::CursorMoved {
        position: Point::new(centre.0, centre.1 - 100.0),
    });
    assert_eq!(
        published(&knob, &mut state, moved, at(centre.0, centre.1 - 100.0)),
        Some(1.0)
    );

    // And down past the bottom edge, well to the side.
    let moved = iced::Event::Mouse(mouse::Event::CursorMoved {
        position: Point::new(centre.0 + 60.0, centre.1 + 35.0),
    });
    assert_eq!(
        published(&knob, &mut state, moved, at(centre.0 + 60.0, centre.1 + 35.0)),
        Some(-0.5)
    );
}

#[test]
fn hover_outside_without_a_press_publishes_nothing() {
    let knob = pan_knob_program(0.0, |v| v);
    let mut state = KnobState::default();
    let moved = iced::Event::Mouse(mouse::Event::CursorMoved {
        position: Point::new(10.0, 10.0),
    });
    assert_eq!(published(&knob, &mut state, moved, at(10.0, 10.0)), None);
}
