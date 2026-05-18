//! Compose drum lane — grouped drum-pad canvas.
//!
//! Each project-scoped [`DrumGroup`] gets one collapsible block on the
//! canvas: a group header row (color dot, name, polymeter tag, density
//! readout) plus one row per articulation pad. Pads inside a group render
//! against the group's own grid + cycle so polymeter and polyrhythm read
//! visually — a 7/16 hat group shows its cycle restart as a dashed marker
//! that doesn't line up with the 4/4 bar.

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use resonance_audio::types::TrackType;

use crate::compose::drumroll::{grid_label, DrumGroup};
use crate::compose::messages::DrumGroupsMessage;
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::state::{InstrumentType, TrackState};
use crate::theme;

use super::super::lane_side::{self, LaneKind};
use super::super::tracks::NAME_COLUMN_WIDTH;

/// Beat count rendered in the lane. One bar = 4 beats. The lane stays
/// visually compact even when the section is longer — generation happens
/// off-canvas via the right-rail Generate button.
const BEATS_IN_LANE: u32 = 4;
const BEATS_PER_BAR: u32 = 4;

const GROUP_HEAD_HEIGHT: f32 = 22.0;
const PAD_ROW_HEIGHT: f32 = 18.0;
const PAD_LABEL_WIDTH: f32 = 76.0;
const STEP_HEADER_HEIGHT: f32 = 16.0;
const LANE_PAD_TOP: f32 = 8.0;
const LANE_PAD_BOTTOM: f32 = 8.0;
const GROUP_GAP: f32 = 6.0;

/// Per-row pad height plus the group header. The total lane height grows
/// with the number of pads — callers ask for it via [`drum_lane_height`].
pub fn drum_lane_height(groups: &[DrumGroup]) -> f32 {
    let pads_total: usize = groups.iter().map(|g| g.pads.len()).sum();
    LANE_PAD_TOP
        + STEP_HEADER_HEIGHT
        + GROUP_HEAD_HEIGHT * groups.len() as f32
        + PAD_ROW_HEIGHT * pads_total as f32
        + GROUP_GAP * groups.len().saturating_sub(1) as f32
        + LANE_PAD_BOTTOM
}

/// Read-only canvas rendering the grouped drum lane for one drum track.
pub struct ComposeDrumCanvas<'a> {
    pub track: &'a TrackState,
    pub groups: &'a [DrumGroup],
    pub selected_group_id: Option<u64>,
    pub track_selected: bool,
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
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BG_1);

        // Lane side panel — RHYTHM tag, track name, meta line.
        let side_rect = Rectangle {
            x: 0.0,
            y: 0.0,
            width: NAME_COLUMN_WIDTH,
            height: bounds.height,
        };
        let meta = self
            .track
            .plugins
            .first()
            .map(|p| p.plugin_name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| {
                format!("Resonance Drums · {} groups", self.groups.len())
            });
        lane_side::draw(
            &mut frame,
            side_rect,
            LaneKind::Rhythm,
            &self.track.name,
            Some(&meta),
            self.track_selected,
        );

        let card_rect = Rectangle {
            x: NAME_COLUMN_WIDTH + 8.0,
            y: 2.0,
            width: (bounds.width - NAME_COLUMN_WIDTH - 10.0).max(0.0),
            height: bounds.height - 4.0,
        };
        frame.fill_rectangle(
            Point::new(card_rect.x, card_rect.y),
            Size::new(card_rect.width, card_rect.height),
            theme::BG_2,
        );

        // Step header — 4 beat numbers, the cell area sits under it.
        let step_area_x = card_rect.x + PAD_LABEL_WIDTH + 8.0;
        let step_area_width = (card_rect.width - PAD_LABEL_WIDTH - 16.0).max(0.0);
        if step_area_width <= 0.0 {
            return vec![frame.into_geometry()];
        }
        let beat_w = step_area_width / BEATS_IN_LANE as f32;
        for b in 0..BEATS_IN_LANE {
            let x = step_area_x + b as f32 * beat_w + beat_w / 2.0 - 4.0;
            frame.fill_text(canvas::Text {
                content: format!("{}", b + 1),
                position: Point::new(x, card_rect.y + 6.0),
                color: if b == 0 { theme::TEXT_2 } else { theme::TEXT_4 },
                size: 10.0.into(),
                ..canvas::Text::default()
            });
        }

        // Each group occupies a vertical block of its own.
        let mut y = card_rect.y + STEP_HEADER_HEIGHT + 4.0;
        for group in self.groups.iter() {
            let focused = self.track_selected && Some(group.id) == self.selected_group_id;
            let color = u8_color(group.color);
            let block_height = GROUP_HEAD_HEIGHT + PAD_ROW_HEIGHT * group.pads.len() as f32;

            // Block background tint when focused.
            if focused {
                let tint = Color {
                    a: 0.06,
                    ..color
                };
                frame.fill_rectangle(
                    Point::new(card_rect.x + 4.0, y),
                    Size::new(card_rect.width - 8.0, block_height),
                    tint,
                );
                // Left edge accent stripe.
                frame.fill_rectangle(
                    Point::new(card_rect.x + 4.0, y),
                    Size::new(2.0, block_height),
                    color,
                );
            }

            // Group header row.
            draw_group_head(&mut frame, group, color, focused, card_rect, step_area_x, step_area_width, y);

            // Pad rows.
            let mut pad_y = y + GROUP_HEAD_HEIGHT;
            let cells = cells_for_group(group);
            let cell_count = cells.len().max(1);
            let cell_w = step_area_width / cell_count as f32;

            for (pi, pad) in group.pads.iter().enumerate() {
                draw_pad_row(
                    &mut frame,
                    pad,
                    group,
                    color,
                    focused,
                    card_rect,
                    PAD_LABEL_WIDTH,
                    step_area_x,
                    cell_w,
                    &cells,
                    pad_y,
                    pi == 0,
                );
                pad_y += PAD_ROW_HEIGHT;
            }

            y += block_height + GROUP_GAP;
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

            // Side panel: open the drum lane in the inspector.
            if pos.x < NAME_COLUMN_WIDTH {
                return (
                    canvas::event::Status::Captured,
                    Some(Message::Compose(ComposeMessage::SelectLane(
                        crate::compose::SelectedLane::Drums(self.track.id),
                    ))),
                );
            }

            // Step area geometry — mirrors `draw` so cell hit-tests
            // resolve to the same on-screen rectangles.
            let card_x = NAME_COLUMN_WIDTH + 8.0;
            let card_width = (bounds.width - NAME_COLUMN_WIDTH - 10.0).max(0.0);
            let step_area_x = card_x + PAD_LABEL_WIDTH + 8.0;
            let step_area_width = (card_width - PAD_LABEL_WIDTH - 16.0).max(0.0);

            let card_top = 2.0 + STEP_HEADER_HEIGHT + 4.0;
            let mut y = card_top;
            for group in self.groups.iter() {
                let block_height = GROUP_HEAD_HEIGHT + PAD_ROW_HEIGHT * group.pads.len() as f32;
                if pos.y < y || pos.y >= y + block_height {
                    y += block_height + GROUP_GAP;
                    continue;
                }

                // Pad-row band: figure out which pad row was hit, then —
                // if the click landed inside the step area — convert x to
                // a step index and emit a TogglePadStep. Outside the step
                // area (or in the header) we fall back to just focusing
                // the group.
                let pad_band_top = y + GROUP_HEAD_HEIGHT;
                if pos.y >= pad_band_top && pos.x >= step_area_x && step_area_width > 0.0 {
                    let pad_index = ((pos.y - pad_band_top) / PAD_ROW_HEIGHT) as usize;
                    if pad_index < group.pads.len() {
                        let cells = cells_for_group(group);
                        let cell_count = cells.len().max(1);
                        let cell_w = step_area_width / cell_count as f32;
                        let step = ((pos.x - step_area_x) / cell_w) as usize;
                        if step < cells.len() {
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::Compose(ComposeMessage::DrumGroups(
                                    DrumGroupsMessage::TogglePadStep {
                                        group_id: group.id,
                                        pad_index,
                                        step,
                                    },
                                ))),
                            );
                        }
                    }
                }

                // Header row click or click past the last step — focus
                // the group instead.
                return (
                    canvas::event::Status::Captured,
                    Some(Message::Compose(ComposeMessage::DrumGroups(
                        DrumGroupsMessage::SelectGroup { group_id: group.id },
                    ))),
                );
            }
        }
        (canvas::event::Status::Ignored, None)
    }
}

/// Pre-computed cell positions inside a group's pattern. The cell list is
/// `cycle` items long (or wrapped to fill `BEATS_IN_LANE * grid` when the
/// cycle is shorter so the visualisation still spans the lane).
fn cells_for_group(group: &DrumGroup) -> Vec<CellRef> {
    let cycle = group.pattern_len().max(1);
    let visible = (BEATS_IN_LANE * group.grid as u32) as usize;
    let span = visible.max(cycle);
    (0..span)
        .map(|i| {
            let pattern_idx = (i + group.phase as usize) % cycle;
            let is_cycle_start = i > 0 && pattern_idx == 0;
            CellRef {
                i,
                pattern_idx,
                is_cycle_start,
            }
        })
        .collect()
}

struct CellRef {
    i: usize,
    pattern_idx: usize,
    is_cycle_start: bool,
}

#[allow(clippy::too_many_arguments)]
fn draw_group_head(
    frame: &mut Frame,
    group: &DrumGroup,
    color: Color,
    focused: bool,
    card: Rectangle,
    _step_area_x: f32,
    _step_area_width: f32,
    y: f32,
) {
    // Color dot.
    let dot_y = y + GROUP_HEAD_HEIGHT / 2.0 - 3.0;
    frame.fill_rectangle(
        Point::new(card.x + 14.0, dot_y),
        Size::new(6.0, 6.0),
        color,
    );

    // Name.
    frame.fill_text(canvas::Text {
        content: group.name.to_ascii_uppercase(),
        position: Point::new(card.x + 26.0, y + 5.0),
        color: if focused { theme::TEXT_1 } else { theme::TEXT_2 },
        size: 10.0.into(),
        font: theme::UI_FONT_SEMIBOLD,
        ..canvas::Text::default()
    });

    // Polymeter tag — visible whenever the group's grid or cycle differs
    // from the section base (4/16, 16 steps).
    let base_grid = 4u8;
    let base_cycle = 16u32;
    let is_odd = group.is_off_grid(base_grid, base_cycle);
    if is_odd {
        let tag = format!(
            "{}/{} \u{00b7} {}",
            group.cycle,
            group.grid as u32 * BEATS_PER_BAR,
            grid_label(group.grid)
        );
        // Rough width estimate so the tag tints sit on a darker pill.
        let tag_w = (tag.len() as f32 * 5.5).max(40.0);
        let tag_x = card.x + 26.0 + group.name.len() as f32 * 6.5 + 10.0;
        let tag_y = y + 4.0;
        let pill = Rectangle {
            x: tag_x,
            y: tag_y,
            width: tag_w,
            height: 12.0,
        };
        let pill_bg = Color {
            a: 0.12,
            ..color
        };
        frame.fill_rectangle(
            Point::new(pill.x, pill.y),
            Size::new(pill.width, pill.height),
            pill_bg,
        );
        frame.fill_text(canvas::Text {
            content: tag,
            position: Point::new(pill.x + 4.0, pill.y + 1.0),
            color,
            size: 8.5.into(),
            font: theme::MONO_FONT,
            ..canvas::Text::default()
        });
    }

    // Right-aligned meta — "{pad_count} pads · density {pct}%".
    let pad_word = if group.pads.len() == 1 { "pad" } else { "arts" };
    let meta = format!(
        "{} {} \u{00b7} density {}%",
        group.pads.len(),
        pad_word,
        (group.density * 100.0).round() as i32
    );
    frame.fill_text(canvas::Text {
        content: meta,
        position: Point::new(card.x + card.width - 160.0, y + 5.0),
        color: theme::TEXT_4,
        size: 9.5.into(),
        font: theme::MONO_FONT,
        ..canvas::Text::default()
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_pad_row(
    frame: &mut Frame,
    pad: &crate::compose::drumroll::DrumGroupPad,
    group: &DrumGroup,
    color: Color,
    focused: bool,
    card: Rectangle,
    label_width: f32,
    step_area_x: f32,
    cell_w: f32,
    cells: &[CellRef],
    y: f32,
    is_first_pad: bool,
) {
    // Pad name.
    frame.fill_text(canvas::Text {
        content: pad.name.clone(),
        position: Point::new(card.x + 26.0, y + 2.0),
        color: if focused {
            theme::TEXT_2
        } else {
            theme::TEXT_3
        },
        size: 11.0.into(),
        ..canvas::Text::default()
    });

    // Share %.
    let share = group.weight_share(
        group
            .pads
            .iter()
            .position(|p| p.note == pad.note && p.name == pad.name)
            .unwrap_or(0),
    );
    frame.fill_text(canvas::Text {
        content: format!("{}%", share),
        position: Point::new(card.x + 6.0 + label_width - 28.0, y + 4.0),
        color: theme::TEXT_4,
        size: 9.0.into(),
        font: theme::MONO_FONT,
        ..canvas::Text::default()
    });

    // Cells.
    for cell in cells {
        let cx = step_area_x + cell.i as f32 * cell_w;
        if cx >= step_area_x + cell_w * cells.len() as f32 {
            break;
        }
        let is_beat_start = (cell.i % group.grid as usize) == 0;
        let bg = if is_beat_start { theme::LINE_2 } else { theme::BG_1 };
        let rect = Rectangle {
            x: cx + 1.0,
            y: y + 2.0,
            width: (cell_w - 2.0).max(1.0),
            height: PAD_ROW_HEIGHT - 4.0,
        };
        frame.fill_rectangle(
            Point::new(rect.x, rect.y),
            Size::new(rect.width, rect.height),
            bg,
        );
        let on = pad.pattern.get(cell.pattern_idx).copied().unwrap_or(0) > 0;
        if on {
            let alpha = 0.55 + (pad.weight as f32 / 250.0).clamp(0.0, 0.4);
            let fill = Color { a: alpha, ..color };
            frame.fill_rectangle(
                Point::new(rect.x, rect.y),
                Size::new(rect.width, rect.height),
                fill,
            );
        }
    }

    // Cycle-restart dashed markers — drawn only on the first pad row so
    // they don't repeat per pad. The marker spans from just above the
    // group header down through this pad row.
    if is_first_pad {
        for cell in cells.iter().filter(|c| c.is_cycle_start) {
            let cx = step_area_x + cell.i as f32 * cell_w;
            let stroke = Stroke::default().with_width(1.0).with_color(color);
            // Dashed manually — iced 0.13 stroke styles don't expose dash
            // patterns, so draw a stack of short segments.
            let top = y - GROUP_HEAD_HEIGHT + 6.0;
            let bottom = y + PAD_ROW_HEIGHT - 2.0;
            let mut yy = top;
            while yy < bottom {
                let segment_end = (yy + 3.0).min(bottom);
                frame.stroke(
                    &Path::line(Point::new(cx, yy), Point::new(cx, segment_end)),
                    stroke,
                );
                yy += 5.0;
            }
            // Tiny label "→ N" so the user can see the cycle step.
            frame.fill_text(canvas::Text {
                content: format!("\u{2192} {}", cell.pattern_idx + 1),
                position: Point::new(cx + 2.0, top - 2.0),
                color,
                size: 8.0.into(),
                font: theme::MONO_FONT,
                ..canvas::Text::default()
            });
        }
    }
}

fn u8_color(rgb: [u8; 3]) -> Color {
    Color::from_rgb(
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
    )
}

/// Sorted list of all drum-type tracks in the registry.
pub fn sorted_drum_tracks(tracks: &[TrackState]) -> Vec<&TrackState> {
    let mut v: Vec<&TrackState> = tracks
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
