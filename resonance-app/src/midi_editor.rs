/// Piano roll MIDI editor canvas for the Resonance DAW.
use iced::widget::canvas;
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use crate::message::*;
use crate::piano_roll::{
    self, hit_test_note, is_black_key, NoteEdge, NoteStyle, PianoRollLayout, PianoRollViewport,
    NOTE_COUNT,
};
use crate::state::MidiClipState;
use crate::theme;

use resonance_audio::types::{TrackId, TICKS_PER_QUARTER_NOTE};

/// Width of the piano keyboard area on the left side of the editor.
pub const KEYBOARD_WIDTH: f32 = 50.0;
/// Height of the velocity lane at the bottom of the editor.
const VELOCITY_LANE_HEIGHT: f32 = 40.0;
/// Default velocity for newly created notes.
const DEFAULT_VELOCITY: f32 = 0.8;

/// Data passed to the piano roll canvas for rendering.
#[derive(Debug)]
pub struct PianoRollCanvas<'a> {
    pub clip: &'a MidiClipState,
    pub track_id: TrackId,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub snap_ticks: u64,
    pub selected_note: Option<usize>,
    pub time_sig_num: u8,
}

/// Interaction mode being tracked during a drag operation.
#[derive(Debug, Clone)]
enum DragMode {
    /// Moving a note: (note_index, tick_offset_from_cursor).
    MoveNote {
        note_index: usize,
        start_tick_offset: i64,
    },
    /// Resizing a note from its right edge.
    ResizeNote { note_index: usize, anchor_tick: u64 },
}

/// Local state for the piano roll canvas, tracking drags and previews.
#[derive(Debug, Default)]
pub struct PianoRollState {
    drag: Option<DragMode>,
    previewing_note: Option<u8>,
    /// Cached drawn geometry — invalidated only when the fingerprint of
    /// the inputs (notes / scroll / zoom / selection / clip identity)
    /// changes. Without this the piano roll redrew on every hover and
    /// engine-event tick, which made window resize feel particularly
    /// chunky because every paint had to re-rasterize ~100 note rects.
    cache: canvas::Cache,
    cache_fingerprint: std::cell::Cell<PianoRollFingerprint>,
}

/// Minimal projection of the piano roll's inputs into a comparable
/// value. The draw routine asks for the current fingerprint, compares
/// it with what was used for the cached geometry, and only re-runs the
/// drawing closure when something visible has actually changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PianoRollFingerprint {
    pub clip_id: u64,
    pub notes_len: usize,
    /// Hash of the (note, start_tick, duration_ticks, velocity) tuples
    /// so an edit inside the clip invalidates the cache even when
    /// `notes_len` doesn't change.
    pub notes_hash: u64,
    pub scroll_x_bits: u32,
    pub scroll_y_bits: u32,
    pub zoom_x_bits: u32,
    pub zoom_y_bits: u32,
    pub snap_ticks: u64,
    pub selected_note: Option<usize>,
    pub time_sig_num: u8,
    pub drag_active: bool,
    pub preview_note: Option<u8>,
}

impl canvas::Program<Message> for PianoRollCanvas<'_> {
    type State = PianoRollState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let layout = self.layout(bounds);
        let viewport = self.viewport();
        let grid_x = layout.grid_x();
        let grid_h = layout.grid_h;

        match event {
            // --- Scroll ---
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                // Only handle wheel events when the cursor is actually over the
                // piano roll — otherwise scrolling the arrangement would also
                // scroll this editor.
                if cursor.position_in(bounds).is_none() {
                    return None;
                }
                // Horizontal scroll is handled by the outer `Scrollable`
                // that wraps this canvas now (see `view_midi_editor_panel`).
                // Returning `Ignored` lets the event bubble up. Vertical
                // pitch scroll stays inside the canvas because the
                // keyboard column needs to scroll in lockstep with the
                // note rows.
                match delta {
                    mouse::ScrollDelta::Lines { x, y } => {
                        if x.abs() > f32::EPSILON {
                            return None;
                        }
                        return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::ScrollY(-y * 30.0))).and_capture());
                    }
                    mouse::ScrollDelta::Pixels { x, y } => {
                        if x.abs() > f32::EPSILON {
                            return None;
                        }
                        return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::ScrollY(-y))).and_capture());
                    }
                }
            }

            // --- Mouse press ---
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    // Piano keyboard area: preview note
                    if pos.x < grid_x && pos.y < grid_h {
                        let note = viewport.y_local_to_note(pos.y);
                        state.previewing_note = Some(note);
                        return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::PreviewNote(
                                self.track_id,
                                note,
                            ))).and_capture());
                    }

                    // Velocity lane: not interactive for now (future: drag velocity bars)
                    if pos.y >= grid_h {
                        return None;
                    }

                    // Note grid area
                    if pos.x >= grid_x {
                        let click_tick = viewport.x_local_to_tick(pos.x - grid_x);
                        let click_note = viewport.y_local_to_note(pos.y);

                        // Check if clicking on an existing note
                        for (i, n) in self.clip.notes.iter().enumerate() {
                            let rect = self.note_rect(&layout, &viewport, n);
                            if let Some(edge) = hit_test_note(rect, pos) {
                                state.drag = Some(match edge {
                                    NoteEdge::ResizeRight => DragMode::ResizeNote {
                                        note_index: i,
                                        anchor_tick: n.start_tick,
                                    },
                                    NoteEdge::Body => {
                                        let tick_offset =
                                            n.start_tick as i64 - click_tick as i64;
                                        DragMode::MoveNote {
                                            note_index: i,
                                            start_tick_offset: tick_offset,
                                        }
                                    }
                                });
                                return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::SelectNote {
                                        note_index: Some(i),
                                    })).and_capture());
                            }
                        }

                        // Clicked empty space: create a new note
                        let snapped = self.snap(click_tick);
                        return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::AddNote {
                                clip_id: self.clip.id,
                                note: click_note,
                                start_tick: snapped,
                                duration_ticks: self.snap_ticks,
                                velocity: DEFAULT_VELOCITY,
                            })).and_capture());
                    }
                }
            }

            // --- Right-click: remove selected note ---
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    if pos.x >= grid_x && pos.y < grid_h {
                        for (i, n) in self.clip.notes.iter().enumerate() {
                            let rect = self.note_rect(&layout, &viewport, n);
                            if hit_test_note(rect, pos).is_some() {
                                return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::RemoveNote {
                                        clip_id: self.clip.id,
                                        note_index: i,
                                    })).and_capture());
                            }
                        }
                    }
                }
            }

            // --- Mouse move (drag) ---
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    match &state.drag {
                        Some(DragMode::MoveNote {
                            note_index,
                            start_tick_offset,
                            ..
                        }) if pos.x >= grid_x && pos.y < grid_h => {
                            let tick = viewport.x_local_to_tick(pos.x - grid_x);
                            let raw_tick = (tick as i64 + start_tick_offset).max(0) as u64;
                            let snapped_tick = self.snap(raw_tick);
                            let note = viewport.y_local_to_note(pos.y);
                            return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::MoveNote {
                                    clip_id: self.clip.id,
                                    note_index: *note_index,
                                    new_start_tick: snapped_tick,
                                    new_note: note,
                                })).and_capture());
                        }
                        Some(DragMode::ResizeNote {
                            note_index,
                            anchor_tick,
                        }) if pos.x >= grid_x => {
                            let tick = viewport.x_local_to_tick(pos.x - grid_x);
                            let snapped = self.snap(tick);
                            let new_dur =
                                snapped.saturating_sub(*anchor_tick).max(self.snap_ticks);
                            return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::ResizeNote {
                                    clip_id: self.clip.id,
                                    note_index: *note_index,
                                    new_duration_ticks: new_dur,
                                })).and_capture());
                        }
                        Some(_) | None => {}
                    }
                }
            }

            // --- Mouse release ---
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag = None;
                if let Some(note) = state.previewing_note.take() {
                    return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::StopPreview(
                            self.track_id,
                            note,
                        ))).and_capture());
                }
            }

            // --- Delete key: remove selected note ---
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Delete),
                ..
            })
            | iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Backspace),
                ..
            }) => {
                if let Some(idx) = self.selected_note {
                    if idx < self.clip.notes.len() {
                        return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::RemoveNote {
                                clip_id: self.clip.id,
                                note_index: idx,
                            })).and_capture());
                    }
                }
            }

            _ => {}
        }
        None
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let fp = self.fingerprint(state);
        if state.cache_fingerprint.get() != fp {
            state.cache.clear();
            state.cache_fingerprint.set(fp);
        }
        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            self.draw_into(frame, bounds);
        });
        vec![geometry]
    }
}

impl PianoRollCanvas<'_> {
    /// Layout for the bottom-panel piano roll: keyboard on the left,
    /// no toolbar, velocity lane below the grid.
    fn layout(&self, bounds: Rectangle) -> PianoRollLayout {
        PianoRollLayout {
            keyboard_w: KEYBOARD_WIDTH,
            grid_top: 0.0,
            grid_h: bounds.height - VELOCITY_LANE_HEIGHT,
        }
    }

    fn viewport(&self) -> PianoRollViewport {
        PianoRollViewport {
            zoom_x: self.zoom_x,
            zoom_y: self.zoom_y,
            scroll_x: self.scroll_x,
            scroll_y: self.scroll_y,
        }
    }

    /// Pixel rectangle for `note`, in canvas-local coordinates.
    fn note_rect(
        &self,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
        note: &resonance_audio::types::MidiNote,
    ) -> Rectangle {
        Rectangle {
            x: layout.grid_x() + viewport.tick_to_x_local(note.start_tick),
            y: layout.grid_top + viewport.note_to_y_local(note.note),
            width: viewport.duration_to_w(note.duration_ticks),
            height: viewport.zoom_y,
        }
    }

    /// Snap a tick value to the nearest grid position.
    fn snap(&self, tick: u64) -> u64 {
        if self.snap_ticks == 0 {
            return tick;
        }
        let half = self.snap_ticks / 2;
        ((tick + half) / self.snap_ticks) * self.snap_ticks
    }

    /// Hash of the inputs that affect the drawn geometry. Excludes
    /// `bounds.size()` because the cache invalidates on size change
    /// automatically (via `canvas::Cache::draw`), so adding it here
    /// would double the work during a resize.
    fn fingerprint(&self, state: &PianoRollState) -> PianoRollFingerprint {
        use std::hash::{Hash, Hasher};
        let mut nh = std::collections::hash_map::DefaultHasher::new();
        for n in &self.clip.notes {
            n.note.hash(&mut nh);
            n.start_tick.hash(&mut nh);
            n.duration_ticks.hash(&mut nh);
            n.velocity.to_bits().hash(&mut nh);
        }
        PianoRollFingerprint {
            clip_id: self.clip.id,
            notes_len: self.clip.notes.len(),
            notes_hash: nh.finish(),
            scroll_x_bits: self.scroll_x.to_bits(),
            scroll_y_bits: self.scroll_y.to_bits(),
            zoom_x_bits: self.zoom_x.to_bits(),
            zoom_y_bits: self.zoom_y.to_bits(),
            snap_ticks: self.snap_ticks,
            selected_note: self.selected_note,
            time_sig_num: self.time_sig_num,
            drag_active: state.drag.is_some(),
            preview_note: state.previewing_note,
        }
    }

    fn draw_into(&self, frame: &mut canvas::Frame, bounds: Rectangle) {
        let layout = self.layout(bounds);
        let viewport = self.viewport();
        let grid_x = layout.grid_x();
        let grid_w = bounds.width - grid_x;
        let grid_h = layout.grid_h;

        // --- Background ---
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BG);

        // --- Note row backgrounds ---
        self.draw_note_rows(frame, &viewport, grid_x, grid_w, grid_h);

        // --- Grid lines ---
        self.draw_grid_lines(frame, &viewport, grid_x, grid_w, grid_h);

        // --- Notes ---
        self.draw_notes(frame, &layout, &viewport);

        // --- Piano keyboard ---
        piano_roll::draw_keyboard(frame, &layout, &viewport);

        // --- Velocity lane ---
        self.draw_velocity_lane(frame, &viewport, grid_x, grid_w, grid_h, bounds.height);

        // --- Separator lines ---
        // Vertical separator between keyboard and grid
        frame.fill_rectangle(
            Point::new(grid_x, 0.0),
            Size::new(1.0, grid_h),
            theme::SEPARATOR,
        );
        // Horizontal separator between grid and velocity lane
        frame.fill_rectangle(
            Point::new(0.0, grid_h),
            Size::new(bounds.width, 1.0),
            theme::SEPARATOR,
        );
    }

    /// Draw alternating row backgrounds for each semitone.
    fn draw_note_rows(
        &self,
        frame: &mut canvas::Frame,
        viewport: &PianoRollViewport,
        grid_x: f32,
        grid_w: f32,
        grid_h: f32,
    ) {
        // Backdrop is BG_2; only black-key rows darken to BG_1. White
        // keys reuse the backdrop so the row striping reads softly.
        frame.fill_rectangle(
            Point::new(grid_x, 0.0),
            Size::new(grid_w, grid_h),
            theme::BG_2,
        );
        for midi_note in 0..NOTE_COUNT {
            let y = viewport.note_to_y_local(midi_note);
            let h = viewport.zoom_y;

            if y + h < 0.0 || y > grid_h {
                continue;
            }

            if is_black_key(midi_note) {
                frame.fill_rectangle(Point::new(grid_x, y), Size::new(grid_w, h), theme::BG_1);
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

    /// Draw vertical grid lines at beat and bar boundaries.
    fn draw_grid_lines(
        &self,
        frame: &mut canvas::Frame,
        viewport: &PianoRollViewport,
        grid_x: f32,
        grid_w: f32,
        grid_h: f32,
    ) {
        let ticks_per_beat = TICKS_PER_QUARTER_NOTE;
        let ticks_per_bar = TICKS_PER_QUARTER_NOTE * self.time_sig_num as u64;
        let pixels_per_beat = ticks_per_beat as f32 * viewport.zoom_x;

        // Determine visible tick range
        let start_tick = (viewport.scroll_x / viewport.zoom_x).max(0.0) as u64;
        let end_tick = ((viewport.scroll_x + grid_w) / viewport.zoom_x) as u64 + ticks_per_beat;

        // Draw beat lines
        if pixels_per_beat >= 8.0 {
            let first_beat = start_tick / ticks_per_beat;
            let last_beat = end_tick / ticks_per_beat + 1;

            for beat_idx in first_beat..=last_beat {
                let tick = beat_idx * ticks_per_beat;
                let x = grid_x + viewport.tick_to_x_local(tick);

                if x < grid_x || x > grid_x + grid_w {
                    continue;
                }

                let is_bar = tick.is_multiple_of(ticks_per_bar);
                let color = if is_bar {
                    theme::BAR_LINE
                } else {
                    theme::BEAT_LINE
                };

                frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, grid_h), color);
            }
        }

        // Draw subdivision lines (16th notes) if zoomed in enough
        let snap_px = self.snap_ticks as f32 * viewport.zoom_x;
        if snap_px >= 8.0 && self.snap_ticks < ticks_per_beat {
            let first = start_tick / self.snap_ticks;
            let last = end_tick / self.snap_ticks + 1;
            for idx in first..=last {
                let tick = idx * self.snap_ticks;
                if tick.is_multiple_of(ticks_per_beat) {
                    continue; // already drawn as beat/bar line
                }
                let x = grid_x + viewport.tick_to_x_local(tick);
                if x < grid_x || x > grid_x + grid_w {
                    continue;
                }
                frame.fill_rectangle(
                    Point::new(x, 0.0),
                    Size::new(1.0, grid_h),
                    Color {
                        a: 0.5,
                        ..theme::LINE_2
                    },
                );
            }
        }
    }

    /// Draw MIDI note rectangles on the grid.
    fn draw_notes(
        &self,
        frame: &mut canvas::Frame,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
    ) {
        let grid_x = layout.grid_x();
        for (i, n) in self.clip.notes.iter().enumerate() {
            let rect = self.note_rect(layout, viewport, n);

            if rect.x + rect.width < grid_x
                || rect.x > grid_x + 2000.0
                || rect.y + rect.height < 0.0
                || rect.y > layout.grid_h
            {
                continue;
            }

            let style = if self.selected_note == Some(i) {
                NoteStyle::selected()
            } else {
                NoteStyle::plain()
            };
            piano_roll::draw_note(frame, rect, n.velocity, style);
        }
    }

    /// Draw the velocity lane at the bottom.
    fn draw_velocity_lane(
        &self,
        frame: &mut canvas::Frame,
        viewport: &PianoRollViewport,
        grid_x: f32,
        grid_w: f32,
        grid_h: f32,
        total_h: f32,
    ) {
        let lane_y = grid_h + 1.0;
        let lane_h = total_h - grid_h - 1.0;

        // Lane background
        frame.fill_rectangle(
            Point::new(0.0, lane_y),
            Size::new(grid_x + grid_w, lane_h),
            theme::PANEL_DARK,
        );

        // "Vel" label
        frame.fill_text(canvas::Text {
            content: "Vel".to_string(),
            position: Point::new(4.0, lane_y + 2.0),
            color: theme::TEXT_DIM,
            size: 9.0.into(),
            ..canvas::Text::default()
        });

        // Velocity bars for each note
        for (i, n) in self.clip.notes.iter().enumerate() {
            let x = grid_x + viewport.tick_to_x_local(n.start_tick);
            let w = viewport.duration_to_w(n.duration_ticks).clamp(2.0, 6.0);

            if x + w < grid_x || x > grid_x + 2000.0 {
                continue;
            }

            let bar_h = n.velocity.clamp(0.0, 1.0) * (lane_h - 4.0);
            let bar_y = lane_y + lane_h - bar_h - 2.0;

            let is_selected = self.selected_note == Some(i);
            let color = if is_selected {
                theme::ACCENT
            } else {
                theme::ACCENT_SOFT
            };

            frame.fill_rectangle(Point::new(x, bar_y), Size::new(w, bar_h), color);
        }
    }
}
