//! Drawing helpers and hit-test methods for [`ComposeTrackCanvas`].

use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{Color, Point, Rectangle, Size};

use resonance_audio::types::{TrackId, TICKS_PER_QUARTER_NOTE};

use crate::message::*;
use crate::state::MidiClipState;
use crate::theme;

use super::{
    pitch_count, snap_tick, ComposeTrackCanvas, ADD_BUTTON_SIZE, COLLAPSED_TRACK_HEIGHT,
    COMPOSE_TRACK_HEIGHT, DEFAULT_NEW_NOTE_TICKS, DEFAULT_NEW_NOTE_VELOCITY, NAME_COLUMN_WIDTH,
    NOTE_GRID_PAD, PITCH_RANGE_HIGH, PITCH_RANGE_LOW,
};

impl<'a> ComposeTrackCanvas<'a> {
    pub(super) fn track_row_rect(&self, index: usize, bounds: Rectangle) -> Rectangle {
        if self.expanded_track_id.is_none() {
            // Normal mode: all tracks at full height
            let y = index as f32 * COMPOSE_TRACK_HEIGHT - self.scroll_offset_y;
            return Rectangle {
                x: 0.0,
                y,
                width: bounds.width,
                height: COMPOSE_TRACK_HEIGHT,
            };
        }

        // Expanded mode: collapsed strips for non-expanded tracks.
        // The expanded track itself is not drawn in this canvas (it gets
        // the separate expanded_editor canvas), so all tracks shown here
        // are collapsed strips.
        let tracks = self.sorted_tracks();
        let mut y: f32 = 0.0;
        for (i, t) in tracks.iter().enumerate() {
            let h = if Some(t.id) == self.expanded_track_id {
                // The expanded track still gets a small strip in this
                // canvas so the user can double-click to collapse.
                COLLAPSED_TRACK_HEIGHT
            } else {
                COLLAPSED_TRACK_HEIGHT
            };
            if i == index {
                return Rectangle {
                    x: 0.0,
                    y: y - self.scroll_offset_y,
                    width: bounds.width,
                    height: h,
                };
            }
            y += h;
        }
        // Fallback (should not be reached)
        Rectangle {
            x: 0.0,
            y: index as f32 * COLLAPSED_TRACK_HEIGHT - self.scroll_offset_y,
            width: bounds.width,
            height: COLLAPSED_TRACK_HEIGHT,
        }
    }

    /// Total ticks in the section, summing per-bar numerators.
    pub(super) fn section_total_ticks(&self) -> u64 {
        (0..self.section_length_bars)
            .map(|b| {
                self.tempo_map.numerator_at_bar(self.start_bar + b) as u64
                    * TICKS_PER_QUARTER_NOTE
            })
            .sum()
    }

    /// Map a section-relative tick position to pixel x within `clip_width`.
    pub(super) fn tick_to_x(&self, tick: f64, clip_width: f32) -> f32 {
        let total = self.section_total_ticks() as f64;
        if total <= 0.0 {
            return 0.0;
        }
        (tick / total * clip_width as f64) as f32
    }

    /// Inverse of `tick_to_x`: pixel x to section-relative tick.
    pub(super) fn x_to_tick(&self, x: f32, clip_width: f32) -> f64 {
        let total = self.section_total_ticks() as f64;
        if clip_width <= 0.0 {
            return 0.0;
        }
        x as f64 / clip_width as f64 * total
    }

    /// Convert an absolute sample position to a section-relative tick.
    pub(super) fn sample_to_section_tick(&self, sample: u64) -> f64 {
        let (bar, frac) = self.tempo_map.sample_to_bar(sample, self.sample_rate);
        let mut tick: f64 = 0.0;
        if bar > self.start_bar {
            for b in self.start_bar..bar {
                tick +=
                    self.tempo_map.numerator_at_bar(b) as f64 * TICKS_PER_QUARTER_NOTE as f64;
            }
        } else if bar < self.start_bar {
            for b in bar..self.start_bar {
                tick -=
                    self.tempo_map.numerator_at_bar(b) as f64 * TICKS_PER_QUARTER_NOTE as f64;
            }
        }
        let bar_ticks =
            self.tempo_map.numerator_at_bar(bar) as f64 * TICKS_PER_QUARTER_NOTE as f64;
        tick += frac * bar_ticks;
        tick
    }

    /// Tick range of a clip within the section, or `None` if the clip
    /// doesn't overlap.
    pub(super) fn clip_tick_range(&self, clip: &MidiClipState) -> Option<(f64, f64)> {
        let clip_start_tick = self.sample_to_section_tick(clip.start_sample);
        let clip_end_tick = clip_start_tick + clip.duration_ticks as f64;
        let section_ticks = self.section_total_ticks() as f64;
        if clip_end_tick <= 0.0 || clip_start_tick >= section_ticks {
            return None;
        }
        Some((clip_start_tick.max(0.0), clip_end_tick.min(section_ticks)))
    }

    pub(super) fn pitch_to_y(&self, midi: u8, clip_area: Rectangle) -> f32 {
        let grid_h = clip_area.height - NOTE_GRID_PAD * 2.0;
        let clamped = midi.clamp(PITCH_RANGE_LOW, PITCH_RANGE_HIGH);
        let row_from_top = (PITCH_RANGE_HIGH - clamped) as f32;
        clip_area.y + NOTE_GRID_PAD + row_from_top * (grid_h / pitch_count() as f32)
    }

    pub(super) fn cell_height(&self, clip_area: Rectangle) -> f32 {
        (clip_area.height - NOTE_GRID_PAD * 2.0) / pitch_count() as f32
    }

    pub(super) fn y_to_pitch(&self, y: f32, clip_area: Rectangle) -> Option<u8> {
        let grid_top = clip_area.y + NOTE_GRID_PAD;
        let grid_h = clip_area.height - NOTE_GRID_PAD * 2.0;
        if y < grid_top || y > grid_top + grid_h {
            return None;
        }
        let row = ((y - grid_top) / (grid_h / pitch_count() as f32)) as i32;
        let note = PITCH_RANGE_HIGH as i32 - row;
        if note < PITCH_RANGE_LOW as i32 || note > PITCH_RANGE_HIGH as i32 {
            return None;
        }
        Some(note as u8)
    }

    pub(super) fn draw_grid_background(&self, frame: &mut Frame, clip_area: Rectangle) {
        // Card backdrop in BG_2 matches the redesign's piano-roll cards.
        // Black-key rows stay slightly darker (BG_1) so the keyboard
        // anatomy still reads; out-of-scale rows get a very subtle
        // warm tint to mark them as outside the section's mode.
        frame.fill_rectangle(
            Point::new(clip_area.x, clip_area.y),
            Size::new(clip_area.width, clip_area.height),
            theme::BG_2,
        );
        let cell_h = self.cell_height(clip_area);
        if cell_h <= 0.0 {
            return;
        }
        for midi in PITCH_RANGE_LOW..=PITCH_RANGE_HIGH {
            let y = self.pitch_to_y(midi, clip_area);
            let is_black = matches!(midi % 12, 1 | 3 | 6 | 8 | 10);
            let in_scale = self.scale.map(|s| s.contains(midi)).unwrap_or(true);
            let color = if !in_scale {
                Color {
                    a: 0.05,
                    ..theme::WARM
                }
            } else if is_black {
                theme::BG_1
            } else {
                // transparent — let the BG_2 backdrop show through
                continue;
            };
            frame.fill_rectangle(
                Point::new(clip_area.x, y),
                Size::new(clip_area.width, cell_h),
                color,
            );
            if midi % 12 == 0 && cell_h >= 2.0 {
                frame.fill_rectangle(
                    Point::new(clip_area.x, y + cell_h - 1.0),
                    Size::new(clip_area.width, 1.0),
                    theme::LINE_2,
                );
            }
        }
    }

    pub(super) fn draw_beat_grid(&self, frame: &mut Frame, clip_area: Rectangle) {
        if self.section_length_bars == 0 || clip_area.width <= 0.0 {
            return;
        }
        let mut tick_pos: u64 = 0;
        for bar_offset in 0..self.section_length_bars {
            let bar = self.start_bar + bar_offset;
            let num = self.tempo_map.numerator_at_bar(bar) as u64;
            let bar_ticks = num * TICKS_PER_QUARTER_NOTE;

            // Bar line — LINE, 1px (matches the redesign's hairline ruler).
            let x = clip_area.x + self.tick_to_x(tick_pos as f64, clip_area.width);
            frame.stroke(
                &Path::line(
                    Point::new(x, clip_area.y + NOTE_GRID_PAD),
                    Point::new(x, clip_area.y + clip_area.height - NOTE_GRID_PAD),
                ),
                Stroke::default().with_width(1.0).with_color(theme::LINE),
            );

            // Beat sub-lines — LINE_2 hairlines.
            for beat in 1..num {
                let beat_tick = tick_pos + beat * TICKS_PER_QUARTER_NOTE;
                let bx = clip_area.x + self.tick_to_x(beat_tick as f64, clip_area.width);
                frame.stroke(
                    &Path::line(
                        Point::new(bx, clip_area.y + NOTE_GRID_PAD),
                        Point::new(bx, clip_area.y + clip_area.height - NOTE_GRID_PAD),
                    ),
                    Stroke::default().with_width(1.0).with_color(theme::LINE_2),
                );
            }

            tick_pos += bar_ticks;
        }
        // Final bar line at section end
        let x = clip_area.x + self.tick_to_x(tick_pos as f64, clip_area.width);
        frame.stroke(
            &Path::line(
                Point::new(x, clip_area.y + NOTE_GRID_PAD),
                Point::new(x, clip_area.y + clip_area.height - NOTE_GRID_PAD),
            ),
            Stroke::default().with_width(1.0).with_color(theme::LINE),
        );
    }

    pub(super) fn draw_clip_outline(
        &self,
        frame: &mut Frame,
        clip_area: Rectangle,
        tick_range: (f64, f64),
    ) {
        let (vis_start, vis_end) = tick_range;
        let x = clip_area.x + self.tick_to_x(vis_start, clip_area.width);
        let right = clip_area.x + self.tick_to_x(vis_end, clip_area.width);
        let w = (right - x).max(2.0);
        let rect = Rectangle {
            x,
            y: clip_area.y + NOTE_GRID_PAD,
            width: w,
            height: clip_area.height - NOTE_GRID_PAD * 2.0,
        };
        // Lavender rounded outline matches the Arrange clip styling.
        frame.stroke(
            &Path::rounded_rectangle(
                Point::new(rect.x, rect.y),
                Size::new(rect.width, rect.height),
                6.0.into(),
            ),
            Stroke::default()
                .with_width(1.0)
                .with_color(theme::ACCENT_LINE),
        );
    }

    pub(super) fn draw_notes(
        &self,
        frame: &mut Frame,
        clip: &MidiClipState,
        clip_area: Rectangle,
        clip_start_tick: f64,
    ) {
        let cell_h = self.cell_height(clip_area);
        let total_ticks = self.section_total_ticks() as f64;
        for note in &clip.notes {
            let note_start_tick = clip_start_tick + note.start_tick as f64;
            let note_end_tick = note_start_tick + note.duration_ticks as f64;
            if note_end_tick <= 0.0 || note_start_tick >= total_ticks {
                continue;
            }
            let vs = note_start_tick.max(0.0);
            let ve = note_end_tick.min(total_ticks);
            let x = clip_area.x + self.tick_to_x(vs, clip_area.width);
            let right = clip_area.x + self.tick_to_x(ve, clip_area.width);
            let w = (right - x).max(2.0);
            let y = self.pitch_to_y(note.note, clip_area);
            let h = (cell_h - 1.0).max(2.0);
            // Lavender notes — velocity raises the alpha so harder hits
            // pop without changing hue. Rounded 2px corners match the
            // Arrange clip note style.
            let v = note.velocity.clamp(0.0, 1.0);
            let fill = Color {
                a: 0.55 + 0.40 * v,
                ..theme::ACCENT_SOFT
            };
            let body = if w >= 4.0 && h >= 4.0 {
                Path::rounded_rectangle(Point::new(x, y), Size::new(w, h), 2.0.into())
            } else {
                Path::rectangle(Point::new(x, y), Size::new(w, h))
            };
            frame.fill(&body, fill);
        }
    }

    pub(super) fn add_button_rect(&self, clip_area: Rectangle) -> Rectangle {
        Rectangle {
            x: clip_area.x + clip_area.width / 2.0 - ADD_BUTTON_SIZE / 2.0,
            y: clip_area.y + clip_area.height / 2.0 - ADD_BUTTON_SIZE / 2.0,
            width: ADD_BUTTON_SIZE,
            height: ADD_BUTTON_SIZE,
        }
    }

    pub(super) fn draw_add_button(&self, frame: &mut Frame, clip_area: Rectangle) {
        if clip_area.width < ADD_BUTTON_SIZE + 8.0 {
            return;
        }
        let r = self.add_button_rect(clip_area);
        frame.fill_rectangle(
            Point::new(r.x, r.y),
            Size::new(r.width, r.height),
            Color::from_rgba(0.0, 0.0, 0.0, 0.35),
        );
        frame.stroke(
            &Path::rectangle(Point::new(r.x, r.y), Size::new(r.width, r.height)),
            Stroke::default().with_width(1.0).with_color(theme::ACCENT),
        );
        frame.fill_text(canvas::Text {
            content: "+".to_string(),
            position: Point::new(r.x + r.width / 2.0 - 5.0, r.y + r.height / 2.0 - 11.0),
            color: theme::ACCENT,
            size: 22.0.into(),
            ..canvas::Text::default()
        });
    }

    /// Determine which track row (if any) the given point falls in,
    /// respecting expanded/collapsed layout.
    pub(super) fn hit_test_track(&self, pos: Point, bounds: Rectangle) -> Option<TrackId> {
        let tracks = self.sorted_tracks();
        for (idx, track) in tracks.iter().enumerate() {
            let row = self.track_row_rect(idx, bounds);
            if pos.y >= row.y && pos.y <= row.y + row.height {
                return Some(track.id);
            }
        }
        None
    }

    pub(super) fn hit_test_name_column(&self, pos: Point, bounds: Rectangle) -> Option<TrackId> {
        if pos.x >= NAME_COLUMN_WIDTH {
            return None;
        }
        let tracks = self.sorted_tracks();
        for (idx, track) in tracks.iter().enumerate() {
            let row = self.track_row_rect(idx, bounds);
            if pos.y >= row.y && pos.y <= row.y + row.height {
                return Some(track.id);
            }
        }
        None
    }

    pub(super) fn hit_test_add_button(&self, pos: Point, bounds: Rectangle) -> Option<TrackId> {
        if pos.x < NAME_COLUMN_WIDTH {
            return None;
        }
        let tracks = self.sorted_tracks();
        for (idx, track) in tracks.iter().enumerate() {
            let row = self.track_row_rect(idx, bounds);
            if pos.y < row.y || pos.y > row.y + row.height {
                continue;
            }
            let has_clip = self.midi_clips.iter().any(|c| {
                c.track_id == track.id && self.clip_tick_range(c).is_some()
            });
            if has_clip {
                return None;
            }
            let clip_area = Rectangle {
                x: row.x + NAME_COLUMN_WIDTH,
                y: row.y,
                width: (row.width - NAME_COLUMN_WIDTH).max(0.0),
                height: row.height,
            };
            let btn = self.add_button_rect(clip_area);
            if pos.x >= btn.x
                && pos.x <= btn.x + btn.width
                && pos.y >= btn.y
                && pos.y <= btn.y + btn.height
            {
                return Some(track.id);
            }
            return None;
        }
        None
    }

    pub(super) fn hit_test_note_edit(&self, pos: Point, bounds: Rectangle) -> Option<Message> {
        if pos.x < NAME_COLUMN_WIDTH {
            return None;
        }
        let tracks = self.sorted_tracks();
        let total_ticks = self.section_total_ticks() as f64;
        for (idx, track) in tracks.iter().enumerate() {
            let row = self.track_row_rect(idx, bounds);
            if pos.y < row.y || pos.y > row.y + row.height {
                continue;
            }
            let clip_area = Rectangle {
                x: row.x + NAME_COLUMN_WIDTH,
                y: row.y,
                width: (row.width - NAME_COLUMN_WIDTH).max(0.0),
                height: row.height,
            };
            let cell_h = self.cell_height(clip_area);

            // Check existing notes for removal
            for clip in self.midi_clips.iter().filter(|c| c.track_id == track.id) {
                let clip_start_tick = self.sample_to_section_tick(clip.start_sample);
                for (note_index, note) in clip.notes.iter().enumerate() {
                    let note_start_tick = clip_start_tick + note.start_tick as f64;
                    let note_end_tick = note_start_tick + note.duration_ticks as f64;
                    if note_end_tick <= 0.0 || note_start_tick >= total_ticks {
                        continue;
                    }
                    let vs = note_start_tick.max(0.0);
                    let ve = note_end_tick.min(total_ticks);
                    let x = clip_area.x + self.tick_to_x(vs, clip_area.width);
                    let right = clip_area.x + self.tick_to_x(ve, clip_area.width);
                    let y = self.pitch_to_y(note.note, clip_area);
                    let h = (cell_h - 1.0).max(2.0);
                    if pos.x >= x && pos.x <= right && pos.y >= y && pos.y <= y + h {
                        return Some(Message::MidiEditor(MidiEditorMessage::RemoveNote {
                            clip_id: clip.id,
                            note_index,
                        }));
                    }
                }
            }

            // Click in empty space: add note
            let pitch = self.y_to_pitch(pos.y, clip_area)?;
            let rel_x = pos.x - clip_area.x;
            if rel_x < 0.0 || rel_x > clip_area.width {
                return None;
            }
            let section_tick = self.x_to_tick(rel_x, clip_area.width);

            for clip in self.midi_clips.iter().filter(|c| c.track_id == track.id) {
                let clip_start_tick = self.sample_to_section_tick(clip.start_sample);
                let clip_end_tick = clip_start_tick + clip.duration_ticks as f64;
                if section_tick >= clip_start_tick && section_tick < clip_end_tick {
                    let raw_tick = (section_tick - clip_start_tick) as u64;
                    let snapped = snap_tick(raw_tick, DEFAULT_NEW_NOTE_TICKS);
                    return Some(Message::MidiEditor(MidiEditorMessage::AddNote {
                        clip_id: clip.id,
                        note: pitch,
                        start_tick: snapped,
                        duration_ticks: DEFAULT_NEW_NOTE_TICKS,
                        velocity: DEFAULT_NEW_NOTE_VELOCITY,
                    }));
                }
            }
            return None;
        }
        None
    }
}
