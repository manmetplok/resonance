//! A right-click area that reports WHERE it was clicked, in window
//! coordinates — what a floating context menu needs to open under the
//! pointer.
//!
//! `mouse_area`'s `on_right_press` carries no position, and the cursor a
//! widget sees is in its scrollable's *content* space (shifted by the
//! scroll offset), so a strip in a scrolled mixer would place a menu off
//! to the side. The scroll shift is what iced hands each widget's
//! `overlay()` as `translation` (each scrollable on the way down
//! subtracts its offset), and the runtime asks for overlays before it
//! routes every event; this widget keeps the last one it was given and
//! adds it back to the cursor.

use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{tree, Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector};

/// Wrap `content` so a right press on it publishes `on_right(position)`,
/// `position` in window coordinates. Every other event passes through.
pub fn context_area<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    on_right: impl Fn(Point) -> Message + 'a,
) -> ContextArea<'a, Message> {
    ContextArea {
        content: content.into(),
        on_right: Box::new(on_right),
    }
}

pub struct ContextArea<'a, Message> {
    content: Element<'a, Message>,
    on_right: Box<dyn Fn(Point) -> Message + 'a>,
}

/// The scroll shift from this widget's space to the window's.
#[derive(Default)]
struct State {
    translation: Vector,
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for ContextArea<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        if shell.is_event_captured() {
            return;
        }
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) = event {
            if let Some(position) = cursor.position_over(layout.bounds()) {
                let translation = tree.state.downcast_ref::<State>().translation;
                shell.publish((self.on_right)(position + translation));
                shell.capture_event();
            }
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        tree.state.downcast_mut::<State>().translation = translation;
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: 'a> From<ContextArea<'a, Message>> for Element<'a, Message> {
    fn from(area: ContextArea<'a, Message>) -> Self {
        Element::new(area)
    }
}
