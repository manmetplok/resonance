/// Expanded inline piano-roll editor for the Compose tab.
///
/// When the user double-clicks a track in the compact grid, it opens this
/// full-width editor which provides a comfortable piano-roll experience
/// scoped to the current section. Drawing primitives (keyboard column,
/// note rectangles, coordinate helpers, hit testing) are shared with
/// `view/midi_editor.rs` via `crate::view::piano_roll`; this canvas keeps the
/// section-specific bits inline: per-bar beat grid, scale row highlight,
/// toolbar, and the multi-clip note loop.
///
/// The canvas's concerns are split across files:
///
/// - this file: [`ExpandedEditorCanvas`] struct, the `view` entry function,
///   small coordinate helpers, and the [`canvas::Program`] impl that
///   orchestrates per-event dispatch and per-frame drawing.
/// - [`draw`]: pure-draw helpers ([`draw_note_rows`], [`draw_beat_grid`],
///   [`draw_notes`]).
/// - [`input`]: pointer interaction helpers ([`handle_grid_click`],
///   [`handle_right_click`], [`handle_drag`]).
use iced::widget::canvas::{self, Frame, Geometry};
use iced::widget::{container, Canvas};
use iced::{mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use resonance_audio::types::{TempoMap, TrackId, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::Scale;

use crate::compose::{ComposeMessage, SectionDefinitionState, SectionPlacementState};
use crate::message::*;
use crate::view::piano_roll::{
    self, note_name, PianoRollLayout, PianoRollViewport,
};
use crate::state::MidiClipState;
use crate::theme;
use crate::Resonance;

mod draw;
mod input;

/// Width of the piano keyboard column on the left.
const KEYBOARD_WIDTH: f32 = 52.0;
/// Default velocity for newly added notes.
pub(super) const DEFAULT_VELOCITY: f32 = 0.8;
/// Height reserved for the collapse button bar at the top of the expanded editor.
pub(super) const TOOLBAR_HEIGHT: f32 = 24.0;
/// Default snap resolution: quarter notes.
pub(super) const SNAP_TICKS: u64 = TICKS_PER_QUARTER_NOTE;

pub fn view<'a>(
    app: &'a Resonance,
    track_id: TrackId,
    placement: &'a SectionPlacementState,
    definition: &'a SectionDefinitionState,
) -> Element<'a, Message> {
    let section_start = app.tempo_map.bar_to_sample(placement.start_bar);
    let section_end = app.tempo_map.bar_to_sample(placement.start_bar + definition.length_bars);

    let canvas = Canvas::new(ExpandedEditorCanvas {
        track_id,
        midi_clips: &app.midi_clips,
        section_start,
        section_end,
        section_length_bars: definition.length_bars,
        sample_rate: app.sample_rate,
        tempo_map: &app.tempo_map,
        start_bar: placement.start_bar,
        scale: definition.scale,
        zoom_y: app.compose.expanded_zoom_y,
        scroll_x: app.compose.expanded_scroll_x,
        scroll_y: app.compose.expanded_scroll_y,
    })
    .width(Length::Fill)
    .height(Length::Fill);

    container(canvas)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

pub struct ExpandedEditorCanvas<'a> {
    pub track_id: TrackId,
    pub midi_clips: &'a [MidiClipState],
    pub section_start: u64,
    pub section_end: u64,
    pub section_length_bars: u32,
    pub sample_rate: u32,
    pub tempo_map: &'a TempoMap,
    pub start_bar: u32,
    pub scale: Option<Scale>,
    pub zoom_y: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
}

/// Local drag state for the expanded editor canvas.
#[derive(Debug, Clone)]
pub(super) enum DragMode {
    MoveNote {
        note_index: usize,
        start_tick_offset: i64,
        clip_id: u64,
    },
    ResizeNote {
        note_index: usize,
        anchor_tick: u64,
        clip_id: u64,
    },
}

#[derive(Debug, Default)]
pub struct ExpandedEditorState {
    pub(super) drag: Option<DragMode>,
    pub(super) previewing_note: Option<u8>,
}

impl<'a> canvas::Program<Message> for ExpandedEditorCanvas<'a> {
    type State = ExpandedEditorState;

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BG);

        if self.section_end <= self.section_start || bounds.width <= 0.0 {
            return vec![frame.into_geometry()];
        }

        let layout = self.layout(bounds);
        let viewport = self.viewport(&layout, bounds);
        let grid_x = layout.grid_x();
        let grid_w = bounds.width - grid_x;
        let grid_h = layout.grid_h;

        // -- Toolbar --
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(bounds.width, TOOLBAR_HEIGHT),
            theme::PANEL,
        );
        frame.fill_text(canvas::Text {
            content: "X  Close Editor".to_string(),
            position: Point::new(8.0, 4.0),
            color: theme::TEXT,
            size: 13.0.into(),
            ..canvas::Text::default()
        });
        frame.fill_text(canvas::Text {
            content: format!("Zoom: {:.0}px  (+/- to adjust)", self.zoom_y),
            position: Point::new(bounds.width - 180.0, 5.0),
            color: theme::TEXT_DIM,
            size: 11.0.into(),
            ..canvas::Text::default()
        });
        frame.fill_rectangle(
            Point::new(0.0, TOOLBAR_HEIGHT - 1.0),
            Size::new(bounds.width, 1.0),
            theme::SEPARATOR,
        );

        // Grid backdrop in BG_2 so the cards read against the BG_1 app
        // body. Black-key rows + out-of-scale rows render their own
        // fills over this; white-key rows just show the backdrop.
        frame.fill_rectangle(
            Point::new(grid_x, TOOLBAR_HEIGHT),
            Size::new(grid_w, grid_h),
            theme::BG_2,
        );

        // -- Note row backgrounds --
        self.draw_note_rows(&mut frame, &layout, &viewport, grid_w);

        // -- Beat grid lines --
        self.draw_beat_grid(&mut frame, &layout, &viewport, grid_w);

        // -- Notes --
        self.draw_notes(&mut frame, &layout, &viewport);

        // -- Piano keyboard --
        piano_roll::draw_keyboard(&mut frame, &layout, &viewport);

        // -- Separator between keyboard and grid --
        frame.fill_rectangle(
            Point::new(grid_x, TOOLBAR_HEIGHT),
            Size::new(1.0, grid_h),
            theme::SEPARATOR,
        );

        // -- Hover tooltip showing note name under cursor --
        if let Some(pos) = cursor.position_in(bounds) {
            if pos.y > TOOLBAR_HEIGHT && pos.x >= grid_x {
                let note = viewport.y_local_to_note(pos.y - TOOLBAR_HEIGHT);
                let name = note_name(note);
                // Draw in the keyboard area so it doesn't obscure the grid
                frame.fill_text(canvas::Text {
                    content: name,
                    position: Point::new(pos.x + 12.0, (pos.y - 14.0).max(TOOLBAR_HEIGHT + 2.0)),
                    color: Color::from_rgba(1.0, 1.0, 1.0, 0.75),
                    size: 11.0.into(),
                    ..canvas::Text::default()
                });
            }
        }

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let layout = self.layout(bounds);
        let viewport = self.viewport(&layout, bounds);
        let grid_x = layout.grid_x();
        let grid_h = layout.grid_h;

        match event {
            // -- Scroll --
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                cursor.position_in(bounds)?;
                let (dx, dy) = match delta {
                    mouse::ScrollDelta::Lines { x, y } => (-x * 30.0, -y * 30.0),
                    mouse::ScrollDelta::Pixels { x, y } => (-x, -y),
                };
                if dx.abs() > f32::EPSILON {
                    return Some(canvas::Action::publish(Message::Compose(ComposeMessage::ExpandedScrollX(dx))).and_capture());
                }
                return Some(canvas::Action::publish(Message::Compose(ComposeMessage::ExpandedScrollY(dy))).and_capture());
            }

            // -- Left click --
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    // Toolbar click: collapse
                    if pos.y < TOOLBAR_HEIGHT {
                        return Some(canvas::Action::publish(Message::Compose(ComposeMessage::CollapseTrack)).and_capture());
                    }

                    let gy = pos.y - TOOLBAR_HEIGHT;

                    // Piano keyboard: preview note
                    if pos.x < grid_x && gy < grid_h {
                        let note = viewport.y_local_to_note(gy);
                        state.previewing_note = Some(note);
                        return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::PreviewNote(
                                self.track_id,
                                note,
                            ))).and_capture());
                    }

                    // Grid area
                    if pos.x >= grid_x && gy < grid_h {
                        return self.handle_grid_click(state, &layout, &viewport, pos, gy);
                    }
                }
            }

            // -- Right-click: remove note --
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    if pos.y > TOOLBAR_HEIGHT && pos.x >= grid_x {
                        return self.handle_right_click(&layout, &viewport, pos);
                    }
                }
            }

            // -- Mouse move (drag) --
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    if let Some(msg) = self.handle_drag(state, &viewport, pos, grid_x) {
                        return Some(canvas::Action::publish(msg).and_capture());
                    }
                }
            }

            // -- Mouse release --
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag = None;
                if let Some(note) = state.previewing_note.take() {
                    return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::StopPreview(
                            self.track_id,
                            note,
                        ))).and_capture());
                }
            }

            // -- Keyboard shortcuts --
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Character(ref ch),
                ..
            }) if cursor.position_in(bounds).is_some() => {
                let s = ch.as_str();
                if s == "+" || s == "=" {
                    return Some(canvas::Action::publish(Message::Compose(ComposeMessage::ExpandedZoomY(2.0))).and_capture());
                }
                if s == "-" {
                    return Some(canvas::Action::publish(Message::Compose(ComposeMessage::ExpandedZoomY(-2.0))).and_capture());
                }
            }

            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                ..
            }) => {
                return Some(canvas::Action::publish(Message::Compose(ComposeMessage::CollapseTrack)).and_capture());
            }

            _ => {}
        }

        None
    }
}

// ---------------------------------------------------------------------------
// Coordinate helpers
// ---------------------------------------------------------------------------

impl<'a> ExpandedEditorCanvas<'a> {
    /// Section duration in ticks, summing per-bar numerators.
    pub(super) fn section_ticks(&self) -> u64 {
        (0..self.section_length_bars)
            .map(|b| {
                self.tempo_map.numerator_at_bar(self.start_bar + b) as u64
                    * TICKS_PER_QUARTER_NOTE
            })
            .sum()
    }

    pub(super) fn layout(&self, bounds: Rectangle) -> PianoRollLayout {
        PianoRollLayout {
            keyboard_w: KEYBOARD_WIDTH,
            grid_top: TOOLBAR_HEIGHT,
            grid_h: bounds.height - TOOLBAR_HEIGHT,
        }
    }

    pub(super) fn viewport(&self, layout: &PianoRollLayout, bounds: Rectangle) -> PianoRollViewport {
        let grid_w = bounds.width - layout.grid_x();
        PianoRollViewport {
            zoom_x: self.compute_zoom_x(grid_w),
            zoom_y: self.zoom_y,
            scroll_x: self.scroll_x,
            scroll_y: self.scroll_y,
        }
    }

    /// Compute pixels-per-tick so the full section fills `grid_w`.
    fn compute_zoom_x(&self, grid_w: f32) -> f32 {
        let ticks = self.section_ticks();
        if ticks == 0 {
            return 1.0;
        }
        grid_w / ticks as f32
    }

    /// Pixel rectangle for `note`, in canvas-local coordinates.
    pub(super) fn note_rect(
        &self,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
        note: &resonance_audio::types::MidiNote,
    ) -> Rectangle {
        let h = viewport.zoom_y;
        let note_h = (h - 1.0).max(2.0);
        Rectangle {
            x: layout.grid_x() + viewport.tick_to_x_local(note.start_tick),
            y: layout.grid_top + viewport.note_to_y_local(note.note),
            width: viewport.duration_to_w(note.duration_ticks),
            height: note_h,
        }
    }

    pub(super) fn midi_clip_end_sample(&self, clip: &MidiClipState) -> u64 {
        self.tempo_map
            .tick_to_abs_sample(clip.start_sample, clip.duration_ticks, self.sample_rate)
    }

    pub(super) fn clip_intersects_section(&self, clip: &MidiClipState) -> bool {
        let clip_end = self.midi_clip_end_sample(clip);
        !(clip_end <= self.section_start || clip.start_sample >= self.section_end)
    }
}
