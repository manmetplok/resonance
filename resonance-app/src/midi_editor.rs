/// Piano roll MIDI editor canvas for the Resonance DAW.
use iced::widget::canvas;
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use crate::message::*;
use crate::state::MidiClipState;
use crate::theme;

use resonance_audio::types::{TrackId, TICKS_PER_QUARTER_NOTE};

/// Width of the piano keyboard area on the left side of the editor.
pub const KEYBOARD_WIDTH: f32 = 50.0;
/// Height of the velocity lane at the bottom of the editor.
const VELOCITY_LANE_HEIGHT: f32 = 40.0;
/// Total number of MIDI note rows (0-127).
const NOTE_COUNT: u8 = 128;
/// Default velocity for newly created notes.
const DEFAULT_VELOCITY: f32 = 0.8;
/// Minimum resize threshold in pixels for the right edge of a note.
const RESIZE_EDGE_PX: f32 = 6.0;

/// Returns true if the given MIDI note number corresponds to a black key.
fn is_black_key(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

/// Returns a human-readable note name (e.g. "C4", "F#3").
fn note_name(note: u8) -> String {
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (note / 12) as i8 - 1;
    format!("{}{}", names[note as usize % 12], octave)
}

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
#[allow(dead_code)]
enum DragMode {
    /// Moving a note: (note_index, tick_offset_from_cursor, note_offset_from_cursor)
    MoveNote {
        note_index: usize,
        start_tick_offset: i64,
        original_note: u8,
        original_start_tick: u64,
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
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        let grid_x = KEYBOARD_WIDTH;
        let _grid_w = bounds.width - KEYBOARD_WIDTH;
        let grid_h = bounds.height - VELOCITY_LANE_HEIGHT;

        match event {
            // --- Scroll ---
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                // Only handle wheel events when the cursor is actually over the
                // piano roll — otherwise scrolling the arrangement would also
                // scroll this editor.
                if cursor.position_in(bounds).is_none() {
                    return (canvas::event::Status::Ignored, None);
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
                            return (canvas::event::Status::Ignored, None);
                        }
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::MidiEditor(MidiEditorMessage::ScrollY(-y * 30.0))),
                        );
                    }
                    mouse::ScrollDelta::Pixels { x, y } => {
                        if x.abs() > f32::EPSILON {
                            return (canvas::event::Status::Ignored, None);
                        }
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::MidiEditor(MidiEditorMessage::ScrollY(-y))),
                        );
                    }
                }
            }

            // --- Mouse press ---
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    // Piano keyboard area: preview note
                    if pos.x < grid_x && pos.y < grid_h {
                        let note = self.y_to_note(pos.y, grid_h);
                        state.previewing_note = Some(note);
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::MidiEditor(MidiEditorMessage::PreviewNote(
                                self.track_id,
                                note,
                            ))),
                        );
                    }

                    // Velocity lane: not interactive for now (future: drag velocity bars)
                    if pos.y >= grid_h {
                        return (canvas::event::Status::Ignored, None);
                    }

                    // Note grid area
                    if pos.x >= grid_x {
                        let click_tick = self.x_to_tick(pos.x - grid_x);
                        let click_note = self.y_to_note(pos.y, grid_h);

                        // Check if clicking on an existing note
                        for (i, n) in self.clip.notes.iter().enumerate() {
                            let nx = self.tick_to_x(n.start_tick);
                            let nw = self.duration_to_width(n.duration_ticks);
                            let ny = self.note_to_y(n.note, grid_h);
                            let nh = self.zoom_y;

                            let rel_x = pos.x - grid_x;
                            let rel_y = pos.y;

                            if rel_x >= nx && rel_x <= nx + nw && rel_y >= ny && rel_y <= ny + nh {
                                // Right edge: resize
                                if (nx + nw) - rel_x < RESIZE_EDGE_PX {
                                    state.drag = Some(DragMode::ResizeNote {
                                        note_index: i,
                                        anchor_tick: n.start_tick,
                                    });
                                    return (
                                        canvas::event::Status::Captured,
                                        Some(Message::MidiEditor(MidiEditorMessage::SelectNote {
                                            note_index: Some(i),
                                        })),
                                    );
                                }
                                // Body: move
                                let tick_offset = n.start_tick as i64 - click_tick as i64;
                                state.drag = Some(DragMode::MoveNote {
                                    note_index: i,
                                    start_tick_offset: tick_offset,
                                    original_note: n.note,
                                    original_start_tick: n.start_tick,
                                });
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::MidiEditor(MidiEditorMessage::SelectNote {
                                        note_index: Some(i),
                                    })),
                                );
                            }
                        }

                        // Clicked empty space: create a new note
                        let snapped = self.snap(click_tick);
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::MidiEditor(MidiEditorMessage::AddNote {
                                clip_id: self.clip.id,
                                note: click_note,
                                start_tick: snapped,
                                duration_ticks: self.snap_ticks,
                                velocity: DEFAULT_VELOCITY,
                            })),
                        );
                    }
                }
            }

            // --- Right-click: remove selected note ---
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    if pos.x >= grid_x && pos.y < grid_h {
                        let click_tick = self.x_to_tick(pos.x - grid_x);
                        let click_note = self.y_to_note(pos.y, grid_h);

                        // Find note under cursor
                        for (i, n) in self.clip.notes.iter().enumerate() {
                            let nx = self.tick_to_x(n.start_tick);
                            let nw = self.duration_to_width(n.duration_ticks);
                            let ny = self.note_to_y(n.note, grid_h);
                            let nh = self.zoom_y;

                            let rel_x = pos.x - grid_x;
                            if rel_x >= nx && rel_x <= nx + nw && pos.y >= ny && pos.y <= ny + nh {
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::MidiEditor(MidiEditorMessage::RemoveNote {
                                        clip_id: self.clip.id,
                                        note_index: i,
                                    })),
                                );
                            }
                        }
                        // Right-click on empty space: ignore
                        let _ = (click_tick, click_note);
                    }
                }
            }

            // --- Mouse move (drag) ---
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    match &state.drag {
                        Some(DragMode::MoveNote {
                            note_index,
                            start_tick_offset,
                            ..
                        }) => {
                            if pos.x >= grid_x && pos.y < grid_h {
                                let tick = self.x_to_tick(pos.x - grid_x);
                                let raw_tick = (tick as i64 + start_tick_offset).max(0) as u64;
                                let snapped_tick = self.snap(raw_tick);
                                let note = self.y_to_note(pos.y, grid_h);
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::MidiEditor(MidiEditorMessage::MoveNote {
                                        clip_id: self.clip.id,
                                        note_index: *note_index,
                                        new_start_tick: snapped_tick,
                                        new_note: note,
                                    })),
                                );
                            }
                        }
                        Some(DragMode::ResizeNote {
                            note_index,
                            anchor_tick,
                        }) => {
                            if pos.x >= grid_x {
                                let tick = self.x_to_tick(pos.x - grid_x);
                                let snapped = self.snap(tick);
                                let new_dur =
                                    snapped.saturating_sub(*anchor_tick).max(self.snap_ticks);
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::MidiEditor(MidiEditorMessage::ResizeNote {
                                        clip_id: self.clip.id,
                                        note_index: *note_index,
                                        new_duration_ticks: new_dur,
                                    })),
                                );
                            }
                        }
                        None => {}
                    }
                }
            }

            // --- Mouse release ---
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

            // --- Delete key: remove selected note ---
            canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Delete),
                ..
            })
            | canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Backspace),
                ..
            }) => {
                if let Some(idx) = self.selected_note {
                    if idx < self.clip.notes.len() {
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::MidiEditor(MidiEditorMessage::RemoveNote {
                                clip_id: self.clip.id,
                                note_index: idx,
                            })),
                        );
                    }
                }
            }

            _ => {}
        }
        (canvas::event::Status::Ignored, None)
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
        let grid_x = KEYBOARD_WIDTH;
        let grid_w = bounds.width - KEYBOARD_WIDTH;
        let grid_h = bounds.height - VELOCITY_LANE_HEIGHT;

        // --- Background ---
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BG);

        // --- Note row backgrounds ---
        self.draw_note_rows(frame, grid_x, grid_w, grid_h);

        // --- Grid lines ---
        self.draw_grid_lines(frame, grid_x, grid_w, grid_h);

        // --- Notes ---
        self.draw_notes(frame, grid_x, grid_h);

        // --- Piano keyboard ---
        self.draw_keyboard(frame, grid_h);

        // --- Velocity lane ---
        self.draw_velocity_lane(frame, grid_x, grid_w, grid_h, bounds.height);

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
}

impl PianoRollCanvas<'_> {
    // --- Coordinate conversions ---

    /// Convert a tick position to pixel x offset within the grid area.
    fn tick_to_x(&self, tick: u64) -> f32 {
        tick as f32 * self.zoom_x - self.scroll_x
    }

    /// Convert a pixel x offset within the grid area to a tick position.
    fn x_to_tick(&self, x: f32) -> u64 {
        let tick = (x + self.scroll_x) / self.zoom_x;
        if tick < 0.0 {
            0
        } else {
            tick as u64
        }
    }

    /// Convert a note duration in ticks to pixel width.
    fn duration_to_width(&self, ticks: u64) -> f32 {
        ticks as f32 * self.zoom_x
    }

    /// Convert a MIDI note number to pixel y position (top of the row).
    /// Note 127 is at the top, note 0 is at the bottom.
    fn note_to_y(&self, note: u8, _grid_h: f32) -> f32 {
        let row = (NOTE_COUNT - 1 - note) as f32;
        row * self.zoom_y - self.scroll_y
    }

    /// Convert a pixel y position to a MIDI note number.
    fn y_to_note(&self, y: f32, _grid_h: f32) -> u8 {
        let row = ((y + self.scroll_y) / self.zoom_y).floor() as i32;
        let note = (NOTE_COUNT as i32 - 1) - row;
        note.clamp(0, 127) as u8
    }

    /// Snap a tick value to the nearest grid position.
    fn snap(&self, tick: u64) -> u64 {
        if self.snap_ticks == 0 {
            return tick;
        }
        let half = self.snap_ticks / 2;
        ((tick + half) / self.snap_ticks) * self.snap_ticks
    }

    // --- Drawing helpers ---

    /// Draw alternating row backgrounds for each semitone.
    fn draw_note_rows(&self, frame: &mut canvas::Frame, grid_x: f32, grid_w: f32, grid_h: f32) {
        // Backdrop is BG_2; only black-key rows darken to BG_1. White
        // keys reuse the backdrop so the row striping reads softly.
        frame.fill_rectangle(
            Point::new(grid_x, 0.0),
            Size::new(grid_w, grid_h),
            theme::BG_2,
        );
        for midi_note in 0..NOTE_COUNT {
            let y = self.note_to_y(midi_note, grid_h);
            let h = self.zoom_y;

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
    fn draw_grid_lines(&self, frame: &mut canvas::Frame, grid_x: f32, grid_w: f32, grid_h: f32) {
        let ticks_per_beat = TICKS_PER_QUARTER_NOTE;
        let ticks_per_bar = TICKS_PER_QUARTER_NOTE * self.time_sig_num as u64;
        let pixels_per_beat = ticks_per_beat as f32 * self.zoom_x;

        // Determine visible tick range
        let start_tick = (self.scroll_x / self.zoom_x).max(0.0) as u64;
        let end_tick = ((self.scroll_x + grid_w) / self.zoom_x) as u64 + ticks_per_beat;

        // Draw beat lines
        if pixels_per_beat >= 8.0 {
            let first_beat = start_tick / ticks_per_beat;
            let last_beat = end_tick / ticks_per_beat + 1;

            for beat_idx in first_beat..=last_beat {
                let tick = beat_idx * ticks_per_beat;
                let x = grid_x + self.tick_to_x(tick);

                if x < grid_x || x > grid_x + grid_w {
                    continue;
                }

                let is_bar = tick % ticks_per_bar == 0;
                let color = if is_bar {
                    theme::BAR_LINE
                } else {
                    theme::BEAT_LINE
                };
                let width = if is_bar { 1.0 } else { 1.0 };

                frame.fill_rectangle(Point::new(x, 0.0), Size::new(width, grid_h), color);
            }
        }

        // Draw subdivision lines (16th notes) if zoomed in enough
        let snap_px = self.snap_ticks as f32 * self.zoom_x;
        if snap_px >= 8.0 && self.snap_ticks < ticks_per_beat {
            let first = start_tick / self.snap_ticks;
            let last = end_tick / self.snap_ticks + 1;
            for idx in first..=last {
                let tick = idx * self.snap_ticks;
                if tick % ticks_per_beat == 0 {
                    continue; // already drawn as beat/bar line
                }
                let x = grid_x + self.tick_to_x(tick);
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
    fn draw_notes(&self, frame: &mut canvas::Frame, grid_x: f32, grid_h: f32) {
        for (i, n) in self.clip.notes.iter().enumerate() {
            let x = grid_x + self.tick_to_x(n.start_tick);
            let w = self.duration_to_width(n.duration_ticks);
            let y = self.note_to_y(n.note, grid_h);
            let h = self.zoom_y;

            if x + w < grid_x || x > grid_x + 2000.0 || y + h < 0.0 || y > grid_h {
                continue;
            }

            // Lavender notes — velocity raises alpha so harder hits
            // are visually denser without changing hue.
            let v = n.velocity.clamp(0.0, 1.0);
            let note_color = Color {
                a: 0.55 + 0.40 * v,
                ..theme::ACCENT_SOFT
            };
            let body = if w >= 4.0 && h >= 4.0 {
                canvas::Path::rounded_rectangle(
                    Point::new(x, y),
                    Size::new(w, h),
                    2.0.into(),
                )
            } else {
                canvas::Path::rectangle(Point::new(x, y), Size::new(w, h))
            };
            frame.fill(&body, note_color);

            let is_selected = self.selected_note == Some(i);
            let stroke_color = if is_selected { theme::ACCENT } else { theme::ACCENT_LINE };
            let stroke_w = if is_selected { 1.5 } else { 1.0 };
            frame.stroke(
                &body,
                canvas::Stroke::default()
                    .with_color(stroke_color)
                    .with_width(stroke_w),
            );
        }
    }

    /// Draw the piano keyboard on the left side.
    fn draw_keyboard(&self, frame: &mut canvas::Frame, grid_h: f32) {
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(KEYBOARD_WIDTH, grid_h),
            theme::BG_2,
        );

        for midi_note in 0..NOTE_COUNT {
            let y = self.note_to_y(midi_note, grid_h);
            let h = self.zoom_y;

            if y + h < 0.0 || y > grid_h {
                continue;
            }

            let black = is_black_key(midi_note);
            let key_color = if black { theme::BG_0 } else { theme::BG_3 };
            let key_w = if black {
                KEYBOARD_WIDTH * 0.65
            } else {
                KEYBOARD_WIDTH - 1.0
            };

            frame.fill_rectangle(Point::new(0.0, y), Size::new(key_w, h - 1.0), key_color);

            if midi_note % 12 == 0 && h >= 8.0 {
                frame.fill_text(canvas::Text {
                    content: note_name(midi_note),
                    position: Point::new(2.0, y + 1.0),
                    color: theme::TEXT_3,
                    size: (h * 0.7).min(10.0).into(),
                    font: theme::MONO_FONT,
                    ..canvas::Text::default()
                });
            }
        }
        frame.fill_rectangle(
            Point::new(KEYBOARD_WIDTH - 1.0, 0.0),
            Size::new(1.0, grid_h),
            theme::LINE_2,
        );
    }

    /// Draw the velocity lane at the bottom.
    fn draw_velocity_lane(
        &self,
        frame: &mut canvas::Frame,
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
            let x = grid_x + self.tick_to_x(n.start_tick);
            let w = self.duration_to_width(n.duration_ticks).min(6.0).max(2.0);

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
