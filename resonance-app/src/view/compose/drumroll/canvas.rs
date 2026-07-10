//! Compose drum lane — grouped drum-pad canvas.
//!
//! Each project-scoped [`DrumGroup`] gets one collapsible block on the
//! canvas: a group header row (color dot, name, polymeter tag, density
//! readout) plus one row per articulation pad. Pads inside a group render
//! against the group's own grid + cycle so polymeter and polyrhythm read
//! visually — a 7/16 hat group shows its cycle restart as a dashed marker
//! that doesn't line up with the 4/4 bar.
//!
//! ## Multi-bar arrangement rendering (todo #487)
//!
//! When a section has a chained arrangement, each bar's cells come from
//! the pattern covering that bar (resolved via the #483 resolver). Callers
//! build a [`BarSpanView`] slice from the resolved spans — each entry
//! carries the pattern's accent color and its concrete groups — and pass
//! it via [`ComposeDrumCanvas::bar_spans`]. The canvas then:
//!
//! - draws a faint pattern-color tint over each span's bar range,
//! - draws a 1-px separator at each span boundary,
//! - reads each bar's cell on/off state from that bar's span's groups
//!   (matched by position index so a 5-group Pattern A and a 3-group
//!   Pattern B render the correct cells for each row, leaving excess rows
//!   empty in the shorter-pattern bars).

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

const BEATS_PER_BAR: u32 = 4;

const GROUP_HEAD_HEIGHT: f32 = 22.0;
const PAD_ROW_HEIGHT: f32 = 18.0;
const PAD_LABEL_WIDTH: f32 = 76.0;
const STEP_HEADER_HEIGHT: f32 = 16.0;
const LANE_PAD_TOP: f32 = 8.0;
const LANE_PAD_BOTTOM: f32 = 8.0;
const GROUP_GAP: f32 = 6.0;

/// Tint alpha applied over the bar-range of a non-fill span.
const SPAN_TINT_ALPHA: f32 = 0.05;
/// Tint alpha applied over the bar-range of a fill span (slightly stronger).
const SPAN_FILL_TINT_ALPHA: f32 = 0.13;
/// Alpha of the 1-px separator line drawn at span boundaries.
const SPAN_SEPARATOR_ALPHA: f32 = 0.22;

/// Pre-resolved view of one contiguous bar-range sharing the same pattern.
/// Built by the view layer (see [`super::mod.rs`]) from
/// [`crate::compose::ComposeState::resolve_arrangement_for`]; the canvas
/// stays data-only with no reference to `ComposeState`.
pub struct BarSpanView<'a> {
    /// First bar of this span (0-based, section-relative, inclusive).
    pub bar_start: u32,
    /// Exclusive end bar.
    pub bar_end: u32,
    /// Pattern accent color (RGB) — used for the faint tint overlay.
    pub pattern_color: [u8; 3],
    /// Groups from the pattern covering this span. Cell on/off state is
    /// read from these; matched to the primary-pattern group by position
    /// index.
    pub pattern_groups: &'a [DrumGroup],
    /// Whether this span is an entry's fill bar (slightly stronger tint).
    pub is_fill: bool,
}

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
///
/// `groups` sets the lane structure (row count, height). `bar_spans`
/// provides the per-bar resolved cell data: each bar looks up the span
/// covering it, then reads cells from that span's `pattern_groups` at the
/// same group-position index as the primary group. `section_bars` drives
/// the step-area subdivision so the canvas fills the full section width.
pub struct ComposeDrumCanvas<'a> {
    pub track: &'a TrackState,
    /// Groups from the primary (first-bar) pattern. Determines lane height
    /// and the row labels/colors shown for the whole section.
    pub groups: &'a [DrumGroup],
    pub selected_group_id: Option<u64>,
    pub track_selected: bool,
    /// Resolved per-span data for the section's arrangement. Each entry
    /// covers one contiguous bar range; gap bars use the primary pattern.
    /// Single-entry (no-chain) arrangements produce exactly one span.
    pub bar_spans: Vec<BarSpanView<'a>>,
    /// Total bars in the section. Drives the bar-width subdivision within
    /// the step area.
    pub section_bars: u32,
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

        let step_area_x = card_rect.x + PAD_LABEL_WIDTH + 8.0;
        let step_area_width = (card_rect.width - PAD_LABEL_WIDTH - 16.0).max(0.0);
        if step_area_width <= 0.0 {
            return vec![frame.into_geometry()];
        }

        let section_bars = self.section_bars.max(1);
        let bar_w = step_area_width / section_bars as f32;

        // Step header — bar number labels, one per bar.
        for bar_idx in 0..section_bars {
            let bar_x = step_area_x + bar_idx as f32 * bar_w;
            // Center the label within the bar.
            let label_x = bar_x + bar_w / 2.0 - 4.0;
            frame.fill_text(canvas::Text {
                content: format!("{}", bar_idx + 1),
                position: Point::new(label_x, card_rect.y + 6.0),
                color: if bar_idx == 0 { theme::TEXT_2 } else { theme::TEXT_4 },
                size: 10.0.into(),
                ..canvas::Text::default()
            });
        }

        // Each group occupies a vertical block of its own.
        let mut y = card_rect.y + STEP_HEADER_HEIGHT + 4.0;
        for (gi, group) in self.groups.iter().enumerate() {
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

            // Pattern-span tint overlays: faint colored background over
            // each span's bar range. Only visible when more than one span
            // exists (uniform tint is still drawn, but indistinguishable
            // from the card background for single-pattern arrangements).
            for span in &self.bar_spans {
                let span_x = step_area_x + span.bar_start as f32 * bar_w;
                let span_w = (span.bar_end - span.bar_start) as f32 * bar_w;
                let span_color = u8_color(span.pattern_color);
                let tint_a = if span.is_fill {
                    SPAN_FILL_TINT_ALPHA
                } else {
                    SPAN_TINT_ALPHA
                };
                let tint = Color {
                    a: tint_a,
                    ..span_color
                };
                frame.fill_rectangle(
                    Point::new(span_x, y),
                    Size::new(span_w, block_height),
                    tint,
                );
            }

            // Span separator lines — 1-px vertical stripe at each span
            // boundary (skipping the very first span since there is no
            // "previous" span to separate from).
            for (si, span) in self.bar_spans.iter().enumerate() {
                if si > 0 {
                    let sep_x = step_area_x + span.bar_start as f32 * bar_w;
                    let sep = Color {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: SPAN_SEPARATOR_ALPHA,
                    };
                    frame.fill_rectangle(
                        Point::new(sep_x - 0.5, y),
                        Size::new(1.0, block_height),
                        sep,
                    );
                }
            }

            // Group header row.
            draw_group_head(
                &mut frame,
                group,
                color,
                focused,
                card_rect,
                step_area_x,
                step_area_width,
                y,
            );

            // Pad rows.
            let mut pad_y = y + GROUP_HEAD_HEIGHT;

            for (pi, pad) in group.pads.iter().enumerate() {
                // Pad name.
                frame.fill_text(canvas::Text {
                    content: pad.name.clone(),
                    position: Point::new(card_rect.x + 26.0, pad_y + 2.0),
                    color: if focused {
                        theme::TEXT_2
                    } else {
                        theme::TEXT_3
                    },
                    size: 11.0.into(),
                    ..canvas::Text::default()
                });

                // Share %.
                let share = group.weight_share(pi);
                frame.fill_text(canvas::Text {
                    content: format!("{}%", share),
                    position: Point::new(
                        card_rect.x + 6.0 + PAD_LABEL_WIDTH - 28.0,
                        pad_y + 4.0,
                    ),
                    color: theme::TEXT_4,
                    size: 9.0.into(),
                    font: theme::MONO_FONT,
                    ..canvas::Text::default()
                });

                // Cells — bar-by-bar, reading each bar's cell data from the
                // span covering it (resolved_group at same index gi).
                for bar_idx in 0..section_bars {
                    let bar_x = step_area_x + bar_idx as f32 * bar_w;

                    // Find the span covering this bar.
                    let span_opt = self
                        .bar_spans
                        .iter()
                        .find(|s| bar_idx >= s.bar_start && bar_idx < s.bar_end);

                    // Resolved group: the group at index gi in the bar's
                    // pattern (positional match). Falls back to the primary
                    // group so layout stays consistent when a shorter-group
                    // pattern lacks this row.
                    let resolved_group = span_opt
                        .and_then(|s| s.pattern_groups.get(gi))
                        .unwrap_or(group);
                    let resolved_pad = resolved_group.pads.get(pi);

                    let cells_in_bar = (group.grid as u32 * BEATS_PER_BAR) as usize;
                    let cells_in_bar = cells_in_bar.max(1);
                    let cell_w = bar_w / cells_in_bar as f32;
                    let cycle = resolved_group.pattern_len().max(1);

                    for s in 0..cells_in_bar {
                        let cx = bar_x + s as f32 * cell_w;
                        let global_step = bar_idx as usize * cells_in_bar + s;
                        let pattern_step =
                            (global_step + group.phase as usize) % cycle;

                        let is_beat_start = (s % group.grid as usize) == 0;
                        let bg = if is_beat_start {
                            theme::LINE_2
                        } else {
                            theme::BG_1
                        };
                        let rect_x = cx + 1.0;
                        let rect_y = pad_y + 2.0;
                        let rect_w = (cell_w - 2.0).max(1.0);
                        let rect_h = PAD_ROW_HEIGHT - 4.0;

                        frame.fill_rectangle(
                            Point::new(rect_x, rect_y),
                            Size::new(rect_w, rect_h),
                            bg,
                        );

                        let on = resolved_pad
                            .and_then(|p| p.pattern.get(pattern_step))
                            .copied()
                            .unwrap_or(0)
                            > 0;
                        if on {
                            let alpha =
                                0.55 + (pad.weight as f32 / 250.0).clamp(0.0, 0.4);
                            let fill = Color {
                                a: alpha,
                                ..color
                            };
                            frame.fill_rectangle(
                                Point::new(rect_x, rect_y),
                                Size::new(rect_w, rect_h),
                                fill,
                            );
                        }

                        // Cycle-restart dashed marker — only on the first
                        // pad row, only when the cycle wraps (pattern_step
                        // rolled back to 0). global_step == 0 is excluded so
                        // we don't draw a spurious marker at the lane start.
                        if pi == 0 && global_step > 0 && pattern_step == 0 {
                            draw_cycle_restart_marker(
                                &mut frame,
                                cx,
                                y,
                                pad_y,
                                color,
                            );
                        }
                    }
                }

                pad_y += PAD_ROW_HEIGHT;
            }

            y += block_height + GROUP_GAP;
        }

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        _state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            let pos = cursor.position_in(bounds)?;

            // Side panel: open the drum lane in the inspector.
            if pos.x < NAME_COLUMN_WIDTH {
                return Some(
                    canvas::Action::publish(Message::Compose(ComposeMessage::SelectLane(
                        crate::compose::SelectedLane::Drums(self.track.id),
                    )))
                    .and_capture(),
                );
            }

            // Step area geometry — mirrors `draw` so cell hit-tests
            // resolve to the same on-screen rectangles.
            let card_x = NAME_COLUMN_WIDTH + 8.0;
            let card_width = (bounds.width - NAME_COLUMN_WIDTH - 10.0).max(0.0);
            let step_area_x = card_x + PAD_LABEL_WIDTH + 8.0;
            let step_area_width = (card_width - PAD_LABEL_WIDTH - 16.0).max(0.0);

            let section_bars = self.section_bars.max(1);
            let bar_w = step_area_width / section_bars as f32;

            let card_top = 2.0 + STEP_HEADER_HEIGHT + 4.0;
            let mut y = card_top;
            for (gi, group) in self.groups.iter().enumerate() {
                let block_height =
                    GROUP_HEAD_HEIGHT + PAD_ROW_HEIGHT * group.pads.len() as f32;
                if pos.y < y || pos.y >= y + block_height {
                    y += block_height + GROUP_GAP;
                    continue;
                }

                // Pad-row band: figure out which pad row was hit, then —
                // if the click landed inside the step area — convert x to
                // a bar + step index and emit TogglePadStep. Outside the
                // step area (or in the header) fall back to SelectGroup.
                let pad_band_top = y + GROUP_HEAD_HEIGHT;
                if pos.y >= pad_band_top
                    && pos.x >= step_area_x
                    && step_area_width > 0.0
                    && bar_w > 0.0
                {
                    let pad_index = ((pos.y - pad_band_top) / PAD_ROW_HEIGHT) as usize;
                    if pad_index < group.pads.len() {
                        // Which bar was clicked?
                        let bar_idx = ((pos.x - step_area_x) / bar_w)
                            .max(0.0)
                            .floor() as usize;
                        let bar_idx = bar_idx.min(section_bars as usize - 1);
                        let bar_x = step_area_x + bar_idx as f32 * bar_w;

                        // Find the span covering this bar to get the
                        // resolved group for correct cycle length.
                        let span_opt = self.bar_spans.iter().find(|s| {
                            bar_idx as u32 >= s.bar_start && (bar_idx as u32) < s.bar_end
                        });
                        let resolved_group = span_opt
                            .and_then(|s| s.pattern_groups.get(gi))
                            .unwrap_or(group);
                        let cycle = resolved_group.pattern_len().max(1);

                        let cells_in_bar =
                            (group.grid as u32 * BEATS_PER_BAR) as usize;
                        let cells_in_bar = cells_in_bar.max(1);
                        let cell_w = bar_w / cells_in_bar as f32;
                        let step_in_bar =
                            ((pos.x - bar_x) / cell_w).max(0.0).floor() as usize;
                        let step_in_bar = step_in_bar.min(cells_in_bar - 1);

                        let global_step = bar_idx * cells_in_bar + step_in_bar;
                        let pattern_step =
                            (global_step + group.phase as usize) % cycle;

                        if pattern_step < cycle {
                            return Some(
                                canvas::Action::publish(Message::Compose(
                                    ComposeMessage::DrumGroups(
                                        DrumGroupsMessage::TogglePadStep {
                                            group_id: group.id,
                                            pad_index,
                                            step: pattern_step,
                                        },
                                    ),
                                ))
                                .and_capture(),
                            );
                        }
                    }
                }

                // Header row click or click past the last step — focus
                // the group instead.
                return Some(
                    canvas::Action::publish(Message::Compose(ComposeMessage::DrumGroups(
                        DrumGroupsMessage::SelectGroup { group_id: group.id },
                    )))
                    .and_capture(),
                );
            }
        }
        None
    }
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

/// Draw the cycle-restart dashed marker (a vertical dashed line spanning
/// from the group header through the current pad row) at canvas x = `cx`.
fn draw_cycle_restart_marker(
    frame: &mut Frame,
    cx: f32,
    group_y: f32,
    pad_y: f32,
    color: Color,
) {
    let stroke = Stroke::default().with_width(1.0).with_color(color);
    let top = pad_y - GROUP_HEAD_HEIGHT + 6.0;
    let bottom = pad_y + PAD_ROW_HEIGHT - 2.0;
    let mut yy = top;
    while yy < bottom {
        let segment_end = (yy + 3.0).min(bottom);
        frame.stroke(
            &Path::line(Point::new(cx, yy), Point::new(cx, segment_end)),
            stroke,
        );
        yy += 5.0;
    }
    // Tiny "→1" marker label above the dashed line.
    frame.fill_text(canvas::Text {
        content: "\u{2192}1".to_string(),
        position: Point::new(cx + 2.0, group_y + 4.0),
        color,
        size: 8.0.into(),
        font: theme::MONO_FONT,
        ..canvas::Text::default()
    });
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
