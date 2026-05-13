//! Canvas-rendered lane side panel — the left "header" column of every
//! lane on the Compose view.
//!
//! Per the redesign each lane has a structured side panel with three
//! elements stacked vertically:
//!   1. A small uppercase tag pill (HARMONY / ACCOMP / MELODY / RHYTHM)
//!      colored by the lane's role (lavender for section-level, warm
//!      amber for track-level).
//!   2. A primary title (the lane / track name).
//!   3. A secondary meta line (style, plugin name, etc).
//!
//! All three are painted directly into a `canvas::Frame` so the existing
//! Canvas-based lane renderers can adopt them without restructuring the
//! draw pipeline.
use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{Color, Point, Rectangle, Size};

use crate::theme;

/// Logical kind of a Compose lane. Drives the tag label / color so the
/// rendering helpers don't need a soup of bool flags.
#[derive(Debug, Clone, Copy)]
pub enum LaneKind {
    /// Chord lane / section harmony — lavender accent.
    Harmony,
    /// General instrument track providing accompaniment — warm amber.
    Accomp,
    /// Lead / solo instrument track — warm amber.
    Melody,
    /// Drum / percussion track — warm amber.
    Rhythm,
}

impl LaneKind {
    pub fn tag(self) -> &'static str {
        match self {
            LaneKind::Harmony => "HARMONY",
            LaneKind::Accomp => "ACCOMP",
            LaneKind::Melody => "MELODY",
            LaneKind::Rhythm => "RHYTHM",
        }
    }

    /// Strong accent color used for the tag text and the selection edge.
    pub fn color(self) -> Color {
        match self {
            LaneKind::Harmony => theme::ACCENT_SOFT,
            LaneKind::Accomp | LaneKind::Melody | LaneKind::Rhythm => theme::WARM,
        }
    }

    /// Soft tint used as the tag pill fill.
    pub fn dim(self) -> Color {
        match self {
            LaneKind::Harmony => theme::ACCENT_DIM,
            LaneKind::Accomp | LaneKind::Melody | LaneKind::Rhythm => Color {
                a: 0.14,
                ..theme::WARM
            },
        }
    }

    /// Outline color used on the tag pill border (matches WARM_LINE /
    /// ACCENT_LINE in the design tokens).
    pub fn line(self) -> Color {
        match self {
            LaneKind::Harmony => theme::ACCENT_LINE,
            LaneKind::Accomp | LaneKind::Melody | LaneKind::Rhythm => theme::WARM_LINE,
        }
    }
}

/// Vertical positions (relative to the side rect's top edge) of the three
/// content stack rows. Calibrated so a 64-px chord lane fits cleanly while
/// taller (160-px synth / 188-px drum) rows top-align with the same spacing.
const PILL_TOP_Y: f32 = 10.0;
const PILL_HEIGHT: f32 = 16.0;
const TITLE_BASELINE_Y: f32 = 30.0;
const META_BASELINE_Y: f32 = 49.0;

/// Approximate width per glyph at the tag's 9-px semibold weight. Iced's
/// canvas doesn't expose text metrics, so the pill width is computed from
/// a per-glyph estimate; over-allocating by a couple of pixels is fine
/// because the pill background is rounded and tinted.
const TAG_CHAR_W: f32 = 7.2;
const PILL_HPAD: f32 = 7.0;

/// Paint the lane's left side panel: BG fill, tag pill, title, meta, and
/// the right-edge separator. `selected` lightens the background and turns
/// the separator into the lane's accent color — so the active lane reads
/// clearly even when the title text doesn't change.
pub fn draw(
    frame: &mut Frame,
    rect: Rectangle,
    kind: LaneKind,
    title: &str,
    meta: Option<&str>,
    selected: bool,
) {
    // -- Background ---------------------------------------------------------
    let bg = if selected { theme::BG_3 } else { theme::BG_2 };
    frame.fill_rectangle(Point::new(rect.x, rect.y), Size::new(rect.width, rect.height), bg);

    // -- Tag pill ----------------------------------------------------------
    let tag = kind.tag();
    let pill_w = (tag.len() as f32 * TAG_CHAR_W + 2.0 * PILL_HPAD).max(48.0);
    let pill_x = rect.x + 12.0;
    let pill_y = rect.y + PILL_TOP_Y;
    let pill = Path::rounded_rectangle(
        Point::new(pill_x, pill_y),
        Size::new(pill_w, PILL_HEIGHT),
        theme::RADIUS_XS.into(),
    );
    frame.fill(&pill, kind.dim());
    frame.stroke(
        &pill,
        Stroke::default().with_width(1.0).with_color(kind.line()),
    );
    frame.fill_text(canvas::Text {
        content: tag.to_string(),
        position: Point::new(pill_x + PILL_HPAD, pill_y + 2.5),
        color: kind.color(),
        size: 9.0.into(),
        font: theme::UI_FONT_SEMIBOLD,
        ..canvas::Text::default()
    });

    // -- Title --------------------------------------------------------------
    frame.fill_text(canvas::Text {
        content: title.to_string(),
        position: Point::new(rect.x + 12.0, rect.y + TITLE_BASELINE_Y),
        color: theme::TEXT_1,
        size: 13.0.into(),
        font: theme::UI_FONT_MEDIUM,
        ..canvas::Text::default()
    });

    // -- Meta ---------------------------------------------------------------
    if let Some(meta) = meta {
        frame.fill_text(canvas::Text {
            content: meta.to_string(),
            position: Point::new(rect.x + 12.0, rect.y + META_BASELINE_Y),
            color: theme::TEXT_3,
            size: 10.5.into(),
            ..canvas::Text::default()
        });
    }

    // -- Right-edge separator ----------------------------------------------
    let sep_color = if selected { kind.color() } else { theme::LINE_2 };
    frame.fill_rectangle(
        Point::new(rect.x + rect.width - 1.0, rect.y),
        Size::new(1.0, rect.height),
        sep_color,
    );
}

/// Compact variant for collapsed track strips — single line, no tag pill,
/// optional accent color for the active lane title.
pub fn draw_compact(
    frame: &mut Frame,
    rect: Rectangle,
    title: &str,
    selected: bool,
    expanded: bool,
) {
    let bg = if expanded {
        Color::from_rgb(0.18, 0.22, 0.18)
    } else if selected {
        theme::BG_3
    } else {
        theme::BG_2
    };
    frame.fill_rectangle(Point::new(rect.x, rect.y), Size::new(rect.width, rect.height), bg);
    frame.fill_text(canvas::Text {
        content: title.to_string(),
        position: Point::new(rect.x + 12.0, rect.y + rect.height * 0.5 - 8.0),
        color: if expanded {
            theme::ACCENT_SOFT
        } else {
            theme::TEXT_1
        },
        size: 12.0.into(),
        font: theme::UI_FONT_MEDIUM,
        ..canvas::Text::default()
    });
    let sep_color = if selected || expanded {
        theme::ACCENT
    } else {
        theme::LINE_2
    };
    frame.fill_rectangle(
        Point::new(rect.x + rect.width - 1.0, rect.y),
        Size::new(1.0, rect.height),
        sep_color,
    );
}
