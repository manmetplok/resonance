//! Tiling ribbon lane row — a bar-grid visualisation of a section's drum
//! arrangement (design doc #170, surface #2).
//!
//! The ribbon sits under a bar-number header, sized to the section's
//! `length_bars`. Each arrangement entry paints a contiguous span tinted
//! by its pattern colour and labelled `Name ×N`. Four span *types* are
//! visually distinct (colour + shape + text, per the accessibility rule):
//!
//! - **Normal** — a pattern span, tinted by [`DrumPattern::color`], labelled
//!   `Name ×N`, with a left accent stripe.
//! - **Fill**   — the entry's last bar swapped for its fill pattern: a
//!   hatched warm span tagged `FILL`.
//! - **Gap**    — an under-filled tail: a hatched warm span with a dashed
//!   border tagged `GAP` (the drum lane goes silent there).
//! - **Overflow** — an over-filled cap drawn *past* the section end: a pink
//!   hatched `OVERFLOW (clipped)` badge (won't play or bounce).
//!
//! The concrete spans come from the [`#483`](crate::compose::resolve_arrangement)
//! resolver via [`ComposeState::resolve_arrangement_for`]; the ribbon only
//! *renders* them plus the gap/overflow surfaced by the coverage status. It
//! re-derives each span's owning entry index (the resolver flattens entries
//! into pattern spans) so clicking a span can select that entry — emitting
//! [`ComposeMessage::SelectArrangementEntry`], which drives the right-rail
//! Entry inspector.
//!
//! All colours + radii come from `theme.rs`; the live ribbon is a `Canvas`
//! per the view-performance rules, and the static legend below it is plain
//! Iced widgets.

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::widget::{column, container, row, text, Canvas, Space};
use iced::{alignment, mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use crate::compose::{
    ArrangementCoverage, ComposeMessage, EntryLength, PatternEntry, ResolvedArrangement,
    SectionDefinitionState,
};
use crate::message::Message;
use crate::theme;
use crate::Resonance;

use super::super::tracks::NAME_COLUMN_WIDTH;

/// Height of the bar-number header strip at the top of the ribbon.
const BAR_HEADER_H: f32 = 16.0;
/// Y offset where the span band begins.
const BAND_TOP: f32 = BAR_HEADER_H + 4.0;
/// Height of the span band itself.
const BAND_H: f32 = 30.0;
/// Total canvas height (header + band + bottom breathing room).
const RIBBON_HEIGHT: f32 = BAND_TOP + BAND_H + 6.0;
/// Fixed pixel width reserved for the overflow cap drawn past the section
/// end. The cap hangs off the right of the lane so the over-fill reads as
/// "beyond the boundary" rather than eating into the section grid.
const OVERFLOW_CAP_PX: f32 = 104.0;
/// Diagonal-hatch spacing for fill / gap / overflow spans.
const HATCH_SPACING: f32 = 7.0;

/// Warm amber as raw RGB — mirrors [`theme::WARM`]. Used for the fill / gap
/// span `color` field so a generic consumer (e.g. the legend) can tint
/// without re-deriving from the palette.
const WARM_RGB: [u8; 3] = [0xe8, 0xc4, 0x7b];
/// Soft pink as raw RGB — mirrors [`theme::BAD`]. Used for the overflow cap.
const BAD_RGB: [u8; 3] = [0xe8, 0x7b, 0x8b];

/// Which of the four ribbon span *types* a rendered span is. Drives its
/// colour, shape (hatch / dash), and text so the four read as distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RibbonSpanKind {
    /// A normal pattern span — tinted by the pattern colour, `Name ×N`.
    Normal,
    /// An entry's fill bar — hatched warm, `FILL`.
    Fill,
    /// A trailing under-fill — hatched warm dashed, `GAP`.
    Gap,
    /// An over-fill cap past the section end — pink hatched, `OVERFLOW`.
    Overflow,
}

/// One renderable ribbon span, in section-relative bar coordinates. For
/// [`RibbonSpanKind::Overflow`], `bar_end` extends past the section's
/// `length_bars` — the drawer clamps that overhang to [`OVERFLOW_CAP_PX`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RibbonSpan {
    /// First bar of the span (inclusive, 0-based within the section).
    pub bar_start: u32,
    /// One past the last bar of the span (exclusive).
    pub bar_end: u32,
    /// Span type — colour + shape + text vocabulary.
    pub kind: RibbonSpanKind,
    /// Tint. Meaningful for [`RibbonSpanKind::Normal`] (the pattern
    /// colour); the other kinds carry their semantic palette colour but the
    /// drawer keys off `kind` for the palette.
    pub color: [u8; 3],
    /// Label painted inside the span (`Name ×N`, `FILL`, `GAP`, `OVERFLOW`).
    pub label: String,
    /// Arrangement entry this span belongs to, if any. Fill spans inherit
    /// their entry; gap / overflow spans have no entry (`None`).
    pub entry_index: Option<usize>,
}

/// Build the ribbon's renderable spans from a section's arrangement.
///
/// Pure and deterministic given the lookups — unit-tested without a
/// [`ComposeState`]. It combines the already-clipped pattern/fill spans
/// from `resolved` with a trailing gap span (when the coverage under-fills)
/// or an overflow cap (when it over-fills), and re-derives each pattern
/// span's owning entry index so a click can select that entry.
///
/// - `pattern_len` returns a pattern's intrinsic bar length (for entry-range
///   attribution of `RepeatN` entries — mirrors the resolver).
/// - `pattern_name` / `pattern_color` resolve the pattern bank for labels
///   and tints.
pub fn build_ribbon_spans(
    entries: &[PatternEntry],
    length_bars: u32,
    resolved: &ResolvedArrangement,
    pattern_len: impl Fn(u64) -> u32,
    pattern_name: impl Fn(u64) -> String,
    pattern_color: impl Fn(u64) -> [u8; 3],
) -> Vec<RibbonSpan> {
    // Re-derive each entry's contiguous bar range (head-to-tail from bar 0)
    // exactly as the resolver lays them down, so a span's `bar_start` maps
    // back to the entry that produced it.
    let mut ranges: Vec<(usize, u32, u32)> = Vec::with_capacity(entries.len());
    let mut cursor: u32 = 0;
    for (i, e) in entries.iter().enumerate() {
        let bars = match e.length {
            EntryLength::RepeatN(n) => n.saturating_mul(pattern_len(e.pattern_id).max(1)),
            EntryLength::Bars(b) => b,
        };
        ranges.push((i, cursor, cursor.saturating_add(bars)));
        cursor = cursor.saturating_add(bars);
    }
    let entry_at = |bar: u32| -> Option<usize> {
        ranges
            .iter()
            .find(|(_, s, e)| bar >= *s && bar < *e)
            .map(|(i, _, _)| *i)
    };

    let mut out: Vec<RibbonSpan> = Vec::with_capacity(resolved.spans.len() + 1);

    for span in &resolved.spans {
        let idx = entry_at(span.bar_start);
        if span.is_fill {
            out.push(RibbonSpan {
                bar_start: span.bar_start,
                bar_end: span.bar_end,
                kind: RibbonSpanKind::Fill,
                color: WARM_RGB,
                label: "FILL".to_string(),
                entry_index: idx,
            });
        } else {
            let name = pattern_name(span.pattern_id);
            let label = match idx.and_then(|i| entries.get(i)) {
                Some(e) => match e.length {
                    // A 2-bar pattern repeated 3× reads "Name ×3".
                    EntryLength::RepeatN(n) => format!("{} \u{00d7}{}", name, n),
                    // Fixed-bar entries have no repeat count; the width shows
                    // the span length.
                    EntryLength::Bars(_) => name,
                },
                None => name,
            };
            out.push(RibbonSpan {
                bar_start: span.bar_start,
                bar_end: span.bar_end,
                kind: RibbonSpanKind::Normal,
                color: pattern_color(span.pattern_id),
                label,
                entry_index: idx,
            });
        }
    }

    match resolved.coverage {
        ArrangementCoverage::Gap { bars } if bars > 0 => {
            out.push(RibbonSpan {
                bar_start: length_bars.saturating_sub(bars),
                bar_end: length_bars,
                kind: RibbonSpanKind::Gap,
                color: WARM_RGB,
                label: "GAP".to_string(),
                entry_index: None,
            });
        }
        ArrangementCoverage::Overflow { bars } if bars > 0 => {
            out.push(RibbonSpan {
                bar_start: length_bars,
                bar_end: length_bars.saturating_add(bars),
                kind: RibbonSpanKind::Overflow,
                color: BAD_RGB,
                label: "OVERFLOW (clipped)".to_string(),
                entry_index: None,
            });
        }
        _ => {}
    }

    out
}

/// Read-only canvas rendering the tiling ribbon. Holds pre-computed spans
/// (a pure function of the arrangement) plus the geometry it needs so the
/// draw / hit-test math agrees.
pub struct RibbonCanvas {
    /// Renderable spans in section-relative bar coordinates.
    pub spans: Vec<RibbonSpan>,
    /// Section length in bars (>= 1). Sizes the bar grid.
    pub length_bars: u32,
    /// Currently selected entry — highlights its span(s).
    pub selected_entry_index: Option<usize>,
    /// Section name shown in the lane's side tag.
    pub section_name: String,
    /// Pixel width of the bar grid (workspace width minus the name column).
    /// Bar width is `content_width / length_bars`, independent of the
    /// canvas bounds so it aligns with the other lane canvases even when the
    /// canvas is widened to fit an overflow cap.
    pub content_width: f32,
}

impl RibbonCanvas {
    fn bar_px(&self) -> f32 {
        self.content_width / self.length_bars.max(1) as f32
    }

    /// On-screen X of a bar boundary (bar 0 == left edge of the grid).
    fn bar_x(&self, bar: u32) -> f32 {
        NAME_COLUMN_WIDTH + bar as f32 * self.bar_px()
    }

    /// Clamped [start_x, end_x) of a span in screen pixels; overflow caps
    /// are limited to [`OVERFLOW_CAP_PX`] so they don't run off forever.
    fn span_x_range(&self, span: &RibbonSpan) -> (f32, f32) {
        let sx = self.bar_x(span.bar_start);
        let mut ex = self.bar_x(span.bar_end);
        if span.kind == RibbonSpanKind::Overflow {
            ex = ex.min(sx + OVERFLOW_CAP_PX);
        }
        (sx, ex)
    }
}

impl canvas::Program<Message> for RibbonCanvas {
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

        // --- Side tag: "TILING" kicker + section name -------------------
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(NAME_COLUMN_WIDTH, bounds.height),
            theme::BG_1,
        );
        frame.fill_text(Text {
            content: "TILING".to_string(),
            position: Point::new(12.0, 10.0),
            color: theme::TEXT_3,
            size: 9.0.into(),
            font: theme::UI_FONT_SEMIBOLD,
            ..Text::default()
        });
        frame.fill_text(Text {
            content: self.section_name.clone(),
            position: Point::new(12.0, 24.0),
            color: theme::TEXT_1,
            size: 12.0.into(),
            ..Text::default()
        });

        // --- Content card background ------------------------------------
        let grid_x = NAME_COLUMN_WIDTH;
        frame.fill_rectangle(
            Point::new(grid_x, 0.0),
            Size::new((bounds.width - grid_x).max(0.0), bounds.height),
            theme::BG_2,
        );

        // --- Bar-number header + grid lines -----------------------------
        let bars = self.length_bars.max(1);
        for b in 0..=bars {
            let x = self.bar_x(b);
            frame.stroke(
                &Path::line(
                    Point::new(x, BAND_TOP - 2.0),
                    Point::new(x, BAND_TOP + BAND_H),
                ),
                Stroke::default().with_width(1.0).with_color(theme::LINE_2),
            );
            if b < bars {
                frame.fill_text(Text {
                    content: format!("{}", b + 1),
                    position: Point::new(x + 4.0, 2.0),
                    color: if b == 0 { theme::TEXT_3 } else { theme::TEXT_4 },
                    size: 9.0.into(),
                    font: theme::MONO_FONT,
                    ..Text::default()
                });
            }
        }

        // --- Spans ------------------------------------------------------
        for span in &self.spans {
            let (sx, ex) = self.span_x_range(span);
            let w = (ex - sx).max(0.0);
            if w <= 0.0 {
                continue;
            }
            let selected = span.entry_index.is_some()
                && span.entry_index == self.selected_entry_index;
            draw_span(&mut frame, sx, w, span, selected);
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
            // Ignore clicks in the side tag column and outside the span band.
            if pos.x < NAME_COLUMN_WIDTH || pos.y < BAND_TOP || pos.y > BAND_TOP + BAND_H {
                return None;
            }
            // Resolve which span was hit, mapping to its entry (gap /
            // overflow carry `None`, which clears the selection).
            for span in &self.spans {
                let (sx, ex) = self.span_x_range(span);
                if pos.x >= sx && pos.x < ex {
                    return Some(
                        canvas::Action::publish(Message::Compose(
                            ComposeMessage::SelectArrangementEntry(span.entry_index),
                        ))
                        .and_capture(),
                    );
                }
            }
            // Clicked empty band → clear the selection.
            return Some(
                canvas::Action::publish(Message::Compose(
                    ComposeMessage::SelectArrangementEntry(None),
                ))
                .and_capture(),
            );
        }
        None
    }
}

/// Paint a single span according to its kind. `sx` is the left edge, `w`
/// the (clamped) width in pixels.
fn draw_span(frame: &mut Frame, sx: f32, w: f32, span: &RibbonSpan, selected: bool) {
    let rect = Rectangle {
        x: sx + 1.0,
        y: BAND_TOP + 3.0,
        width: (w - 2.0).max(1.0),
        height: BAND_H - 6.0,
    };
    let path = Path::rounded_rectangle(
        Point::new(rect.x, rect.y),
        Size::new(rect.width, rect.height),
        theme::RADIUS_SM.into(),
    );

    match span.kind {
        RibbonSpanKind::Normal => {
            let c = u8_color(span.color);
            frame.fill(&path, Color { a: 0.22, ..c });
            // Left accent stripe.
            frame.fill_rectangle(
                Point::new(rect.x, rect.y),
                Size::new(2.0, rect.height),
                c,
            );
            let (bw, bc) = if selected {
                (2.0, c)
            } else {
                (1.0, Color { a: 0.55, ..c })
            };
            frame.stroke(&path, Stroke::default().with_width(bw).with_color(bc));
            span_label(frame, &rect, &span.label, theme::TEXT_1, 11.0);
        }
        RibbonSpanKind::Fill => {
            let warm = theme::WARM;
            frame.fill(&path, Color { a: 0.14, ..warm });
            hatch(frame, &rect, Color { a: 0.5, ..warm });
            let (bw, bc) = if selected { (2.0, warm) } else { (1.0, theme::WARM_LINE) };
            frame.stroke(&path, Stroke::default().with_width(bw).with_color(bc));
            span_label(frame, &rect, &span.label, warm, 9.5);
        }
        RibbonSpanKind::Gap => {
            let warm = theme::WARM;
            frame.fill(&path, Color { a: 0.08, ..warm });
            hatch(frame, &rect, Color { a: 0.4, ..warm });
            dashed_border(frame, &rect, warm);
            span_label(frame, &rect, &span.label, warm, 9.5);
        }
        RibbonSpanKind::Overflow => {
            let bad = theme::BAD;
            // Section-end boundary marker at the cap's left edge.
            frame.stroke(
                &Path::line(
                    Point::new(sx, BAND_TOP - 2.0),
                    Point::new(sx, BAND_TOP + BAND_H),
                ),
                Stroke::default().with_width(2.0).with_color(bad),
            );
            frame.fill(&path, Color { a: 0.18, ..bad });
            hatch(frame, &rect, Color { a: 0.55, ..bad });
            frame.stroke(&path, Stroke::default().with_width(1.5).with_color(bad));
            span_label(frame, &rect, &span.label, bad, 8.5);
        }
    }
}

/// Draw a left-aligned, vertically-centred label inside a span rect.
fn span_label(frame: &mut Frame, rect: &Rectangle, label: &str, color: Color, size: f32) {
    if rect.width < 16.0 {
        return;
    }
    frame.fill_text(Text {
        content: label.to_string(),
        position: Point::new(rect.x + 6.0, rect.y + rect.height / 2.0 - size / 2.0),
        color,
        size: size.into(),
        font: theme::UI_FONT_SEMIBOLD,
        ..Text::default()
    });
}

/// Diagonal (45°, up-right) hatch fill clipped to `rect`. Marks fill / gap /
/// overflow spans so they read as distinct from a flat normal span even in
/// monochrome.
fn hatch(frame: &mut Frame, rect: &Rectangle, color: Color) {
    let stroke = Stroke::default().with_width(1.0).with_color(color);
    let (rx, ry, rw, rh) = (rect.x, rect.y, rect.width, rect.height);
    // Lines of constant x + y = k. Clip each to the rect's x-range.
    let mut k = rx + ry;
    let k_max = rx + rw + ry + rh;
    while k <= k_max {
        let x_lo = rx.max(k - (ry + rh));
        let x_hi = (rx + rw).min(k - ry);
        if x_lo < x_hi {
            frame.stroke(
                &Path::line(
                    Point::new(x_lo, k - x_lo),
                    Point::new(x_hi, k - x_hi),
                ),
                stroke,
            );
        }
        k += HATCH_SPACING;
    }
}

/// Draw a dashed rectangle border (iced strokes have no dash pattern, so we
/// lay down short segments along each edge).
fn dashed_border(frame: &mut Frame, rect: &Rectangle, color: Color) {
    let stroke = Stroke::default().with_width(1.0).with_color(color);
    let dash = 4.0;
    let gap = 3.0;
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.width, rect.y + rect.height);
    // Horizontal edges.
    let mut x = x0;
    while x < x1 {
        let xe = (x + dash).min(x1);
        frame.stroke(&Path::line(Point::new(x, y0), Point::new(xe, y0)), stroke);
        frame.stroke(&Path::line(Point::new(x, y1), Point::new(xe, y1)), stroke);
        x += dash + gap;
    }
    // Vertical edges.
    let mut y = y0;
    while y < y1 {
        let ye = (y + dash).min(y1);
        frame.stroke(&Path::line(Point::new(x0, y), Point::new(x0, ye)), stroke);
        frame.stroke(&Path::line(Point::new(x1, y), Point::new(x1, ye)), stroke);
        y += dash + gap;
    }
}

fn u8_color(rgb: [u8; 3]) -> Color {
    Color::from_rgb(
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
    )
}

/// Build the full tiling ribbon lane row: the live ribbon canvas plus a
/// static legend explaining the four span types. `width` is the fixed
/// workspace width shared by every Compose lane.
pub fn ribbon<'a>(
    app: &'a Resonance,
    definition: &'a SectionDefinitionState,
    width: f32,
) -> Element<'a, Message> {
    let resolved = app.compose.resolve_arrangement_for(definition);
    let spans = build_ribbon_spans(
        &definition.arrangement,
        definition.length_bars,
        &resolved,
        |id| app.compose.find_pattern(id).map(|p| p.bar_span()).unwrap_or(1),
        |id| {
            app.compose
                .find_pattern(id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "?".to_string())
        },
        |id| {
            app.compose
                .find_pattern(id)
                .map(|p| p.color)
                .unwrap_or([0x5d, 0x62, 0x6d])
        },
    );

    let overflow = matches!(resolved.coverage, ArrangementCoverage::Overflow { bars } if bars > 0);
    let content_width = (width - NAME_COLUMN_WIDTH).max(0.0);
    let canvas_width = width + if overflow { OVERFLOW_CAP_PX } else { 0.0 };

    let prog = RibbonCanvas {
        spans,
        length_bars: definition.length_bars.max(1),
        selected_entry_index: app.compose.drumroll.selected_entry_index,
        section_name: definition.name.clone(),
        content_width,
    };

    let canvas = Canvas::new(prog)
        .width(Length::Fixed(canvas_width))
        .height(Length::Fixed(RIBBON_HEIGHT));

    column![canvas, legend_row()].spacing(2).into()
}

/// Static legend beneath the ribbon: one swatch + label per span type.
/// Left-padded to align under the ribbon's content area.
fn legend_row<'a>() -> Element<'a, Message> {
    let items = row![
        legend_item(theme::ACCENT, 0.22, false, "Pattern"),
        legend_item(theme::WARM, 0.16, false, "Fill (last bar)"),
        legend_item(theme::WARM, 0.06, true, "Gap (uncovered)"),
        legend_item(theme::BAD, 0.18, false, "Overflow (clipped)"),
    ]
    .spacing(14)
    .align_y(alignment::Vertical::Center);

    row![
        Space::new().width(Length::Fixed(NAME_COLUMN_WIDTH)),
        items,
    ]
    .into()
}

/// One legend entry — a 12×12 swatch tinted like the span type, plus text.
/// `dashed` renders the swatch with a hairline border to hint the gap's
/// dashed treatment (iced container borders can't dash, so colour + word
/// carry the distinction).
fn legend_item<'a>(
    base: Color,
    fill_alpha: f32,
    dashed: bool,
    label: &'static str,
) -> Element<'a, Message> {
    let swatch = container(Space::new().width(Length::Fixed(12.0)).height(Length::Fixed(12.0)))
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(Color {
                a: fill_alpha,
                ..base
            })),
            border: iced::Border {
                color: if dashed {
                    Color { a: 0.7, ..base }
                } else {
                    base
                },
                width: 1.0,
                radius: theme::RADIUS_XS.into(),
            },
            ..Default::default()
        });
    row![swatch, text(label).size(10).color(theme::TEXT_2)]
        .spacing(5)
        .align_y(alignment::Vertical::Center)
        .into()
}
