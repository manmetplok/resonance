//! Performance-mode next-chords look-ahead lane (epic #11, todo #309;
//! design #151, arch doc #152).
//!
//! Renders the lane shown beneath the centre stage: the NEXT 2–3 upcoming
//! chords (from the chord-derivation core, #304) as cards, each carrying
//!
//! - the chord symbol (root bright, quality suffix + slash/bass muted, via
//!   the same components [`Chord`]'s `Display` joins);
//! - a **mini chord-box** — an Iced [`canvas`] driven by the shared
//!   [`crate::chord_box`] layout module (#305) at a smaller scale than the
//!   centre-stage hero (#308), root dot = ACCENT lavender, other dots a
//!   neutral dark fill, `O`/`X` markers, a nut bar or a `"{n}fr"` boxed
//!   start-fret label); and
//! - a **bars-until** label (`"in 1 bar"` / `"in N bars"`).
//!
//! The IMMEDIATE next chord ([`Emphasis::First`]) is larger / brighter with
//! an [`theme::ACCENT_LINE`] outline; later cards dim progressively. When
//! there are no upcoming chords — the end of a progression or an empty
//! project — the lane shows the `"no upcoming chords"` empty state.
//!
//! Per the view-performance rules the mini diagrams are **Canvas with a
//! cached static layer** ([`canvas::Cache`]) that repaints only when the
//! chord / tuning / capo changes (a fingerprint guard), and the whole lane
//! sits behind an `iced::widget::lazy` cache in the parent shell keyed on
//! [`fingerprint`] — so the status bar's per-frame clock never rebuilds it
//! and the lane only repaints on a chord / section / bar change. All colours
//! come from [`crate::theme`].

use std::cell::Cell;
use std::hash::{Hash, Hasher};

use iced::widget::text::{Alignment as TextAlignment, LineHeight};
use iced::widget::{canvas, column, container, row, text, Space};
use iced::{alignment, Element, Length, Point};

use resonance_music_theory::{Chord, Tuning, WINDOW_FRETS};

use crate::chord_box::{self, Dims, Marker, Nut};
use crate::message::Message;
use crate::theme;
use crate::view::performance::center_stage::voicing_for;

// -- Mini chord-box geometry (px) --------------------------------------------
//
// A deliberately smaller scale than the centre-stage hero diagram
// (`center_stage`): the lane previews several chords side by side, so each
// box trades the hero's note-name dots + string labels for a compact shape
// that still reads the fingering at a glance.

/// Horizontal spacing between adjacent strings.
const STRING_SPACING: f32 = 11.0;
/// Vertical spacing between adjacent fret lines.
const FRET_SPACING: f32 = 15.0;
/// Number of fret cells drawn — the full voicing window (see [`WINDOW_FRETS`]).
const FRET_COUNT: u8 = WINDOW_FRETS;
/// Finger-dot radius.
const DOT_R: f32 = 4.0;
/// Vertical band above the nut that holds the `O`/`X` marker row.
const HEADER_H: f32 = 12.0;
/// Horizontal lead-in/out either side of the board (room to centre it and to
/// print the boxed start-fret label).
const SIDE_PAD: f32 = 16.0;
/// Padding above the marker row.
const TOP_PAD: f32 = 4.0;
/// Padding below the board.
const BOTTOM_PAD: f32 = 4.0;
/// Stroke width of the (open-position) nut bar.
const NUT_WIDTH: f32 = 3.0;
/// Stroke width of ordinary fret + string lines.
const LINE_WIDTH: f32 = 1.0;
/// Radius of the `O`/`X` markers above the nut.
const MARKER_R: f32 = 3.5;

// -- Lane layout -------------------------------------------------------------

/// Gap between adjacent cards (the `›` arrow sits in this gap).
const CARD_GAP: f32 = 22.0;

// -- Card model --------------------------------------------------------------

/// How prominently a card is drawn. The immediate-next chord is emphasised;
/// later previews dim progressively (design #151).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Emphasis {
    /// The immediate next chord — larger, brightest, ACCENT_LINE outline.
    First,
    /// The second upcoming chord — normal weight.
    Mid,
    /// Any further preview — dimmed.
    Later,
}

/// One upcoming-chord card: the chord, the instrument/tuning + capo its mini
/// diagram is drawn for, the bars-until distance, and its emphasis tier.
///
/// (`Tuning` is a `&'static` reference table that does not derive `Debug`, so
/// this struct does not either.)
#[derive(Clone)]
pub struct NextCard {
    /// The upcoming chord (root / quality / slash bass).
    pub chord: Chord,
    /// Instrument tuning the mini diagram is voiced for.
    pub tuning: &'static Tuning,
    /// Capo position in frets (`0` = none).
    pub capo: u8,
    /// Whole bar-lines from the playhead's current bar to this chord.
    pub bars_until: u32,
    /// Emphasis tier (immediate next vs later previews).
    pub emphasis: Emphasis,
}

/// Emphasis tier for the `index`-th upcoming card (0 = immediate next).
pub fn emphasis_for(index: usize) -> Emphasis {
    match index {
        0 => Emphasis::First,
        1 => Emphasis::Mid,
        _ => Emphasis::Later,
    }
}

/// Whole bar-lines from `current_bar` to a chord slot starting at
/// `slot_start_bar`. Clamped at 0, so a slot at or before the current bar
/// (e.g. a loop wrap-around) reads as "this bar" rather than underflowing.
pub fn bars_until(current_bar: u32, slot_start_bar: u32) -> u32 {
    slot_start_bar.saturating_sub(current_bar)
}

/// The bars-until label for a card, per design #151: `"in 1 bar"` /
/// `"in N bars"`, with a sub-bar change (`0`) reading `"this bar"`.
pub fn bars_until_label(bars: u32) -> String {
    match bars {
        0 => "this bar".to_string(),
        1 => "in 1 bar".to_string(),
        n => format!("in {n} bars"),
    }
}

/// Stable fingerprint of the lane's inputs for the parent `iced::widget::lazy`
/// cache. Folds in each card's chord, tuning, capo, bars-until, and emphasis
/// so the lane rebuilds exactly when a chord / section / bar change alters it
/// — never per frame.
pub fn fingerprint(cards: &[NextCard]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    cards.len().hash(&mut h);
    for c in cards {
        c.chord.hash(&mut h);
        c.tuning.name.hash(&mut h);
        c.capo.hash(&mut h);
        c.bars_until.hash(&mut h);
        c.emphasis.hash(&mut h);
    }
    h.finish()
}

/// Pixel size `(width, height)` of one mini chord-box canvas for `tuning`,
/// derived from the fixed mini geometry. Wider for instruments with more
/// strings; height depends only on the fret window.
pub fn mini_size(tuning: &Tuning) -> (f32, f32) {
    let n = tuning.string_count() as f32;
    let board_w = (n - 1.0).max(0.0) * STRING_SPACING;
    let width = board_w + 2.0 * SIDE_PAD;
    let board_h = FRET_COUNT as f32 * FRET_SPACING;
    let height = TOP_PAD + HEADER_H + board_h + BOTTOM_PAD;
    (width, height)
}

// -- Lane content ------------------------------------------------------------

/// Build the look-ahead lane's inner content: a row of up to three chord
/// cards (symbol + mini chord-box + bars-until) separated by `›` arrows, the
/// immediate-next emphasised — or the empty/end state when `cards` is empty.
/// Returns owned (`'static`) content so it can live behind the parent shell's
/// `iced::widget::lazy` cache.
pub fn lane_content(cards: Vec<NextCard>) -> Element<'static, Message> {
    if cards.is_empty() {
        return empty_state();
    }

    let mut lane = row![]
        .spacing(CARD_GAP)
        .align_y(alignment::Vertical::Center);
    for (i, card) in cards.into_iter().enumerate() {
        if i > 0 {
            lane = lane.push(arrow());
        }
        lane = lane.push(card_view(card));
    }
    lane.into()
}

/// The empty / end-of-progression state: an em-dash with a `"no upcoming
/// chords"` caption (also the genuine empty-project fallback).
fn empty_state() -> Element<'static, Message> {
    container(
        column![
            text("\u{2014}")
                .size(40)
                .font(theme::SERIF_ITALIC_FONT)
                .color(theme::TEXT_4)
                .line_height(LineHeight::Relative(1.0)),
            Space::new().height(6),
            text("no upcoming chords")
                .size(11)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_3),
        ]
        .align_x(alignment::Horizontal::Center),
    )
    .padding([14, 22])
    .into()
}

/// One upcoming-chord card.
fn card_view(card: NextCard) -> Element<'static, Message> {
    let NextCard {
        chord,
        tuning,
        capo,
        bars_until,
        emphasis,
    } = card;

    let inner = row![
        card_symbol(chord, emphasis),
        mini_diagram(chord, tuning, capo),
        text(bars_until_label(bars_until))
            .size(11)
            .font(theme::MONO_FONT)
            .color(when_color(emphasis))
            .line_height(LineHeight::Relative(1.0)),
    ]
    .spacing(16)
    .align_y(alignment::Vertical::Center);

    container(inner)
        .padding([12, 18])
        .style(card_style(emphasis))
        .into()
}

/// The chord symbol for a card — root bright, quality suffix + slash/bass
/// muted, scaled and tinted by the card's emphasis. Built from the same
/// components `Chord::to_string` joins, so the symbol reads identically to
/// the centre-stage hero (root · suffix · "/bass").
fn card_symbol(chord: Chord, emphasis: Emphasis) -> Element<'static, Message> {
    let (root_size, sub_size, root_c, suffix_c, bass_c) = match emphasis {
        Emphasis::First => (56.0, 28.0, theme::TEXT_1, theme::ACCENT_SOFT, theme::TEXT_3),
        Emphasis::Mid => (40.0, 20.0, theme::TEXT_2, theme::TEXT_3, theme::TEXT_3),
        Emphasis::Later => (40.0, 20.0, theme::TEXT_3, theme::TEXT_4, theme::TEXT_4),
    };

    let mut sym = row![text(chord.root.to_string())
        .size(root_size)
        .font(theme::SERIF_ITALIC_FONT)
        .color(root_c)
        .line_height(LineHeight::Relative(0.9))]
    .spacing(1)
    .align_y(alignment::Vertical::Bottom);

    let suffix = chord.quality.suffix();
    if !suffix.is_empty() {
        sym = sym.push(
            text(suffix.to_string())
                .size(sub_size)
                .font(theme::SERIF_ITALIC_FONT)
                .color(suffix_c)
                .line_height(LineHeight::Relative(0.9)),
        );
    }
    if let Some(bass) = chord.bass {
        sym = sym.push(
            text(format!("/{bass}"))
                .size(sub_size)
                .font(theme::SERIF_ITALIC_FONT)
                .color(bass_c)
                .line_height(LineHeight::Relative(0.9)),
        );
    }
    sym.into()
}

/// The `›` separator drawn between adjacent cards.
fn arrow() -> Element<'static, Message> {
    text("\u{203a}")
        .size(20)
        .font(theme::UI_FONT)
        .color(theme::TEXT_4)
        .line_height(LineHeight::Relative(1.0))
        .into()
}

/// The mini chord-box canvas for `chord` on `tuning` + `capo`.
fn mini_diagram(chord: Chord, tuning: &'static Tuning, capo: u8) -> Element<'static, Message> {
    let voicing = voicing_for(chord, tuning, capo);
    let (w, h) = mini_size(tuning);
    canvas(MiniDiagram {
        tuning,
        frets: voicing.frets,
        start_fret: voicing.start_fret,
        chord,
        fingerprint: mini_fingerprint(chord, tuning, capo),
    })
    .width(Length::Fixed(w))
    .height(Length::Fixed(h))
    .into()
}

/// Fingerprint of one mini diagram's appearance (chord / tuning / capo), used
/// by its Canvas layer cache so it repaints only when those change.
fn mini_fingerprint(chord: Chord, tuning: &Tuning, capo: u8) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    chord.hash(&mut h);
    tuning.name.hash(&mut h);
    capo.hash(&mut h);
    h.finish()
}

// -- Card / when styling -----------------------------------------------------

/// The bars-until label colour for an emphasis tier.
fn when_color(emphasis: Emphasis) -> iced::Color {
    match emphasis {
        Emphasis::First => theme::ACCENT_SOFT,
        Emphasis::Mid => theme::TEXT_3,
        Emphasis::Later => theme::TEXT_4,
    }
}

/// The card container style: the immediate-next gets an ACCENT_LINE outline
/// over a faint ACCENT_DIM wash; later cards are chromeless.
fn card_style(emphasis: Emphasis) -> impl Fn(&iced::Theme) -> container::Style {
    let (bg, border) = match emphasis {
        Emphasis::First => (Some(theme::ACCENT_DIM), theme::ACCENT_LINE),
        _ => (None, iced::Color::TRANSPARENT),
    };
    move |_theme| container::Style {
        background: bg.map(iced::Background::Color),
        border: iced::Border {
            color: border,
            width: 1.0,
            radius: theme::RADIUS_LG.into(),
        },
        ..Default::default()
    }
}

// -- Mini chord-box Canvas ---------------------------------------------------

/// Canvas program for one mini fingering diagram. Owns the resolved voicing +
/// chord so its geometry is a pure function of [`Self::fingerprint`].
struct MiniDiagram {
    tuning: &'static Tuning,
    frets: Vec<Option<u8>>,
    start_fret: u8,
    chord: Chord,
    fingerprint: u64,
}

/// Per-widget Canvas state: the cached static layer plus the fingerprint it
/// was drawn for. When the live fingerprint drifts we clear the cache and
/// repaint once; otherwise redraws reuse the stored geometry.
#[derive(Default)]
struct DiagramState {
    cache: canvas::Cache,
    drawn: Cell<Option<u64>>,
}

impl<M> canvas::Program<M> for MiniDiagram {
    type State = DiagramState;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        if state.drawn.get() != Some(self.fingerprint) {
            state.cache.clear();
            state.drawn.set(Some(self.fingerprint));
        }
        let geometry = state
            .cache
            .draw(renderer, bounds.size(), |frame| self.render(frame));
        vec![geometry]
    }
}

impl MiniDiagram {
    fn render(&self, frame: &mut canvas::Frame) {
        use canvas::{Path, Stroke};

        let n = self.tuning.string_count();
        let board_w = (n.saturating_sub(1)) as f32 * STRING_SPACING;
        let dims = Dims {
            cell_w: frame.width(),
            chord_name_h: 0.0,
            header_h: HEADER_H,
            fret_spacing: FRET_SPACING,
            fret_count: FRET_COUNT,
            string_spacing: STRING_SPACING,
            dot_r: DOT_R,
        };
        let layout = chord_box::layout(
            &dims,
            (0.0, TOP_PAD),
            self.tuning,
            &self.frets,
            self.start_fret,
            &self.chord,
        );

        // Fret lines — a heavy nut bar in open position, an ordinary line in a
        // boxed window (where the start-fret label takes over).
        for (i, &fy) in layout.fret_ys.iter().enumerate() {
            let is_nut_bar = i == 0 && matches!(layout.nut, Nut::Open);
            let (width, color) = if is_nut_bar {
                (NUT_WIDTH, theme::TEXT_2)
            } else {
                (LINE_WIDTH, theme::TEXT_4)
            };
            frame.stroke(
                &Path::line(
                    Point::new(layout.board_x, fy),
                    Point::new(layout.board_x + board_w, fy),
                ),
                Stroke::default().with_width(width).with_color(color),
            );
        }

        // String lines.
        let board_top = layout.nut_y;
        let board_bottom = layout.nut_y + layout.board_h;
        for s in &layout.strings {
            frame.stroke(
                &Path::line(
                    Point::new(s.x, board_top),
                    Point::new(s.x, board_bottom),
                ),
                Stroke::default()
                    .with_width(LINE_WIDTH)
                    .with_color(theme::TEXT_4),
            );
        }

        // Boxed start-fret label ("5fr"), to the left of the board.
        if let Nut::StartFret(fret) = layout.nut {
            frame.fill_text(canvas::Text {
                content: format!("{fret}fr"),
                position: Point::new(layout.board_x - 6.0, layout.nut_y + FRET_SPACING * 0.5),
                color: theme::TEXT_3,
                size: 9.0.into(),
                font: theme::MONO_FONT,
                align_x: TextAlignment::Right,
                align_y: alignment::Vertical::Center,
                ..canvas::Text::default()
            });
        }

        // Open (`O`) / mute (`X`) markers above the nut. Drawn as shapes so
        // they render identically regardless of font coverage.
        let marker_y = layout.nut_y - HEADER_H * 0.5;
        for s in &layout.strings {
            match s.marker {
                Some(Marker::Open) => {
                    frame.stroke(
                        &Path::circle(Point::new(s.x, marker_y), MARKER_R),
                        Stroke::default().with_width(1.0).with_color(theme::TEXT_2),
                    );
                }
                Some(Marker::Mute) => {
                    let r = MARKER_R;
                    frame.stroke(
                        &Path::line(
                            Point::new(s.x - r, marker_y - r),
                            Point::new(s.x + r, marker_y + r),
                        ),
                        Stroke::default().with_width(1.0).with_color(theme::TEXT_3),
                    );
                    frame.stroke(
                        &Path::line(
                            Point::new(s.x - r, marker_y + r),
                            Point::new(s.x + r, marker_y - r),
                        ),
                        Stroke::default().with_width(1.0).with_color(theme::TEXT_3),
                    );
                }
                None => {}
            }
        }

        // Finger dots — root/bass dot in ACCENT lavender, others a neutral
        // dark fill with a thin outline. The note name is dropped at this
        // scale (the centre-stage hero carries it); the shape alone reads the
        // fingering at a glance.
        for dot in &layout.dots {
            let center = Point::new(dot.x, dot.y);
            if dot.is_root {
                frame.fill(&Path::circle(center, dot.r), theme::ACCENT);
            } else {
                frame.fill(&Path::circle(center, dot.r), theme::BG_3);
                frame.stroke(
                    &Path::circle(center, dot.r),
                    Stroke::default().with_width(1.0).with_color(theme::LINE),
                );
            }
        }
    }
}
