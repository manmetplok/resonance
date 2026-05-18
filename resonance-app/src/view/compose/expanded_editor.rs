/// Expanded inline piano-roll editor for the Compose tab.
///
/// When the user double-clicks a track in the compact grid, it opens this
/// full-width editor which provides a comfortable piano-roll experience
/// scoped to the current section. Drawing primitives (keyboard column,
/// note rectangles, coordinate helpers, hit testing) are shared with
/// `midi_editor.rs` via `crate::piano_roll`; this canvas keeps the
/// section-specific bits inline: per-bar beat grid, scale row highlight,
/// toolbar, and the multi-clip note loop.
use iced::widget::canvas::{self, Frame, Geometry};
use iced::widget::{container, Canvas};
use iced::{mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use resonance_audio::types::{TempoMap, TrackId, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::Scale;

use crate::compose::{ComposeMessage, SectionDefinitionState, SectionPlacementState};
use crate::message::*;
use crate::piano_roll::{
    self, hit_test_note, is_black_key, note_name, snap_tick, NoteEdge, NoteStyle, PianoRollLayout,
    PianoRollViewport, NOTE_COUNT,
};
use crate::state::MidiClipState;
use crate::theme;
use crate::Resonance;

/// Width of the piano keyboard column on the left.
const KEYBOARD_WIDTH: f32 = 52.0;
/// Default velocity for newly added notes.
const DEFAULT_VELOCITY: f32 = 0.8;
/// Height reserved for the collapse button bar at the top of the expanded editor.
const TOOLBAR_HEIGHT: f32 = 24.0;
/// Default snap resolution: quarter notes.
const SNAP_TICKS: u64 = TICKS_PER_QUARTER_NOTE;

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
enum DragMode {
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
    drag: Option<DragMode>,
    previewing_note: Option<u8>,
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
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        let layout = self.layout(bounds);
        let viewport = self.viewport(&layout, bounds);
        let grid_x = layout.grid_x();
        let grid_h = layout.grid_h;

        match event {
            // -- Scroll --
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if cursor.position_in(bounds).is_none() {
                    return (canvas::event::Status::Ignored, None);
                }
                let (dx, dy) = match delta {
                    mouse::ScrollDelta::Lines { x, y } => (-x * 30.0, -y * 30.0),
                    mouse::ScrollDelta::Pixels { x, y } => (-x, -y),
                };
                if dx.abs() > f32::EPSILON {
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::Compose(ComposeMessage::ExpandedScrollX(dx))),
                    );
                }
                return (
                    canvas::event::Status::Captured,
                    Some(Message::Compose(ComposeMessage::ExpandedScrollY(dy))),
                );
            }

            // -- Left click --
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    // Toolbar click: collapse
                    if pos.y < TOOLBAR_HEIGHT {
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::Compose(ComposeMessage::CollapseTrack)),
                        );
                    }

                    let gy = pos.y - TOOLBAR_HEIGHT;

                    // Piano keyboard: preview note
                    if pos.x < grid_x && gy < grid_h {
                        let note = viewport.y_local_to_note(gy);
                        state.previewing_note = Some(note);
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::MidiEditor(MidiEditorMessage::PreviewNote(
                                self.track_id,
                                note,
                            ))),
                        );
                    }

                    // Grid area
                    if pos.x >= grid_x && gy < grid_h {
                        return self.handle_grid_click(state, &layout, &viewport, pos, gy);
                    }
                }
            }

            // -- Right-click: remove note --
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    if pos.y > TOOLBAR_HEIGHT && pos.x >= grid_x {
                        return self.handle_right_click(&layout, &viewport, pos);
                    }
                }
            }

            // -- Mouse move (drag) --
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    if let Some(msg) = self.handle_drag(state, &viewport, pos, grid_x) {
                        return (canvas::event::Status::Captured, Some(msg));
                    }
                }
            }

            // -- Mouse release --
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag = None;
                if let Some(note) = state.previewing_note.take() {
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::MidiEditor(MidiEditorMessage::StopPreview(
                            self.track_id,
                            note,
                        ))),
                    );
                }
            }

            // -- Keyboard shortcuts --
            canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Character(ref ch),
                ..
            }) if cursor.position_in(bounds).is_some() => {
                let s = ch.as_str();
                if s == "+" || s == "=" {
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::Compose(ComposeMessage::ExpandedZoomY(2.0))),
                    );
                }
                if s == "-" {
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::Compose(ComposeMessage::ExpandedZoomY(-2.0))),
                    );
                }
            }

            canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                ..
            }) => {
                return (
                    canvas::event::Status::Captured,
                    Some(Message::Compose(ComposeMessage::CollapseTrack)),
                );
            }

            _ => {}
        }

        (canvas::event::Status::Ignored, None)
    }
}

// ---------------------------------------------------------------------------
// Coordinate helpers
// ---------------------------------------------------------------------------

impl<'a> ExpandedEditorCanvas<'a> {
    /// Section duration in ticks, summing per-bar numerators.
    fn section_ticks(&self) -> u64 {
        (0..self.section_length_bars)
            .map(|b| {
                self.tempo_map.numerator_at_bar(self.start_bar + b) as u64
                    * TICKS_PER_QUARTER_NOTE
            })
            .sum()
    }

    fn layout(&self, bounds: Rectangle) -> PianoRollLayout {
        PianoRollLayout {
            keyboard_w: KEYBOARD_WIDTH,
            grid_top: TOOLBAR_HEIGHT,
            grid_h: bounds.height - TOOLBAR_HEIGHT,
        }
    }

    fn viewport(&self, layout: &PianoRollLayout, bounds: Rectangle) -> PianoRollViewport {
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
    fn note_rect(
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

    fn midi_clip_end_sample(&self, clip: &MidiClipState) -> u64 {
        self.tempo_map
            .tick_to_abs_sample(clip.start_sample, clip.duration_ticks, self.sample_rate)
    }

    fn clip_intersects_section(&self, clip: &MidiClipState) -> bool {
        let clip_end = self.midi_clip_end_sample(clip);
        !(clip_end <= self.section_start || clip.start_sample >= self.section_end)
    }
}

// ---------------------------------------------------------------------------
// Interaction helpers
// ---------------------------------------------------------------------------

impl<'a> ExpandedEditorCanvas<'a> {
    fn handle_grid_click(
        &self,
        state: &mut ExpandedEditorState,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
        pos: Point,
        gy: f32,
    ) -> (canvas::event::Status, Option<Message>) {
        let grid_x = layout.grid_x();
        let click_note = viewport.y_local_to_note(gy);

        // Check existing notes for move / resize
        for clip in self
            .midi_clips
            .iter()
            .filter(|c| c.track_id == self.track_id)
        {
            if !self.clip_intersects_section(clip) {
                continue;
            }
            for (i, n) in clip.notes.iter().enumerate() {
                let rect = self.note_rect(layout, viewport, n);
                if let Some(edge) = hit_test_note(rect, pos) {
                    state.drag = Some(match edge {
                        NoteEdge::ResizeRight => DragMode::ResizeNote {
                            note_index: i,
                            anchor_tick: n.start_tick,
                            clip_id: clip.id,
                        },
                        NoteEdge::Body => {
                            let click_tick = viewport.x_local_to_tick(pos.x - grid_x);
                            let tick_offset = n.start_tick as i64 - click_tick as i64;
                            DragMode::MoveNote {
                                note_index: i,
                                start_tick_offset: tick_offset,
                                clip_id: clip.id,
                            }
                        }
                    });
                    return (canvas::event::Status::Captured, None);
                }
            }
        }

        // Empty space: add note
        let click_tick = viewport.x_local_to_tick(pos.x - grid_x);
        let snapped = snap_tick(click_tick, SNAP_TICKS);

        for clip in self
            .midi_clips
            .iter()
            .filter(|c| c.track_id == self.track_id)
        {
            let clip_end = self.midi_clip_end_sample(clip);
            if self.section_start >= clip.start_sample && self.section_start < clip_end {
                return (
                    canvas::event::Status::Captured,
                    Some(Message::MidiEditor(MidiEditorMessage::AddNote {
                        clip_id: clip.id,
                        note: click_note,
                        start_tick: snapped,
                        duration_ticks: SNAP_TICKS,
                        velocity: DEFAULT_VELOCITY,
                    })),
                );
            }
        }

        (canvas::event::Status::Captured, None)
    }

    fn handle_right_click(
        &self,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
        pos: Point,
    ) -> (canvas::event::Status, Option<Message>) {
        for clip in self
            .midi_clips
            .iter()
            .filter(|c| c.track_id == self.track_id)
        {
            if !self.clip_intersects_section(clip) {
                continue;
            }
            for (i, n) in clip.notes.iter().enumerate() {
                let rect = self.note_rect(layout, viewport, n);
                if hit_test_note(rect, pos).is_some() {
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::MidiEditor(MidiEditorMessage::RemoveNote {
                            clip_id: clip.id,
                            note_index: i,
                        })),
                    );
                }
            }
        }
        (canvas::event::Status::Ignored, None)
    }

    fn handle_drag(
        &self,
        state: &mut ExpandedEditorState,
        viewport: &PianoRollViewport,
        pos: Point,
        grid_x: f32,
    ) -> Option<Message> {
        match &state.drag {
            Some(DragMode::MoveNote {
                note_index,
                start_tick_offset,
                clip_id,
                ..
            }) => {
                if pos.x >= grid_x && pos.y > TOOLBAR_HEIGHT {
                    let gy = pos.y - TOOLBAR_HEIGHT;
                    let tick = viewport.x_local_to_tick(pos.x - grid_x);
                    let raw = (tick as i64 + start_tick_offset).max(0) as u64;
                    let snapped = snap_tick(raw, SNAP_TICKS);
                    let note = viewport.y_local_to_note(gy);
                    return Some(Message::MidiEditor(MidiEditorMessage::MoveNote {
                        clip_id: *clip_id,
                        note_index: *note_index,
                        new_start_tick: snapped,
                        new_note: note,
                    }));
                }
                None
            }
            Some(DragMode::ResizeNote {
                note_index,
                anchor_tick,
                clip_id,
            }) => {
                if pos.x >= grid_x {
                    let tick = viewport.x_local_to_tick(pos.x - grid_x);
                    let snapped = snap_tick(tick, SNAP_TICKS);
                    let new_dur = snapped.saturating_sub(*anchor_tick).max(SNAP_TICKS);
                    return Some(Message::MidiEditor(MidiEditorMessage::ResizeNote {
                        clip_id: *clip_id,
                        note_index: *note_index,
                        new_duration_ticks: new_dur,
                    }));
                }
                None
            }
            None => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing helpers
// ---------------------------------------------------------------------------

impl<'a> ExpandedEditorCanvas<'a> {
    fn draw_note_rows(
        &self,
        frame: &mut Frame,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
        grid_w: f32,
    ) {
        let grid_x = layout.grid_x();
        for midi_note in 0..NOTE_COUNT {
            let y = layout.grid_top + viewport.note_to_y_local(midi_note);
            let h = viewport.zoom_y;

            if y + h < layout.grid_top || y > layout.grid_top + layout.grid_h {
                continue;
            }

            let is_black = is_black_key(midi_note);
            let in_scale = self.scale.map(|s| s.contains(midi_note)).unwrap_or(true);

            // Black-key rows render darker against the BG_2 backdrop;
            // out-of-scale rows get a subtle warm tint. White-key rows
            // skip the fill entirely so the BG_2 backdrop shows through.
            let color = if !in_scale {
                Some(Color {
                    a: 0.06,
                    ..theme::WARM
                })
            } else if is_black {
                Some(theme::BG_1)
            } else {
                None
            };
            if let Some(color) = color {
                frame.fill_rectangle(Point::new(grid_x, y), Size::new(grid_w, h), color);
            }

            if midi_note % 12 == 0 {
                frame.fill_rectangle(
                    Point::new(grid_x, y + h - 1.0),
                    Size::new(grid_w, 1.0),
                    theme::LINE_2,
                );
            }
        }
    }

    fn draw_beat_grid(
        &self,
        frame: &mut Frame,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
        grid_w: f32,
    ) {
        if self.section_length_bars == 0 {
            return;
        }
        let grid_x = layout.grid_x();
        let grid_h = layout.grid_h;
        let tpb = TICKS_PER_QUARTER_NOTE;
        let total_ticks = self.section_ticks();

        // Walk bars for correct placement with varying time signatures.
        let mut tick_pos: u64 = 0;
        for bar_offset in 0..self.section_length_bars {
            let bar = self.start_bar + bar_offset;
            let num = self.tempo_map.numerator_at_bar(bar) as u64;
            let bar_ticks = num * tpb;

            // Bar line — LINE, 1px hairline like the rest of the redesign.
            let x = grid_x + viewport.tick_to_x_local(tick_pos);
            if x >= grid_x && x <= grid_x + grid_w {
                frame.fill_rectangle(
                    Point::new(x, TOOLBAR_HEIGHT),
                    Size::new(1.0, grid_h),
                    theme::LINE,
                );
                if tick_pos < total_ticks {
                    frame.fill_text(canvas::Text {
                        content: format!("{}", bar_offset + 1),
                        position: Point::new(x + 3.0, TOOLBAR_HEIGHT + 2.0),
                        color: theme::TEXT_3,
                        size: 9.0.into(),
                        font: theme::MONO_FONT,
                        ..canvas::Text::default()
                    });
                }
            }

            // Beat lines — LINE_2 hairlines.
            for beat in 1..num {
                let beat_tick = tick_pos + beat * tpb;
                let bx = grid_x + viewport.tick_to_x_local(beat_tick);
                if bx >= grid_x && bx <= grid_x + grid_w {
                    frame.fill_rectangle(
                        Point::new(bx, TOOLBAR_HEIGHT),
                        Size::new(1.0, grid_h),
                        theme::LINE_2,
                    );
                }
            }

            tick_pos += bar_ticks;
        }
        // Final bar line at section end
        let x = grid_x + viewport.tick_to_x_local(tick_pos);
        if x >= grid_x && x <= grid_x + grid_w {
            frame.fill_rectangle(
                Point::new(x, TOOLBAR_HEIGHT),
                Size::new(1.0, grid_h),
                theme::LINE,
            );
        }

        // Subdivision lines (16th notes) when zoomed in enough — even
        // softer than beat lines so they don't compete.
        let sub = tpb / 4;
        let sub_px = sub as f32 * viewport.zoom_x;
        if sub_px >= 6.0 {
            let sub_color = Color {
                a: 0.5,
                ..theme::LINE_2
            };
            for idx in 0..=(total_ticks / sub) {
                let tick = idx * sub;
                if tick.is_multiple_of(tpb) {
                    continue;
                }
                let x = grid_x + viewport.tick_to_x_local(tick);
                if x < grid_x || x > grid_x + grid_w {
                    continue;
                }
                frame.fill_rectangle(
                    Point::new(x, TOOLBAR_HEIGHT),
                    Size::new(1.0, grid_h),
                    sub_color,
                );
            }
        }
    }

    fn draw_notes(
        &self,
        frame: &mut Frame,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
    ) {
        let grid_x = layout.grid_x();
        for clip in self
            .midi_clips
            .iter()
            .filter(|c| c.track_id == self.track_id)
        {
            if !self.clip_intersects_section(clip) {
                continue;
            }

            for n in &clip.notes {
                let rect = self.note_rect(layout, viewport, n);

                if rect.x + rect.width < grid_x
                    || rect.y + rect.height < layout.grid_top
                    || rect.y > layout.grid_top + layout.grid_h
                {
                    continue;
                }

                // Always paint with the brighter ACCENT stroke (no
                // selection state in this canvas); labels render inside
                // notes large enough to be readable.
                let style = NoteStyle {
                    stroke: theme::ACCENT,
                    stroke_width: 1.0,
                    label: Some(note_name(n.note)),
                };
                piano_roll::draw_note(frame, rect, n.velocity, style);
            }
        }
    }
}
