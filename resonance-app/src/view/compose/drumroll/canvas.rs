use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use resonance_audio::types::{ClipId, TrackId, TrackType, TICKS_PER_QUARTER_NOTE};

use crate::compose::drumroll::{DrumPadMap, DrumrollMessage};
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::state::{InstrumentType, MidiClipState, TrackState};
use crate::theme;

use super::super::tracks::NAME_COLUMN_WIDTH;

/// Row height of a single pad row in the drum grid. 12 rows per track =
/// 168px + the header gives a touch under 180px per drum track — slightly
/// taller than a synth row, which is fine for the denser hit grid.
const PAD_ROW_HEIGHT: f32 = 14.0;
const HEADER_HEIGHT: f32 = 20.0;
const PAD_LABEL_WIDTH: f32 = 84.0;
const NUM_PADS: usize = 12;

pub const DRUM_TRACK_HEIGHT: f32 = HEADER_HEIGHT + PAD_ROW_HEIGHT * NUM_PADS as f32;

/// Read-only canvas that renders a 12-pad drum grid for every drum-type
/// instrument track in the current section. Mirrors
/// `ComposeTrackCanvas` but with a fixed pad layout instead of a pitch grid.
pub struct ComposeDrumCanvas<'a> {
    pub tracks: &'a [TrackState],
    pub midi_clips: &'a [MidiClipState],
    pub pad_map: &'a DrumPadMap,
    pub section_start: u64,
    pub section_end: u64,
    pub section_length_bars: u32,
    pub steps_per_bar: u32,
    pub sample_rate: u32,
    pub bpm: f32,
    pub time_sig_num: u8,
    pub scroll_offset_y: f32,
    pub details_track_id: Option<TrackId>,
    pub selected_pad: Option<usize>,
}

impl<'a> canvas::Program<Message> for ComposeDrumCanvas<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BG);

        if self.section_end <= self.section_start || bounds.width <= 0.0 {
            return vec![frame.into_geometry()];
        }

        let tracks = self.sorted_drum_tracks();
        for (idx, track) in tracks.iter().enumerate() {
            let row_rect = self.track_row_rect(idx, bounds);
            if row_rect.y + row_rect.height < 0.0 || row_rect.y > bounds.height {
                continue;
            }

            self.draw_track_row(&mut frame, track, row_rect, bounds);
        }

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        _state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        if let canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            let Some(pos) = cursor.position_in(bounds) else {
                return (canvas::event::Status::Ignored, None);
            };

            // Click the name column: open the instrument details panel
            // (matches the synth canvas behavior).
            if pos.x < NAME_COLUMN_WIDTH {
                if let Some(track_id) = self.hit_test_name_column(pos, bounds) {
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::Compose(
                            ComposeMessage::SelectInstrumentForDetails { track_id },
                        )),
                    );
                }
                return (canvas::event::Status::Ignored, None);
            }

            // Pad label column (inside the clip area, left edge): pick the pad.
            if let Some(pad_index) = self.hit_test_pad_label(pos, bounds) {
                return (
                    canvas::event::Status::Captured,
                    Some(Message::Compose(ComposeMessage::Drumroll(
                        DrumrollMessage::SelectPad { pad_index },
                    ))),
                );
            }

            // Empty-state "+" button: create a clip that spans the section.
            if let Some(track_id) = self.hit_test_add_button(pos, bounds) {
                return (
                    canvas::event::Status::Captured,
                    Some(Message::Compose(ComposeMessage::CreateMidiClipInSection {
                        track_id,
                        start_sample: self.section_start,
                        length_bars: self.section_length_bars,
                    })),
                );
            }

            // Grid cell click: toggle the step.
            if let Some((clip_id, pad_index, step)) = self.hit_test_step(pos, bounds) {
                return (
                    canvas::event::Status::Captured,
                    Some(Message::Compose(ComposeMessage::Drumroll(
                        DrumrollMessage::ToggleStep { clip_id, pad_index, step },
                    ))),
                );
            }
        }
        (canvas::event::Status::Ignored, None)
    }
}

impl<'a> ComposeDrumCanvas<'a> {
    pub fn sorted_drum_tracks(&self) -> Vec<&TrackState> {
        let mut v: Vec<&TrackState> = self
            .tracks
            .iter()
            .filter(|t| {
                matches!(t.track_type, TrackType::Instrument)
                    && t.sub_track.is_none()
                    && t.instrument_type == InstrumentType::Drum
            })
            .collect();
        v.sort_by_key(|t| t.order);
        v
    }

    fn track_row_rect(&self, index: usize, bounds: Rectangle) -> Rectangle {
        let y = index as f32 * DRUM_TRACK_HEIGHT - self.scroll_offset_y;
        Rectangle { x: 0.0, y, width: bounds.width, height: DRUM_TRACK_HEIGHT }
    }

    fn clip_area(&self, row: Rectangle) -> Rectangle {
        Rectangle {
            x: row.x + NAME_COLUMN_WIDTH,
            y: row.y,
            width: (row.width - NAME_COLUMN_WIDTH).max(0.0),
            height: row.height,
        }
    }

    fn grid_area(&self, clip_area: Rectangle) -> Rectangle {
        // Reserve the left edge for the pad labels, and the top for a
        // step header.
        Rectangle {
            x: clip_area.x + PAD_LABEL_WIDTH,
            y: clip_area.y + HEADER_HEIGHT,
            width: (clip_area.width - PAD_LABEL_WIDTH).max(0.0),
            height: (clip_area.height - HEADER_HEIGHT).max(0.0),
        }
    }

    fn total_steps(&self) -> u32 {
        self.section_length_bars * self.steps_per_bar
    }

    fn step_ticks(&self) -> u64 {
        let ticks_per_bar = TICKS_PER_QUARTER_NOTE * self.time_sig_num as u64;
        if self.steps_per_bar == 0 {
            ticks_per_bar
        } else {
            ticks_per_bar / self.steps_per_bar as u64
        }
    }

    fn samples_per_tick(&self) -> f64 {
        self.sample_rate as f64 * 60.0 / self.bpm as f64 / TICKS_PER_QUARTER_NOTE as f64
    }

    fn midi_clip_duration_samples(&self, clip: &MidiClipState) -> u64 {
        (clip.duration_ticks as f64 * self.samples_per_tick()) as u64
    }

    fn pad_row_y(&self, grid_area: Rectangle, pad_index: usize) -> f32 {
        grid_area.y + pad_index as f32 * PAD_ROW_HEIGHT
    }

    /// Find the clip on `track_id` that overlaps the current section, if any.
    /// Drumroll edits only target the first overlapping clip — multiple clips
    /// in one section on a drum track is not a supported layout.
    fn find_section_clip(&self, track_id: TrackId) -> Option<&'a MidiClipState> {
        self.midi_clips.iter().find(|c| {
            c.track_id == track_id && {
                let end = c.start_sample + self.midi_clip_duration_samples(c);
                end > self.section_start && c.start_sample < self.section_end
            }
        })
    }

    fn draw_track_row(
        &self,
        frame: &mut Frame,
        track: &TrackState,
        row_rect: Rectangle,
        bounds: Rectangle,
    ) {
        let is_selected = self.details_track_id == Some(track.id);

        // Name column (matches the synth track style).
        let name_bg = if is_selected {
            Color::from_rgb(0.22, 0.22, 0.27)
        } else {
            theme::PANEL
        };
        frame.fill_rectangle(
            Point::new(0.0, row_rect.y),
            Size::new(NAME_COLUMN_WIDTH, row_rect.height),
            name_bg,
        );
        frame.fill_text(canvas::Text {
            content: track.instrument_icon.glyph().to_string(),
            position: Point::new(10.0, row_rect.y + row_rect.height * 0.5 - 12.0),
            color: if is_selected { theme::ACCENT } else { theme::TEXT },
            size: 14.0.into(),
            font: theme::ICON_FONT,
            ..canvas::Text::default()
        });
        frame.fill_text(canvas::Text {
            content: track.name.clone(),
            position: Point::new(32.0, row_rect.y + row_rect.height * 0.5 - 12.0),
            color: theme::TEXT,
            size: 12.0.into(),
            ..canvas::Text::default()
        });
        frame.fill_text(canvas::Text {
            content: "Drums".to_string(),
            position: Point::new(10.0, row_rect.y + row_rect.height * 0.5 + 6.0),
            color: theme::TEXT_DIM,
            size: 10.0.into(),
            ..canvas::Text::default()
        });
        frame.fill_rectangle(
            Point::new(NAME_COLUMN_WIDTH, row_rect.y),
            Size::new(1.0, row_rect.height),
            if is_selected { theme::ACCENT } else { theme::SEPARATOR },
        );

        let clip_area = self.clip_area(row_rect);
        // Grid background.
        frame.fill_rectangle(
            Point::new(clip_area.x, clip_area.y),
            Size::new(clip_area.width, clip_area.height),
            Color::from_rgb(0.09, 0.09, 0.10),
        );

        // Empty state: show "+" button centered in the clip area.
        let clip = self.find_section_clip(track.id);
        if clip.is_none() {
            self.draw_add_button(frame, clip_area);
            self.draw_bottom_separator(frame, row_rect, bounds);
            return;
        }
        let clip = clip.unwrap();

        let grid_area = self.grid_area(clip_area);

        self.draw_pad_label_column(frame, clip_area);
        self.draw_step_header(frame, clip_area, grid_area);
        self.draw_pad_rows(frame, grid_area);
        self.draw_step_grid(frame, grid_area);
        self.draw_lit_cells(frame, grid_area, clip);
        self.draw_bottom_separator(frame, row_rect, bounds);
    }

    fn draw_bottom_separator(&self, frame: &mut Frame, row: Rectangle, bounds: Rectangle) {
        frame.fill_rectangle(
            Point::new(0.0, row.y + row.height - 1.0),
            Size::new(bounds.width, 1.0),
            theme::SEPARATOR,
        );
    }

    fn draw_pad_label_column(&self, frame: &mut Frame, clip_area: Rectangle) {
        // Header cell behind the labels.
        frame.fill_rectangle(
            Point::new(clip_area.x, clip_area.y),
            Size::new(PAD_LABEL_WIDTH, clip_area.height),
            Color::from_rgb(0.12, 0.12, 0.14),
        );
        for (idx, pad) in self.pad_map.pads.iter().enumerate() {
            let y = clip_area.y + HEADER_HEIGHT + idx as f32 * PAD_ROW_HEIGHT;
            let is_selected = self.selected_pad == Some(idx);
            if is_selected {
                frame.fill_rectangle(
                    Point::new(clip_area.x, y),
                    Size::new(PAD_LABEL_WIDTH, PAD_ROW_HEIGHT),
                    Color::from_rgb(0.20, 0.20, 0.26),
                );
            }
            // Color swatch on the left edge.
            frame.fill_rectangle(
                Point::new(clip_area.x + 2.0, y + 2.0),
                Size::new(4.0, PAD_ROW_HEIGHT - 4.0),
                Color::from_rgb(pad.color[0], pad.color[1], pad.color[2]),
            );
            frame.fill_text(canvas::Text {
                content: pad.name.to_string(),
                position: Point::new(clip_area.x + 10.0, y + 1.0),
                color: theme::TEXT,
                size: 10.0.into(),
                ..canvas::Text::default()
            });
        }
        // Divider between labels and grid.
        frame.fill_rectangle(
            Point::new(clip_area.x + PAD_LABEL_WIDTH, clip_area.y),
            Size::new(1.0, clip_area.height),
            theme::SEPARATOR,
        );
    }

    fn draw_step_header(&self, frame: &mut Frame, clip_area: Rectangle, grid_area: Rectangle) {
        frame.fill_rectangle(
            Point::new(grid_area.x, clip_area.y),
            Size::new(grid_area.width, HEADER_HEIGHT),
            Color::from_rgb(0.12, 0.12, 0.14),
        );
        let total_steps = self.total_steps();
        if total_steps == 0 || grid_area.width <= 0.0 {
            return;
        }
        let step_w = grid_area.width / total_steps as f32;
        // Label every bar.
        for bar in 0..=self.section_length_bars {
            let x = grid_area.x + bar as f32 * self.steps_per_bar as f32 * step_w;
            frame.fill_text(canvas::Text {
                content: format!("{}", bar + 1),
                position: Point::new(x + 2.0, clip_area.y + 4.0),
                color: theme::TEXT_DIM,
                size: 10.0.into(),
                ..canvas::Text::default()
            });
        }
    }

    fn draw_pad_rows(&self, frame: &mut Frame, grid_area: Rectangle) {
        // Alternate zebra striping so adjacent pad rows are distinguishable.
        for (idx, _pad) in self.pad_map.pads.iter().enumerate() {
            let y = self.pad_row_y(grid_area, idx);
            let is_selected = self.selected_pad == Some(idx);
            let base = if idx % 2 == 0 {
                Color::from_rgb(0.10, 0.10, 0.11)
            } else {
                Color::from_rgb(0.12, 0.12, 0.13)
            };
            let bg = if is_selected {
                Color::from_rgb(0.16, 0.16, 0.22)
            } else {
                base
            };
            frame.fill_rectangle(
                Point::new(grid_area.x, y),
                Size::new(grid_area.width, PAD_ROW_HEIGHT),
                bg,
            );
        }
    }

    fn draw_step_grid(&self, frame: &mut Frame, grid_area: Rectangle) {
        let total_steps = self.total_steps();
        if total_steps == 0 || grid_area.width <= 0.0 {
            return;
        }
        let step_w = grid_area.width / total_steps as f32;
        let beat_step = self.steps_per_bar / self.time_sig_num.max(1) as u32;
        let bar_step = self.steps_per_bar;
        for step in 0..=total_steps {
            let x = grid_area.x + step as f32 * step_w;
            let is_bar = step % bar_step == 0;
            let is_beat = beat_step > 0 && step % beat_step == 0;
            let (w, color) = if is_bar {
                (1.5, Color::from_rgb(0.32, 0.32, 0.36))
            } else if is_beat {
                (1.0, Color::from_rgb(0.22, 0.22, 0.26))
            } else {
                (1.0, Color::from_rgb(0.16, 0.16, 0.18))
            };
            frame.stroke(
                &Path::line(
                    Point::new(x, grid_area.y),
                    Point::new(x, grid_area.y + grid_area.height),
                ),
                Stroke::default().with_width(w).with_color(color),
            );
        }
    }

    fn draw_lit_cells(&self, frame: &mut Frame, grid_area: Rectangle, clip: &MidiClipState) {
        let total_steps = self.total_steps();
        if total_steps == 0 || grid_area.width <= 0.0 {
            return;
        }
        let step_w = grid_area.width / total_steps as f32;
        let step_ticks = self.step_ticks();
        if step_ticks == 0 {
            return;
        }
        for note in &clip.notes {
            let Some(pad_index) = self.pad_map.index_for_note(note.note) else {
                continue;
            };
            let step = note.start_tick / step_ticks;
            if step as u32 >= total_steps {
                continue;
            }
            let pad = &self.pad_map.pads[pad_index];
            let y = self.pad_row_y(grid_area, pad_index);
            let x = grid_area.x + step as f32 * step_w;
            let v = note.velocity.clamp(0.0, 1.0);
            let alpha = 0.55 + 0.45 * v;
            let cell = Rectangle {
                x: x + 1.0,
                y: y + 1.0,
                width: (step_w - 2.0).max(1.0),
                height: (PAD_ROW_HEIGHT - 2.0).max(1.0),
            };
            frame.fill_rectangle(
                Point::new(cell.x, cell.y),
                Size::new(cell.width, cell.height),
                Color::from_rgba(pad.color[0], pad.color[1], pad.color[2], alpha),
            );
            frame.stroke(
                &Path::rectangle(Point::new(cell.x, cell.y), Size::new(cell.width, cell.height)),
                Stroke::default()
                    .with_width(1.0)
                    .with_color(Color::from_rgba(0.0, 0.0, 0.0, 0.6)),
            );
        }
    }

    fn add_button_rect(&self, clip_area: Rectangle) -> Rectangle {
        let size = 32.0f32;
        Rectangle {
            x: clip_area.x + clip_area.width / 2.0 - size / 2.0,
            y: clip_area.y + clip_area.height / 2.0 - size / 2.0,
            width: size,
            height: size,
        }
    }

    fn draw_add_button(&self, frame: &mut Frame, clip_area: Rectangle) {
        if clip_area.width < 40.0 {
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

    // Hit-testing ---------------------------------------------------------

    fn hit_test_name_column(&self, pos: Point, bounds: Rectangle) -> Option<TrackId> {
        if pos.x >= NAME_COLUMN_WIDTH {
            return None;
        }
        for (idx, track) in self.sorted_drum_tracks().iter().enumerate() {
            let row = self.track_row_rect(idx, bounds);
            if pos.y >= row.y && pos.y <= row.y + row.height {
                return Some(track.id);
            }
        }
        None
    }

    fn hit_test_pad_label(&self, pos: Point, bounds: Rectangle) -> Option<usize> {
        for (idx, track) in self.sorted_drum_tracks().iter().enumerate() {
            let row = self.track_row_rect(idx, bounds);
            if pos.y < row.y || pos.y > row.y + row.height {
                continue;
            }
            // Only live once the clip exists (label column sits beside a
            // working grid).
            if self.find_section_clip(track.id).is_none() {
                return None;
            }
            let clip_area = self.clip_area(row);
            if pos.x < clip_area.x || pos.x > clip_area.x + PAD_LABEL_WIDTH {
                return None;
            }
            let grid_top = clip_area.y + HEADER_HEIGHT;
            if pos.y < grid_top {
                return None;
            }
            let pad_idx = ((pos.y - grid_top) / PAD_ROW_HEIGHT) as usize;
            if pad_idx < self.pad_map.len() {
                return Some(pad_idx);
            }
            return None;
        }
        None
    }

    fn hit_test_add_button(&self, pos: Point, bounds: Rectangle) -> Option<TrackId> {
        if pos.x < NAME_COLUMN_WIDTH {
            return None;
        }
        for (idx, track) in self.sorted_drum_tracks().iter().enumerate() {
            let row = self.track_row_rect(idx, bounds);
            if pos.y < row.y || pos.y > row.y + row.height {
                continue;
            }
            if self.find_section_clip(track.id).is_some() {
                return None;
            }
            let clip_area = self.clip_area(row);
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

    fn hit_test_step(&self, pos: Point, bounds: Rectangle) -> Option<(ClipId, usize, u32)> {
        for (idx, track) in self.sorted_drum_tracks().iter().enumerate() {
            let row = self.track_row_rect(idx, bounds);
            if pos.y < row.y || pos.y > row.y + row.height {
                continue;
            }
            let clip = self.find_section_clip(track.id)?;
            let clip_area = self.clip_area(row);
            let grid_area = self.grid_area(clip_area);
            if pos.x < grid_area.x
                || pos.x > grid_area.x + grid_area.width
                || pos.y < grid_area.y
                || pos.y > grid_area.y + grid_area.height
            {
                return None;
            }
            let pad_index = ((pos.y - grid_area.y) / PAD_ROW_HEIGHT) as usize;
            if pad_index >= self.pad_map.len() {
                return None;
            }
            let total_steps = self.total_steps();
            if total_steps == 0 || grid_area.width <= 0.0 {
                return None;
            }
            let step_w = grid_area.width / total_steps as f32;
            let step = ((pos.x - grid_area.x) / step_w) as u32;
            if step >= total_steps {
                return None;
            }
            // Restrict to steps that actually fall inside the clip (the
            // clip may be shorter than the whole section).
            let step_ticks = self.step_ticks();
            if step_ticks == 0 {
                return None;
            }
            let step_start_tick = step as u64 * step_ticks;
            if step_start_tick >= clip.duration_ticks {
                return None;
            }
            return Some((clip.id, pad_index, step));
        }
        None
    }
}
