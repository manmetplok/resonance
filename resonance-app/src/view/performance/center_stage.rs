//! Performance-mode centre stage hero (epic #11, todo #308; design #151,
//! arch doc #152).
//!
//! Renders the three-column hero shown when a chord sits under the
//! playhead:
//!
//! - **(a)** the CURRENT chord symbol, huge and bold via [`Chord`]'s
//!   `Display` (`Chord::to_string`), with the root bright and the quality
//!   suffix + slash/bass in muted tints, plus a chord-tone chip row;
//! - **(b)** the FINGERING DIAGRAM, an Iced [`canvas`] driven by the shared
//!   [`crate::chord_box`] layout module (root dot = ACCENT lavender, other
//!   dots a neutral dark fill, `O`/`X` markers, a nut bar or an `"{n}fr"`
//!   boxed start-fret label), built from
//!   [`resonance_music_theory::fretboard_voicing`] /
//!   [`resonance_music_theory::fretboard_voicing_from`] for the selected
//!   tuning + capo and the current chord — honoring `chord.bass` so slash /
//!   inverted voicings draw a bass-consistent shape;
//! - **(c)** the beat-ring / count-in column, **left to todo G (#310)** — a
//!   sized placeholder keeps the hero balanced until that lands.
//!
//! Per the view-performance rules the diagram is a **Canvas with a cached
//! static layer** ([`canvas::Cache`]) that repaints **only** when the chord /
//! tuning / capo changes (a fingerprint guard), so a take never churns it
//! per frame. The whole hero also sits behind an `iced::widget::lazy` cache
//! in the parent shell keyed on the same fingerprint. All colours come from
//! [`crate::theme`].

use std::cell::Cell;
use std::hash::{Hash, Hasher};

use iced::widget::text::{Alignment as TextAlignment, LineHeight};
use iced::widget::{canvas, column, container, row, text, Space};
use iced::{alignment, Element, Length, Point};

use resonance_music_theory::{
    fretboard_voicing, fretboard_voicing_from, Chord, FretboardVoicing, Tuning, WINDOW_FRETS,
};

use crate::chord_box::{self, Dims, Marker, Nut};
use crate::message::Message;
use crate::theme;

// -- Hero type sizes ---------------------------------------------------------

/// Size of the root letter — the unmistakable, read-from-across-the-room
/// display glyph (design #151 calls for ~230px; trimmed slightly so the
/// three-column hero fits comfortably).
const ROOT_SIZE: f32 = 184.0;
/// Size of the quality suffix and slash/bass — smaller and muted so the
/// root dominates.
const QUALITY_SIZE: f32 = 86.0;

// -- Fingering-diagram geometry (px) -----------------------------------------

/// Horizontal spacing between adjacent strings.
const STRING_SPACING: f32 = 30.0;
/// Vertical spacing between adjacent fret lines.
const FRET_SPACING: f32 = 52.0;
/// Number of fret cells drawn — the full voicing window (see
/// [`WINDOW_FRETS`]).
const FRET_COUNT: u8 = WINDOW_FRETS;
/// Finger-dot radius.
const DOT_R: f32 = 12.0;
/// Vertical band above the nut that holds the `O`/`X` marker row.
const HEADER_H: f32 = 30.0;
/// Horizontal lead-in/out either side of the board (room to centre it and
/// to print the boxed start-fret label).
const SIDE_PAD: f32 = 36.0;
/// Vertical band below the board that holds the string labels.
const LABEL_H: f32 = 24.0;
/// Padding above the marker row.
const TOP_PAD: f32 = 8.0;
/// Stroke width of the (open-position) nut bar.
const NUT_WIDTH: f32 = 4.0;
/// Stroke width of ordinary fret + string lines.
const LINE_WIDTH: f32 = 1.5;
/// Radius of the `O`/`X` markers above the nut.
const MARKER_R: f32 = 5.0;

// -- Hero layout -------------------------------------------------------------

/// Reserved width for the beat-ring / count-in column (todo G / #310).
const RING_COL_W: f32 = 200.0;
/// Gap between the three hero columns.
const COLUMN_GAP: f32 = 56.0;

/// Build the centre-stage hero for `chord` on the selected `tuning` + `capo`.
///
/// Returns owned (`'static`) content so it can live behind the parent
/// shell's `iced::widget::lazy` cache. The fingering diagram is a Canvas
/// whose static layer is cached and repainted only when [`hero_fingerprint`]
/// changes.
pub fn hero(chord: Chord, tuning: &'static Tuning, capo: u8) -> Element<'static, Message> {
    let fingerprint = hero_fingerprint(chord, tuning, capo);
    let voicing = voicing_for(chord, tuning, capo);

    // (a) Huge chord symbol — root bright, quality + slash/bass muted. Built
    // from the same components `Chord::to_string` joins, so the readout reads
    // identically (root · quality suffix · "/bass") while each part takes its
    // own tint.
    let mut symbol = row![text(chord.root.to_string())
        .size(ROOT_SIZE)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1)
        .line_height(LineHeight::Relative(0.9)),]
    .spacing(2)
    .align_y(alignment::Vertical::Bottom);
    let suffix = chord.quality.suffix();
    if !suffix.is_empty() {
        symbol = symbol.push(
            text(suffix.to_string())
                .size(QUALITY_SIZE)
                .font(theme::SERIF_ITALIC_FONT)
                .color(theme::TEXT_2)
                .line_height(LineHeight::Relative(0.9)),
        );
    }
    if let Some(bass) = chord.bass {
        symbol = symbol.push(
            text(format!("/{bass}"))
                .size(QUALITY_SIZE)
                .font(theme::SERIF_ITALIC_FONT)
                .color(theme::TEXT_3)
                .line_height(LineHeight::Relative(0.9)),
        );
    }

    let chord_block = column![symbol, chord_tone_chips(chord)]
        .spacing(26)
        .align_x(alignment::Horizontal::Center);

    // (b) Fingering diagram (Canvas).
    let (dw, dh) = diagram_size(tuning);
    let diagram = canvas(FingeringDiagram {
        tuning,
        frets: voicing.frets,
        start_fret: voicing.start_fret,
        chord,
        fingerprint,
    })
    .width(Length::Fixed(dw))
    .height(Length::Fixed(dh));

    // (c) Beat-ring / count-in column — reserved for todo G (#310). A sized,
    // empty placeholder so the three-column hero balances once the ring
    // lands; nothing is drawn here yet.
    let ring_placeholder = Space::new()
        .width(Length::Fixed(RING_COL_W))
        .height(Length::Fixed(dh));

    row![chord_block, diagram, ring_placeholder]
        .spacing(COLUMN_GAP)
        .align_y(alignment::Vertical::Center)
        .into()
}

/// The chord-tone chip row beneath the symbol: one rounded chip per distinct
/// chord tone, the root tinted with the lavender accent. A slash bass that
/// is not itself a chord tone is appended as a neutral chip.
fn chord_tone_chips(chord: Chord) -> Element<'static, Message> {
    let mut chips = row![].spacing(8).align_y(alignment::Vertical::Center);
    let root_pc = chord.root.to_semitone();
    let mut seen = [false; 12];
    for pc in chord.pitch_classes() {
        let s = pc.to_semitone() as usize;
        if seen[s] {
            continue;
        }
        seen[s] = true;
        chips = chips.push(tone_chip(pc.as_str(), pc.to_semitone() == root_pc));
    }
    if let Some(bass) = chord.bass {
        let s = bass.to_semitone() as usize;
        if !seen[s] {
            chips = chips.push(tone_chip(bass.as_str(), false));
        }
    }
    chips.into()
}

/// One chord-tone chip. `is_root` paints it with the lavender accent.
fn tone_chip(label: &'static str, is_root: bool) -> Element<'static, Message> {
    let (bg, border, fg) = if is_root {
        (theme::ACCENT_DIM, theme::ACCENT_LINE, theme::ACCENT_SOFT)
    } else {
        (theme::BG_2, theme::LINE_2, theme::TEXT_2)
    };
    container(
        text(label.to_string())
            .size(15)
            .font(theme::MONO_FONT)
            .color(fg)
            .line_height(LineHeight::Relative(1.0)),
    )
    .padding([6, 12])
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(bg)),
        border: iced::Border {
            color: border,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    })
    .into()
}

/// Pixel size `(width, height)` of the fingering-diagram canvas for a
/// tuning, derived from the fixed geometry constants. Wider for instruments
/// with more strings.
pub fn diagram_size(tuning: &Tuning) -> (f32, f32) {
    let n = tuning.string_count() as f32;
    let board_w = (n - 1.0).max(0.0) * STRING_SPACING;
    let width = board_w + 2.0 * SIDE_PAD;
    let board_h = FRET_COUNT as f32 * FRET_SPACING;
    let height = TOP_PAD + HEADER_H + board_h + LABEL_H;
    (width, height)
}

// -- Voicing helpers (pure, headless-testable) -------------------------------

/// Playable voicing for `chord` on `tuning`, honoring `capo`.
///
/// With no capo this is the open-position search
/// ([`fretboard_voicing`]); with a capo at fret `c` the open strings are no
/// longer reachable below the capo, so the search floors at fret `c`
/// ([`fretboard_voicing_from`]) — the doc-#152 "capo = offset applied
/// app-side before voicing" convention. Either way `chord.bass` is honored
/// (strings below the bass are muted), so slash / inverted chords get a
/// bass-consistent shape.
pub fn voicing_for(chord: Chord, tuning: &Tuning, capo: u8) -> FretboardVoicing {
    if capo == 0 {
        fretboard_voicing(&chord, tuning)
    } else {
        fretboard_voicing_from(&chord, tuning, capo)
    }
}

/// Pitch class (0..12) of the lowest sounding string of `voicing` on
/// `tuning`, or `None` if every string is muted. For a bass-consistent
/// slash voicing this equals the chord's bass note.
pub fn lowest_sounding_pc(voicing: &FretboardVoicing, tuning: &Tuning) -> Option<u8> {
    voicing
        .frets
        .iter()
        .enumerate()
        .find_map(|(s, f)| f.map(|fret| (tuning.open[s] + fret) % 12))
}

/// Fingerprint of the inputs that determine the hero's appearance. The
/// parent shell keys its `iced::widget::lazy` cache on this (plus a state
/// tag) and the Canvas clears its layer cache when it changes — so the hero
/// repaints only on a chord / tuning / capo change, never per frame.
pub fn hero_fingerprint(chord: Chord, tuning: &Tuning, capo: u8) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    chord.hash(&mut h);
    tuning.name.hash(&mut h);
    capo.hash(&mut h);
    h.finish()
}

// -- Fingering-diagram Canvas ------------------------------------------------

/// Canvas program for the chord fingering diagram. Owns the resolved voicing
/// and chord so its geometry is a pure function of [`Self::fingerprint`].
struct FingeringDiagram {
    tuning: &'static Tuning,
    frets: Vec<Option<u8>>,
    start_fret: u8,
    chord: Chord,
    fingerprint: u64,
}

/// Per-widget Canvas state: the cached static layer plus the fingerprint it
/// was drawn for. When the live fingerprint drifts we clear the cache and
/// repaint once — otherwise redraws (the status-bar clock, hover, resize)
/// reuse the stored geometry.
#[derive(Default)]
struct DiagramState {
    cache: canvas::Cache,
    drawn: Cell<Option<u64>>,
}

impl<M> canvas::Program<M> for FingeringDiagram {
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

impl FingeringDiagram {
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

        // Fret lines — the first is a heavy nut bar in open position, an
        // ordinary line in a boxed window (where the start-fret label takes
        // over the "where am I" job).
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
                position: Point::new(layout.board_x - 12.0, layout.nut_y + FRET_SPACING * 0.5),
                color: theme::TEXT_2,
                size: 15.0.into(),
                font: theme::MONO_FONT,
                align_x: TextAlignment::Right,
                align_y: alignment::Vertical::Center,
                ..canvas::Text::default()
            });
        }

        // Open (`O`) / mute (`X`) markers above the nut. Drawn as shapes, not
        // glyphs, so they render identically regardless of font coverage.
        let marker_y = layout.nut_y - HEADER_H * 0.5;
        for s in &layout.strings {
            match s.marker {
                Some(Marker::Open) => {
                    frame.stroke(
                        &Path::circle(Point::new(s.x, marker_y), MARKER_R),
                        Stroke::default()
                            .with_width(1.5)
                            .with_color(theme::TEXT_2),
                    );
                }
                Some(Marker::Mute) => {
                    let r = MARKER_R;
                    frame.stroke(
                        &Path::line(
                            Point::new(s.x - r, marker_y - r),
                            Point::new(s.x + r, marker_y + r),
                        ),
                        Stroke::default()
                            .with_width(1.5)
                            .with_color(theme::TEXT_3),
                    );
                    frame.stroke(
                        &Path::line(
                            Point::new(s.x - r, marker_y + r),
                            Point::new(s.x + r, marker_y - r),
                        ),
                        Stroke::default()
                            .with_width(1.5)
                            .with_color(theme::TEXT_3),
                    );
                }
                None => {}
            }
        }

        // Finger dots — root/bass dot in ACCENT lavender, others a neutral
        // dark fill with a thin outline, the sounding note name centred in.
        for dot in &layout.dots {
            let center = Point::new(dot.x, dot.y);
            let fill = if dot.is_root {
                theme::ACCENT
            } else {
                theme::BG_3
            };
            frame.fill(&Path::circle(center, dot.r), fill);
            if !dot.is_root {
                frame.stroke(
                    &Path::circle(center, dot.r),
                    Stroke::default().with_width(1.0).with_color(theme::LINE),
                );
            }
            let label_color = if dot.is_root {
                theme::BG_0
            } else {
                theme::TEXT_1
            };
            frame.fill_text(canvas::Text {
                content: dot.note.to_string(),
                position: center,
                color: label_color,
                size: 11.0.into(),
                font: theme::UI_FONT_SEMIBOLD,
                align_x: TextAlignment::Center,
                align_y: alignment::Vertical::Center,
                ..canvas::Text::default()
            });
        }

        // String tuning labels below the board.
        let label_y = board_bottom + LABEL_H * 0.5;
        for s in &layout.strings {
            frame.fill_text(canvas::Text {
                content: s.label.to_string(),
                position: Point::new(s.x, label_y),
                color: theme::TEXT_3,
                size: 12.0.into(),
                font: theme::MONO_FONT,
                align_x: TextAlignment::Center,
                align_y: alignment::Vertical::Center,
                ..canvas::Text::default()
            });
        }
    }
}
