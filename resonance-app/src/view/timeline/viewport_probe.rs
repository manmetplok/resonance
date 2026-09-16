//! A transparent widget wrapper that captures the *visible* part of the
//! timeline canvas.
//!
//! The arrange canvas is sized to the whole song and horizontally panned
//! by the outer `Scrollable`, and the `canvas::Program` API (`update` /
//! `draw`) only ever sees the full canvas `bounds` — never the viewport
//! rectangle iced threads through `Widget::update` / `Widget::draw`. This
//! wrapper delegates every `Widget` method to the wrapped canvas
//! unchanged (including `tag` / `state` / `children` / `diff`, so the
//! widget tree and the canvas's `TimelineState` are exactly what they
//! would be without it) and, on `update` and `draw`, stores the viewport
//! intersected with the canvas bounds — shifted into canvas-local
//! coordinates — in a shared cell the [`TimelineCanvas`] program reads.
//!
//! The write in `draw` happens immediately before the inner canvas's
//! `Program::draw` runs, so the cull window derived from the probe is
//! frame-exact: the cached pass can never present geometry culled for a
//! viewport other than the one being rendered.
//!
//! [`TimelineCanvas`]: super::TimelineCanvas

use std::cell::Cell;
use std::rc::Rc;

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{overlay, renderer, Clipboard, Shell};
use iced::{mouse, Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector};

use crate::message::Message;

pub(crate) struct ViewportProbe<'a> {
    content: Element<'a, Message, Theme, Renderer>,
    probe: Rc<Cell<Option<Rectangle>>>,
}

impl<'a> ViewportProbe<'a> {
    pub(crate) fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        probe: Rc<Cell<Option<Rectangle>>>,
    ) -> Self {
        Self {
            content: content.into(),
            probe,
        }
    }

    /// Store the visible rectangle in canvas-local coordinates. An empty
    /// intersection (the canvas is fully off screen) keeps the previous
    /// value rather than flapping back to "draw everything".
    fn record(&self, layout: Layout<'_>, viewport: &Rectangle) {
        let bounds = layout.bounds();
        if let Some(visible) = viewport.intersection(&bounds) {
            self.probe.set(Some(Rectangle {
                x: visible.x - bounds.x,
                y: visible.y - bounds.y,
                ..visible
            }));
        }
    }
}

impl Widget<Message, Theme, Renderer> for ViewportProbe<'_> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.record(layout, viewport);
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn tag(&self) -> widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> widget::tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.content.as_widget().diff(tree);
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.record(layout, viewport);
        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

impl<'a> From<ViewportProbe<'a>> for Element<'a, Message, Theme, Renderer> {
    fn from(probe: ViewportProbe<'a>) -> Self {
        Element::new(probe)
    }
}
