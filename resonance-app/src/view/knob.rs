/// Minimal pan knob widget.
///
/// iced has no built-in knob, so this module exposes a tiny
/// canvas-based one tuned for the mixer strips: bipolar range -1..=1,
/// center detent, vertical mouse drag to change value, shift for fine
/// adjust, and a double-click anywhere on the knob body to reset to
/// center.
use std::time::Instant;

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::Canvas;
use iced::{mouse, Color, Element, Length, Point, Rectangle, Renderer, Theme};

use crate::theme;

/// Full travel of the knob in pixels. Dragging the cursor this many
/// pixels vertically covers the entire -1..=1 range.
const DRAG_RANGE_PX: f32 = 140.0;
/// Maximum gap between two clicks (seconds) that still counts as a
/// double-click for the reset gesture.
const DOUBLE_CLICK_SECS: f32 = 0.35;
/// Side length of the knob in logical pixels.
pub const PAN_KNOB_SIZE: f32 = 28.0;
/// Sweep angle (radians). The indicator travels across this arc; the
/// remaining wedge at the bottom is the dead zone.
const SWEEP: f32 = std::f32::consts::PI * 1.5;

#[derive(Default)]
pub struct KnobState {
    /// Where the mouse was when the drag began, in bounds-relative pixels.
    drag_anchor_y: Option<f32>,
    /// Value at the moment the drag began — deltas are applied against this
    /// so repeated small moves don't accumulate rounding error.
    drag_anchor_value: f32,
    /// Timestamp of the most recent left-button press; used for the
    /// double-click reset gesture.
    last_click: Option<Instant>,
    /// Cached drawn geometry. Re-runs only when the value changes, so
    /// hover/redraw events that don't touch the knob get a free pass
    /// (Cache compares its stored bounds against the requested bounds).
    cache: canvas::Cache,
    /// The value last drawn into `cache`. When the live value drifts
    /// from this we clear the cache before drawing.
    cached_value: std::cell::Cell<f32>,
}

/// Construct a pan knob element. `value` is the current pan in -1..=1;
/// `on_change` is called with the new value every time the user drags
/// or double-clicks to reset.
pub fn pan_knob<'a, Message, F>(value: f32, on_change: F) -> Element<'a, Message>
where
    Message: 'a,
    F: 'a + Fn(f32) -> Message,
{
    Canvas::new(PanKnob {
        value: value.clamp(-1.0, 1.0),
        on_change: Box::new(on_change),
    })
    .width(Length::Fixed(PAN_KNOB_SIZE))
    .height(Length::Fixed(PAN_KNOB_SIZE))
    .into()
}

struct PanKnob<'a, Message> {
    value: f32,
    on_change: Box<dyn Fn(f32) -> Message + 'a>,
}

impl<'a, Message> canvas::Program<Message> for PanKnob<'a, Message> {
    type State = KnobState;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        // Clear the cache only when the externally-supplied value drifts
        // from what we last drew. The Cache itself handles bounds
        // changes; everything else (hover, scroll, sibling redraws)
        // returns the stored geometry without re-stroking the knob.
        if (state.cached_value.get() - self.value).abs() > f32::EPSILON {
            state.cache.clear();
            state.cached_value.set(self.value);
        }
        let value = self.value;
        let geometry = state.cache.draw(renderer, bounds.size(), |frame: &mut Frame| {
            let center = Point::new(bounds.width * 0.5, bounds.height * 0.5);
            let radius = (bounds.width.min(bounds.height) * 0.5) - 2.0;

            // Body fill + outer ring.
            frame.fill(
                &Path::circle(center, radius),
                Color::from_rgb(0.14, 0.14, 0.16),
            );
            frame.stroke(
                &Path::circle(center, radius),
                Stroke::default()
                    .with_width(1.0)
                    .with_color(Color::from_rgb(0.28, 0.28, 0.32)),
            );

            let start_angle = std::f32::consts::FRAC_PI_2 + SWEEP * 0.5;
            let value_angle = start_angle - (value * 0.5 + 0.5) * SWEEP;
            let center_angle = start_angle - 0.5 * SWEEP;
            let (arc_from, arc_to) = if value_angle <= center_angle {
                (value_angle, center_angle)
            } else {
                (center_angle, value_angle)
            };
            let arc_path = Path::new(|b| {
                b.arc(canvas::path::Arc {
                    center,
                    radius: radius - 2.0,
                    start_angle: iced::Radians(arc_from),
                    end_angle: iced::Radians(arc_to),
                });
            });
            frame.stroke(
                &arc_path,
                Stroke::default().with_width(2.0).with_color(theme::ACCENT),
            );

            for &t in &[0.0f32, 0.5, 1.0] {
                let a = start_angle - t * SWEEP;
                let inner = Point::new(
                    center.x + (radius - 4.0) * a.cos(),
                    center.y - (radius - 4.0) * a.sin(),
                );
                let outer =
                    Point::new(center.x + radius * a.cos(), center.y - radius * a.sin());
                frame.stroke(
                    &Path::line(inner, outer),
                    Stroke::default()
                        .with_width(1.0)
                        .with_color(Color::from_rgb(0.45, 0.45, 0.48)),
                );
            }

            let indicator_end = Point::new(
                center.x + (radius - 3.0) * value_angle.cos(),
                center.y - (radius - 3.0) * value_angle.sin(),
            );
            frame.stroke(
                &Path::line(center, indicator_end),
                Stroke::default().with_width(2.0).with_color(theme::TEXT),
            );
        });
        vec![geometry]
    }

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let pos = cursor.position_in(bounds)?;
                if !hit_circle(pos, bounds) {
                    return None;
                }
                // Double-click to reset to center.
                let now = Instant::now();
                let is_double = state
                    .last_click
                    .map(|prev| now.duration_since(prev).as_secs_f32() < DOUBLE_CLICK_SECS)
                    .unwrap_or(false);
                state.last_click = Some(now);
                if is_double {
                    state.drag_anchor_y = None;
                    return Some(canvas::Action::publish((self.on_change)(0.0)).and_capture());
                }
                state.drag_anchor_y = Some(pos.y);
                state.drag_anchor_value = self.value;
                Some(canvas::Action::capture())
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag_anchor_y = None;
                None
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let anchor_y = state.drag_anchor_y?;
                let pos = cursor.position_in(bounds)?;
                // Drag up = increase (pan right). Drag range is DRAG_RANGE_PX
                // for the full -1..=1 span.
                let dy = anchor_y - pos.y;
                let new = state.drag_anchor_value + dy * (2.0 / DRAG_RANGE_PX);
                let new = new.clamp(-1.0, 1.0);
                if (new - self.value).abs() < f32::EPSILON {
                    return Some(canvas::Action::capture());
                }
                Some(canvas::Action::publish((self.on_change)(new)).and_capture())
            }
            _ => None,
        }
    }
}

fn hit_circle(pos: Point, bounds: Rectangle) -> bool {
    let cx = bounds.width * 0.5;
    let cy = bounds.height * 0.5;
    let r = bounds.width.min(bounds.height) * 0.5;
    let dx = pos.x - cx;
    let dy = pos.y - cy;
    dx * dx + dy * dy <= r * r
}
