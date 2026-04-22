use std::time::Instant;

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{container, Canvas};
use iced::{mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use resonance_audio::types::{TempoMap, TrackId, TrackType, TICKS_PER_QUARTER_NOTE};
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
/// Height for collapsed track strips when another track is expanded.
const COLLAPSED_TRACK_HEIGHT: f32 = 36.0;
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
/// Maximum milliseconds between two clicks to count as a double-click.
const DOUBLE_CLICK_MS: u64 = 400;

fn pitch_count() -> u8 {
    PITCH_RANGE_HIGH - PITCH_RANGE_LOW + 1
}

pub fn view<'a>(
    app: &'a Resonance,
    placement: &'a SectionPlacementState,
    definition: &'a SectionDefinitionState,
) -> Element<'a, Message> {
    let section_start = app.tempo_map.bar_to_sample(placement.start_bar);
    let section_end = app.tempo_map.bar_to_sample(placement.start_bar + definition.length_bars);

    let cropped = Canvas::new(ComposeTrackCanvas {
        tracks: &app.registry.tracks,
        midi_clips: &app.midi_clips,
        section_start,
        section_end,
        section_length_bars: definition.length_bars,
        sample_rate: app.sample_rate,
        tempo_map: &app.tempo_map,
        start_bar: placement.start_bar,
        scroll_offset_y: app.viewport.scroll_offset_y,
        scale: definition.scale,
        details_track_id: app.compose.details_track_id(),
        expanded_track_id: app.compose.expanded_track_id,
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
    pub tempo_map: &'a TempoMap,
    pub start_bar: u32,
    pub scroll_offset_y: f32,
    pub scale: Option<Scale>,
    pub details_track_id: Option<TrackId>,
    /// When set, this track is expanded into the full editor; other tracks
    /// are rendered as collapsed name-only strips.
    pub expanded_track_id: Option<TrackId>,
}

/// Canvas-local state for double-click detection.
#[derive(Debug, Default)]
pub struct ComposeTrackCanvasState {
    last_click: Option<(Instant, TrackId)>,
}

impl<'a> canvas::Program<Message> for ComposeTrackCanvas<'a> {
    type State = ComposeTrackCanvasState;

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
        let is_expanded = self.expanded_track_id.is_some();

        for (idx, track) in tracks.iter().enumerate() {
            let row_rect = self.track_row_rect(idx, bounds);
            if row_rect.y + row_rect.height < 0.0 || row_rect.y > bounds.height {
                continue;
            }

            let is_selected_for_details = self.details_track_id == Some(track.id);
            let is_this_expanded = self.expanded_track_id == Some(track.id);

            if is_expanded {
                // --- Collapsed strip rendering ---
                let bg = if is_this_expanded {
                    Color::from_rgb(0.18, 0.22, 0.18)
                } else if is_selected_for_details {
                    Color::from_rgb(0.22, 0.22, 0.27)
                } else {
                    theme::PANEL
                };
                frame.fill_rectangle(
                    Point::new(0.0, row_rect.y),
                    Size::new(bounds.width, row_rect.height),
                    bg,
                );

                // Icon + name centered vertically
                frame.fill_text(canvas::Text {
                    content: track.instrument_icon.glyph().to_string(),
                    position: Point::new(10.0, row_rect.y + row_rect.height * 0.5 - 8.0),
                    color: if is_this_expanded {
                        theme::ACCENT
                    } else {
                        theme::TEXT
                    },
                    size: 12.0.into(),
                    font: theme::ICON_FONT,
                    ..canvas::Text::default()
                });
                frame.fill_text(canvas::Text {
                    content: track.name.clone(),
                    position: Point::new(30.0, row_rect.y + row_rect.height * 0.5 - 8.0),
                    color: if is_this_expanded {
                        theme::ACCENT
                    } else {
                        theme::TEXT
                    },
                    size: 12.0.into(),
                    ..canvas::Text::default()
                });

                // Hint text for expanded track
                if is_this_expanded {
                    frame.fill_text(canvas::Text {
                        content: "(editing - double-click to collapse)".to_string(),
                        position: Point::new(
                            NAME_COLUMN_WIDTH + 10.0,
                            row_rect.y + row_rect.height * 0.5 - 6.0,
                        ),
                        color: theme::TEXT_DIM,
                        size: 10.0.into(),
                        ..canvas::Text::default()
                    });
                }

                // Bottom separator
                frame.fill_rectangle(
                    Point::new(0.0, row_rect.y + row_rect.height - 1.0),
                    Size::new(bounds.width, 1.0),
                    theme::SEPARATOR,
                );
            } else {
                // --- Normal full-height rendering ---
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
                    color: if is_selected_for_details {
                        theme::ACCENT
                    } else {
                        theme::TEXT
                    },
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
                    if is_selected_for_details {
                        theme::ACCENT
                    } else {
                        theme::SEPARATOR
                    },
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
                    if let Some(tick_range) = self.clip_tick_range(clip) {
                        has_clip_in_section = true;
                        let clip_start_tick =
                            self.sample_to_section_tick(clip.start_sample);
                        self.draw_clip_outline(&mut frame, clip_rect, tick_range);
                        self.draw_notes(&mut frame, clip, clip_rect, clip_start_tick);
                    }
                }

                if !has_clip_in_section {
                    self.draw_add_button(&mut frame, clip_rect);
                }

                // Bottom separator between rows
                frame.fill_rectangle(
                    Point::new(0.0, row_rect.y + row_rect.height - 1.0),
                    Size::new(bounds.width, 1.0),
                    theme::SEPARATOR,
                );
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
        if let canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            let Some(pos) = cursor.position_in(bounds) else {
                return (canvas::event::Status::Ignored, None);
            };

            // Determine which track row was clicked.
            let clicked_track = self.hit_test_track(pos, bounds);

            // Double-click detection: expand/collapse a track.
            if let Some(track_id) = clicked_track {
                let now = Instant::now();
                if let Some((prev_time, prev_id)) = state.last_click {
                    if prev_id == track_id
                        && now.duration_since(prev_time).as_millis() < DOUBLE_CLICK_MS as u128
                    {
                        state.last_click = None;
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::Compose(ComposeMessage::ExpandTrack { track_id })),
                        );
                    }
                }
                state.last_click = Some((now, track_id));
            }

            // When tracks are expanded, only handle name-column clicks
            // on collapsed strips (to select for details).
            if self.expanded_track_id.is_some() {
                if pos.x < NAME_COLUMN_WIDTH {
                    if let Some(track_id) = clicked_track {
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::Compose(ComposeMessage::SelectLane(
                                crate::compose::SelectedLane::Instrument(track_id),
                            ))),
                        );
                    }
                }
                return (canvas::event::Status::Ignored, None);
            }

            // Normal (non-expanded) behaviour below.

            // Click on the name column opens the instrument details panel
            // on the right side of the Compose tab.
            if pos.x < NAME_COLUMN_WIDTH {
                if let Some(track_id) = self.hit_test_name_column(pos, bounds) {
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::Compose(ComposeMessage::SelectLane(
                            crate::compose::SelectedLane::Instrument(track_id),
                        ))),
                    );
                }
                return (canvas::event::Status::Ignored, None);
            }

            // "+" hint button first — only hits rows with no clip
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
    fn section_total_ticks(&self) -> u64 {
        (0..self.section_length_bars)
            .map(|b| {
                self.tempo_map.numerator_at_bar(self.start_bar + b) as u64
                    * TICKS_PER_QUARTER_NOTE
            })
            .sum()
    }

    /// Map a section-relative tick position to pixel x within `clip_width`.
    fn tick_to_x(&self, tick: f64, clip_width: f32) -> f32 {
        let total = self.section_total_ticks() as f64;
        if total <= 0.0 {
            return 0.0;
        }
        (tick / total * clip_width as f64) as f32
    }

    /// Inverse of `tick_to_x`: pixel x to section-relative tick.
    fn x_to_tick(&self, x: f32, clip_width: f32) -> f64 {
        let total = self.section_total_ticks() as f64;
        if clip_width <= 0.0 {
            return 0.0;
        }
        x as f64 / clip_width as f64 * total
    }

    /// Convert an absolute sample position to a section-relative tick.
    fn sample_to_section_tick(&self, sample: u64) -> f64 {
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
    fn clip_tick_range(&self, clip: &MidiClipState) -> Option<(f64, f64)> {
        let clip_start_tick = self.sample_to_section_tick(clip.start_sample);
        let clip_end_tick = clip_start_tick + clip.duration_ticks as f64;
        let section_ticks = self.section_total_ticks() as f64;
        if clip_end_tick <= 0.0 || clip_start_tick >= section_ticks {
            return None;
        }
        Some((clip_start_tick.max(0.0), clip_end_tick.min(section_ticks)))
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
        if self.section_length_bars == 0 || clip_area.width <= 0.0 {
            return;
        }
        let mut tick_pos: u64 = 0;
        for bar_offset in 0..self.section_length_bars {
            let bar = self.start_bar + bar_offset;
            let num = self.tempo_map.numerator_at_bar(bar) as u64;
            let bar_ticks = num * TICKS_PER_QUARTER_NOTE;

            // Bar line
            let x = clip_area.x + self.tick_to_x(tick_pos as f64, clip_area.width);
            frame.stroke(
                &Path::line(
                    Point::new(x, clip_area.y + NOTE_GRID_PAD),
                    Point::new(x, clip_area.y + clip_area.height - NOTE_GRID_PAD),
                ),
                Stroke::default()
                    .with_width(1.5)
                    .with_color(Color::from_rgb(0.30, 0.30, 0.34)),
            );

            // Beat lines within this bar
            for beat in 1..num {
                let beat_tick = tick_pos + beat * TICKS_PER_QUARTER_NOTE;
                let bx = clip_area.x + self.tick_to_x(beat_tick as f64, clip_area.width);
                frame.stroke(
                    &Path::line(
                        Point::new(bx, clip_area.y + NOTE_GRID_PAD),
                        Point::new(bx, clip_area.y + clip_area.height - NOTE_GRID_PAD),
                    ),
                    Stroke::default()
                        .with_width(1.0)
                        .with_color(Color::from_rgb(0.18, 0.18, 0.20)),
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
            Stroke::default()
                .with_width(1.5)
                .with_color(Color::from_rgb(0.30, 0.30, 0.34)),
        );
    }

    fn draw_clip_outline(
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
        frame.stroke(
            &Path::rectangle(
                Point::new(rect.x, rect.y),
                Size::new(rect.width, rect.height),
            ),
            Stroke::default()
                .with_width(1.0)
                .with_color(Color::from_rgba(0.38, 0.58, 0.38, 0.55)),
        );
    }

    fn draw_notes(
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
            // Clamp to section bounds
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

    /// Determine which track row (if any) the given point falls in,
    /// respecting expanded/collapsed layout.
    fn hit_test_track(&self, pos: Point, bounds: Rectangle) -> Option<TrackId> {
        let tracks = self.sorted_tracks();
        for (idx, track) in tracks.iter().enumerate() {
            let row = self.track_row_rect(idx, bounds);
            if pos.y >= row.y && pos.y <= row.y + row.height {
                return Some(track.id);
            }
        }
        None
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

    fn hit_test_note_edit(&self, pos: Point, bounds: Rectangle) -> Option<Message> {
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
            let Some(pitch) = self.y_to_pitch(pos.y, clip_area) else {
                return None;
            };
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

fn snap_tick(tick: u64, snap: u64) -> u64 {
    if snap == 0 {
        return tick;
    }
    (tick / snap) * snap
}
