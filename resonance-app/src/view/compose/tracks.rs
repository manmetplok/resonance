use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{container, Canvas};
use iced::{mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use resonance_audio::types::{TrackId, TrackType, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::Scale;

use crate::compose::{ComposeMessage, SectionDefinitionState, SectionPlacementState};
use crate::message::*;
use crate::state::{InstrumentType, MidiClipState, TrackState};
use crate::theme;
use crate::Resonance;

/// Width reserved on the left edge of the canvas for an inline track-name
/// label. The Compose tab intentionally does not show the full track header
/// (mute/solo/arm/plugins) — that is Arrange-only territory. Clicking the
/// name column opens the instrument details view in the right-side panel.
pub const NAME_COLUMN_WIDTH: f32 = 110.0;
/// Row height used inside the Compose track area. Taller than the Arrange
/// track height so inline note editing has enough vertical room.
const COMPOSE_TRACK_HEIGHT: f32 = 160.0;
/// Top/bottom padding inside each row before the note grid starts.
const NOTE_GRID_PAD: f32 = 6.0;
/// Pitch range shown inline. C2 .. C6 covers most melodic writing and keeps
/// semitone cells tall enough to click reliably.
const PITCH_RANGE_LOW: u8 = 36; // C2
const PITCH_RANGE_HIGH: u8 = 84; // C6
const DEFAULT_NEW_NOTE_TICKS: u64 = TICKS_PER_QUARTER_NOTE;
const DEFAULT_NEW_NOTE_VELOCITY: f32 = 0.8;
/// Size of the "+" hint button drawn over empty instrument rows.
const ADD_BUTTON_SIZE: f32 = 32.0;

fn pitch_count() -> u8 {
    PITCH_RANGE_HIGH - PITCH_RANGE_LOW + 1
}

pub fn view<'a>(
    app: &'a Resonance,
    placement: &'a SectionPlacementState,
    definition: &'a SectionDefinitionState,
) -> Element<'a, Message> {
    let samples_per_bar = samples_per_bar(app);
    let section_start = placement.start_bar as u64 * samples_per_bar;
    let section_end = section_start + definition.length_bars as u64 * samples_per_bar;

    let cropped = Canvas::new(ComposeTrackCanvas {
        tracks: &app.registry.tracks,
        midi_clips: &app.midi_clips,
        section_start,
        section_end,
        section_length_bars: definition.length_bars,
        sample_rate: app.sample_rate,
        bpm: app.transport.bpm,
        scroll_offset_y: app.viewport.scroll_offset_y,
        scale: definition.scale,
        time_sig_num: app.transport.time_sig_num,
        details_track_id: app.compose.details_track_id,
    })
    .width(Length::Fill)
    .height(Length::Fill);

    container(cropped)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Cropped, inline-editable track canvas for the Compose tab. Only
/// instrument tracks are rendered. Each row shows a mini piano-roll grid
/// spanning PITCH_RANGE_LOW..=PITCH_RANGE_HIGH with the clip's notes drawn
/// as colored blocks. Click an empty cell to add a note, click an existing
/// note to remove it, click the "+" hint to spawn a clip that spans the
/// whole section, or click the name column on the left to open the
/// instrument details panel on the right side of the Compose tab.
pub struct ComposeTrackCanvas<'a> {
    pub tracks: &'a [TrackState],
    pub midi_clips: &'a [MidiClipState],
    pub section_start: u64,
    pub section_end: u64,
    pub section_length_bars: u32,
    pub sample_rate: u32,
    pub bpm: f32,
    pub scroll_offset_y: f32,
    pub scale: Option<Scale>,
    pub time_sig_num: u8,
    pub details_track_id: Option<TrackId>,
}

impl<'a> canvas::Program<Message> for ComposeTrackCanvas<'a> {
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

        let tracks = self.sorted_tracks();

        for (idx, track) in tracks.iter().enumerate() {
            let row_rect = self.track_row_rect(idx, bounds);
            if row_rect.y + row_rect.height < 0.0 || row_rect.y > bounds.height {
                continue;
            }

            // Row background — name column panel + grid area
            let is_selected_for_details = self.details_track_id == Some(track.id);
            let name_bg = if is_selected_for_details {
                Color::from_rgb(0.22, 0.22, 0.27)
            } else {
                theme::PANEL
            };
            frame.fill_rectangle(
                Point::new(0.0, row_rect.y),
                Size::new(NAME_COLUMN_WIDTH, row_rect.height),
                name_bg,
            );

            // Icon + name on the first line, instrument type on the second.
            frame.fill_text(canvas::Text {
                content: track.instrument_icon.glyph().to_string(),
                position: Point::new(10.0, row_rect.y + row_rect.height * 0.5 - 12.0),
                color: if is_selected_for_details { theme::ACCENT } else { theme::TEXT },
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
                content: track.instrument_type.as_str().to_string(),
                position: Point::new(10.0, row_rect.y + row_rect.height * 0.5 + 6.0),
                color: theme::TEXT_DIM,
                size: 10.0.into(),
                ..canvas::Text::default()
            });

            frame.fill_rectangle(
                Point::new(NAME_COLUMN_WIDTH, row_rect.y),
                Size::new(1.0, row_rect.height),
                if is_selected_for_details { theme::ACCENT } else { theme::SEPARATOR },
            );

            let clip_rect = Rectangle {
                x: row_rect.x + NAME_COLUMN_WIDTH,
                y: row_rect.y,
                width: (row_rect.width - NAME_COLUMN_WIDTH).max(0.0),
                height: row_rect.height,
            };

            self.draw_grid_background(&mut frame, clip_rect);
            self.draw_beat_grid(&mut frame, clip_rect);

            let mut has_clip_in_section = false;
            for clip in self.midi_clips.iter().filter(|c| c.track_id == track.id) {
                let clip_end = clip.start_sample + self.midi_clip_duration_samples(clip);
                if let Some(range) = self.clip_range(clip.start_sample, clip_end) {
                    has_clip_in_section = true;
                    self.draw_clip_outline(&mut frame, clip_rect, range);
                    self.draw_notes(&mut frame, clip, clip_rect);
                }
            }

            // Bottom separator between rows
            frame.fill_rectangle(
                Point::new(0.0, row_rect.y + row_rect.height - 1.0),
                Size::new(bounds.width, 1.0),
                theme::SEPARATOR,
            );

            if !has_clip_in_section {
                self.draw_add_button(&mut frame, clip_rect);
            }
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

            // Click on the name column opens the instrument details panel
            // on the right side of the Compose tab.
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

            // "+" hint button first — only hits rows with no clip
            if let Some(track_id) = self.hit_test_add_button(pos, bounds) {
                return (
                    canvas::event::Status::Captured,
                    Some(Message::Compose(
                        ComposeMessage::CreateMidiClipInSection {
                            track_id,
                            start_sample: self.section_start,
                            length_bars: self.section_length_bars,
                        },
                    )),
                );
            }
            if let Some(msg) = self.hit_test_note_edit(pos, bounds) {
                return (canvas::event::Status::Captured, Some(msg));
            }
        }
        (canvas::event::Status::Ignored, None)
    }
}

impl<'a> ComposeTrackCanvas<'a> {
    fn sorted_tracks(&self) -> Vec<&TrackState> {
        // Exclude sub-tracks: they don't accept MIDI (their audio comes
        // from their parent plugin's output port) and would clutter the
        // Compose instrument list with empty rows.
        let mut v: Vec<&TrackState> = self
            .tracks
            .iter()
            .filter(|t| {
                matches!(t.track_type, TrackType::Instrument)
                    && t.sub_track.is_none()
                    && t.instrument_type != InstrumentType::Drum
            })
            .collect();
        v.sort_by_key(|t| t.order);
        v
    }

    fn track_row_rect(&self, index: usize, bounds: Rectangle) -> Rectangle {
        let y = index as f32 * COMPOSE_TRACK_HEIGHT - self.scroll_offset_y;
        Rectangle {
            x: 0.0,
            y,
            width: bounds.width,
            height: COMPOSE_TRACK_HEIGHT,
        }
    }

    fn sample_to_x(&self, sample: u64, clip_width: f32) -> f32 {
        let span = (self.section_end - self.section_start) as f64;
        if span <= 0.0 {
            return 0.0;
        }
        let t = (sample as f64 - self.section_start as f64) / span;
        (t * clip_width as f64) as f32
    }

    fn clip_range(&self, start: u64, end: u64) -> Option<(u64, u64)> {
        if end <= self.section_start || start >= self.section_end {
            return None;
        }
        Some((start.max(self.section_start), end.min(self.section_end)))
    }

    fn pitch_to_y(&self, midi: u8, clip_area: Rectangle) -> f32 {
        let grid_h = clip_area.height - NOTE_GRID_PAD * 2.0;
        let clamped = midi.clamp(PITCH_RANGE_LOW, PITCH_RANGE_HIGH);
        let row_from_top = (PITCH_RANGE_HIGH - clamped) as f32;
        clip_area.y + NOTE_GRID_PAD + row_from_top * (grid_h / pitch_count() as f32)
    }

    fn cell_height(&self, clip_area: Rectangle) -> f32 {
        (clip_area.height - NOTE_GRID_PAD * 2.0) / pitch_count() as f32
    }

    fn y_to_pitch(&self, y: f32, clip_area: Rectangle) -> Option<u8> {
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

    fn draw_grid_background(&self, frame: &mut Frame, clip_area: Rectangle) {
        frame.fill_rectangle(
            Point::new(clip_area.x, clip_area.y),
            Size::new(clip_area.width, clip_area.height),
            Color::from_rgb(0.09, 0.09, 0.10),
        );
        let cell_h = self.cell_height(clip_area);
        if cell_h <= 0.0 {
            return;
        }
        for midi in PITCH_RANGE_LOW..=PITCH_RANGE_HIGH {
            let y = self.pitch_to_y(midi, clip_area);
            let is_black = matches!(midi % 12, 1 | 3 | 6 | 8 | 10);
            let in_scale = self.scale.map(|s| s.contains(midi)).unwrap_or(true);
            let base = if is_black {
                Color::from_rgb(0.07, 0.07, 0.08)
            } else {
                Color::from_rgb(0.11, 0.11, 0.12)
            };
            let color = if in_scale {
                base
            } else {
                Color::from_rgb(base.r * 0.75, base.g * 0.55, base.b * 0.55)
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
                    Color::from_rgb(0.22, 0.22, 0.24),
                );
            }
        }
    }

    fn draw_beat_grid(&self, frame: &mut Frame, clip_area: Rectangle) {
        let total_beats = self.section_length_bars * self.time_sig_num as u32;
        if total_beats == 0 || clip_area.width <= 0.0 {
            return;
        }
        let beat_w = clip_area.width / total_beats as f32;
        let bar_beats = self.time_sig_num as u32;
        for beat in 0..=total_beats {
            let x = clip_area.x + beat as f32 * beat_w;
            let is_bar = beat % bar_beats == 0;
            let color = if is_bar {
                Color::from_rgb(0.30, 0.30, 0.34)
            } else {
                Color::from_rgb(0.18, 0.18, 0.20)
            };
            frame.stroke(
                &Path::line(
                    Point::new(x, clip_area.y + NOTE_GRID_PAD),
                    Point::new(x, clip_area.y + clip_area.height - NOTE_GRID_PAD),
                ),
                Stroke::default()
                    .with_width(if is_bar { 1.5 } else { 1.0 })
                    .with_color(color),
            );
        }
    }

    fn draw_clip_outline(&self, frame: &mut Frame, clip_area: Rectangle, range: (u64, u64)) {
        let (vis_start, vis_end) = range;
        let x = clip_area.x + self.sample_to_x(vis_start, clip_area.width);
        let right = clip_area.x + self.sample_to_x(vis_end, clip_area.width);
        let w = (right - x).max(2.0);
        let rect = Rectangle {
            x,
            y: clip_area.y + NOTE_GRID_PAD,
            width: w,
            height: clip_area.height - NOTE_GRID_PAD * 2.0,
        };
        frame.stroke(
            &Path::rectangle(Point::new(rect.x, rect.y), Size::new(rect.width, rect.height)),
            Stroke::default()
                .with_width(1.0)
                .with_color(Color::from_rgba(0.38, 0.58, 0.38, 0.55)),
        );
    }

    fn draw_notes(&self, frame: &mut Frame, clip: &MidiClipState, clip_area: Rectangle) {
        let samples_per_tick = self.samples_per_tick();
        let cell_h = self.cell_height(clip_area);
        for note in &clip.notes {
            let note_start = clip.start_sample + (note.start_tick as f64 * samples_per_tick) as u64;
            let note_end = note_start + (note.duration_ticks as f64 * samples_per_tick) as u64;
            let Some((vs, ve)) = self.clip_range(note_start, note_end) else {
                continue;
            };
            let x = clip_area.x + self.sample_to_x(vs, clip_area.width);
            let right = clip_area.x + self.sample_to_x(ve, clip_area.width);
            let w = (right - x).max(2.0);
            let y = self.pitch_to_y(note.note, clip_area);
            let h = (cell_h - 1.0).max(2.0);
            let v = note.velocity.clamp(0.0, 1.0);
            let fill = Color::from_rgb(0.45 + 0.25 * v, 0.72, 0.42);
            frame.fill_rectangle(Point::new(x, y), Size::new(w, h), fill);
            frame.stroke(
                &Path::rectangle(Point::new(x, y), Size::new(w, h)),
                Stroke::default()
                    .with_width(1.0)
                    .with_color(Color::from_rgb(0.12, 0.22, 0.12)),
            );
        }
    }

    fn add_button_rect(&self, clip_area: Rectangle) -> Rectangle {
        Rectangle {
            x: clip_area.x + clip_area.width / 2.0 - ADD_BUTTON_SIZE / 2.0,
            y: clip_area.y + clip_area.height / 2.0 - ADD_BUTTON_SIZE / 2.0,
            width: ADD_BUTTON_SIZE,
            height: ADD_BUTTON_SIZE,
        }
    }

    fn draw_add_button(&self, frame: &mut Frame, clip_area: Rectangle) {
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

    fn hit_test_name_column(&self, pos: Point, bounds: Rectangle) -> Option<TrackId> {
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

    fn hit_test_add_button(&self, pos: Point, bounds: Rectangle) -> Option<TrackId> {
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
                c.track_id == track.id
                    && self
                        .clip_range(
                            c.start_sample,
                            c.start_sample + self.midi_clip_duration_samples(c),
                        )
                        .is_some()
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

    fn hit_test_note_edit(&self, pos: Point, bounds: Rectangle) -> Option<Message> {
        if pos.x < NAME_COLUMN_WIDTH {
            return None;
        }
        let tracks = self.sorted_tracks();
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
            let samples_per_tick = self.samples_per_tick();
            let cell_h = self.cell_height(clip_area);

            for clip in self.midi_clips.iter().filter(|c| c.track_id == track.id) {
                for (note_index, note) in clip.notes.iter().enumerate() {
                    let note_start =
                        clip.start_sample + (note.start_tick as f64 * samples_per_tick) as u64;
                    let note_end =
                        note_start + (note.duration_ticks as f64 * samples_per_tick) as u64;
                    let Some((vs, ve)) = self.clip_range(note_start, note_end) else {
                        continue;
                    };
                    let x = clip_area.x + self.sample_to_x(vs, clip_area.width);
                    let right = clip_area.x + self.sample_to_x(ve, clip_area.width);
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

            let Some(pitch) = self.y_to_pitch(pos.y, clip_area) else {
                return None;
            };
            let rel_x = pos.x - clip_area.x;
            if rel_x < 0.0 || rel_x > clip_area.width {
                return None;
            }
            let span = (self.section_end - self.section_start) as f64;
            let abs_sample =
                self.section_start as f64 + (rel_x as f64 / clip_area.width as f64) * span;
            let abs_sample = abs_sample as u64;

            for clip in self.midi_clips.iter().filter(|c| c.track_id == track.id) {
                let clip_end = clip.start_sample + self.midi_clip_duration_samples(clip);
                if abs_sample >= clip.start_sample && abs_sample < clip_end {
                    let offset_samples = abs_sample - clip.start_sample;
                    let raw_tick = (offset_samples as f64 / samples_per_tick) as u64;
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

    fn samples_per_tick(&self) -> f64 {
        self.sample_rate as f64 * 60.0 / self.bpm as f64 / TICKS_PER_QUARTER_NOTE as f64
    }

    fn midi_clip_duration_samples(&self, clip: &MidiClipState) -> u64 {
        (clip.duration_ticks as f64 * self.samples_per_tick()) as u64
    }
}

fn snap_tick(tick: u64, snap: u64) -> u64 {
    if snap == 0 {
        return tick;
    }
    (tick / snap) * snap
}

fn samples_per_bar(app: &Resonance) -> u64 {
    let samples_per_beat = app.sample_rate as f64 * 60.0 / app.transport.bpm as f64;
    (samples_per_beat * app.transport.time_sig_num as f64) as u64
}
