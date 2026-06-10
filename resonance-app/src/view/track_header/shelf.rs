//! Global-shelf lane headers on the column side of the Arrange view:
//! the always-visible 32 px shelf header strip (caret + `GLOBAL` tag +
//! count pill + add-track button) and — when expanded — the three lane
//! labels (chords / tempo / signature). These mirror the canvas-side
//! global lanes row-for-row; see the module doc-comment in `super` for
//! how the chrome subtree masks lane-scroll bleed.
use iced::widget::{button, column, container, mouse_area, pick_list, row, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::state;
use crate::theme::{self, fa};
use crate::Resonance;

/// Numerator wrapper for the pick_list (needs Display + PartialEq).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Numerator(u8);
impl std::fmt::Display for Numerator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Denominator(u8);
impl std::fmt::Display for Denominator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Time-signature numerator options (1..=16) cached as a static slice.
/// `pick_list` takes its options by value (a `Borrow<[T]>`), so building
/// `(1..=16).map(Numerator).collect()` every frame allocates a fresh
/// `Vec<Numerator>` per repaint while the signature row is visible.
fn numerator_options() -> &'static [Numerator] {
    static V: std::sync::OnceLock<Vec<Numerator>> = std::sync::OnceLock::new();
    V.get_or_init(|| (1..=16).map(Numerator).collect())
}

/// Time-signature denominator options (powers of two from 2 to 16),
/// cached as a static slice. See `numerator_options` for the rationale.
fn denominator_options() -> &'static [Denominator] {
    static V: std::sync::OnceLock<Vec<Denominator>> = std::sync::OnceLock::new();
    V.get_or_init(|| [2, 4, 8, 16].into_iter().map(Denominator).collect())
}

/// Build the always-visible 32 px global-shelf header strip on the
/// column side. Contains the caret toggle, `GLOBAL` tag, and a small
/// count badge ("3" = chords + tempo + sig). Clicking anywhere on the
/// strip toggles the shelf open / closed.
pub(super) fn build_global_shelf_header(expanded: bool) -> Element<'static, Message> {
    let caret = if expanded {
        fa::CARET_DOWN
    } else {
        fa::CARET_RIGHT
    };
    let caret_el = container(theme::icon(caret).size(9).color(theme::TEXT_3))
        .width(12)
        .height(12)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    let global_tag = text("GLOBAL")
        .size(10)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_2);

    // Small `3` count pill — mirrors the design's `gsTagCount`.
    let count_pill = container(
        text("3")
            .size(9)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_3),
    )
    .padding(iced::Padding {
        top: 1.0,
        right: 5.0,
        bottom: 1.0,
        left: 5.0,
    })
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: 3.0.into(),
        },
        ..Default::default()
    });

    // Right-side: small `+` button — adds a new instrument/audio track.
    // Lives here (in the column-side shelf header) so it's reachable
    // regardless of whether the shelf is expanded. Replaces the previous
    // standalone "TRACKS · +" row that used to sit between the ruler and
    // the lane area before this redesign.
    let add_btn = button(text("+").size(13).color(theme::TEXT_3))
        .on_press(Message::Ui(UiMessage::OpenAddTrackMenu))
        .style(|_theme, status| theme::ghost_button_style(status))
        .padding(iced::Padding {
            top: 0.0,
            right: 6.0,
            bottom: 2.0,
            left: 6.0,
        })
        .width(22)
        .height(22);

    let inner = row![
        Space::new().width(10),
        caret_el,
        Space::new().width(6),
        global_tag,
        Space::new().width(6),
        count_pill,
        Space::new().width(Length::Fill),
        add_btn,
        Space::new().width(8),
    ]
    .align_y(alignment::Vertical::Center)
    .height(theme::GLOBAL_SHELF_HEADER_HEIGHT);

    let strip = container(inner)
        .width(Length::Fill)
        .height(theme::GLOBAL_SHELF_HEADER_HEIGHT)
        .style(theme::base_bg);

    mouse_area(strip)
        .on_press(Message::Ui(UiMessage::ToggleGlobalTracks))
        .into()
}

/// Common chrome for a single global-shelf lane label. Renders a
/// 22 px rounded-square glyph + name (12 px Medium, TEXT_1) + sub-line
/// (10 px Mono, TEXT_3), with optional warm tint on the glyph for the
/// tempo lane (matching the canvas-side automation curve color).
fn build_global_lane_label(
    glyph: char,
    name: &'static str,
    sub: String,
    height: f32,
    warm: bool,
) -> Element<'static, Message> {
    let glyph_color = if warm { theme::WARM } else { theme::TEXT_2 };
    let glyph_box = container(theme::icon(glyph).size(11).color(glyph_color))
        .width(theme::GLOBAL_TRACK_GLYPH_SIZE)
        .height(theme::GLOBAL_TRACK_GLYPH_SIZE)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 5.0.into(),
            },
            ..Default::default()
        });

    let name_el = text(name)
        .size(12)
        .font(theme::UI_FONT_MEDIUM)
        .color(theme::TEXT_1);
    let sub_el = text(sub).size(10).font(theme::MONO_FONT).color(theme::TEXT_3);
    let name_col = column![name_el, sub_el].spacing(1);

    // Mini M / Lock control cluster — placeholders for parity with the
    // design. Wired to no-ops via a `small_button_style` ghost.
    let m_btn = button(
        text("M")
            .size(9)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_4),
    )
    .style(|_theme, status| theme::small_button_style(status))
    .padding([0, 3])
    .width(16)
    .height(16);
    let lock_btn = button(
        text("L")
            .size(9)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_4),
    )
    .style(|_theme, status| theme::small_button_style(status))
    .padding([0, 3])
    .width(16)
    .height(16);
    let controls = row![m_btn, Space::new().width(2), lock_btn].spacing(0);

    let inner = row![
        Space::new().width(14),
        glyph_box,
        Space::new().width(9),
        name_col,
        Space::new().width(Length::Fill),
        controls,
        Space::new().width(8),
    ]
    .align_y(alignment::Vertical::Center)
    .height(height);

    container(inner)
        .width(Length::Fill)
        .height(height)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::GLOBAL_TRACK_BG)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Chord lane label — name "Chords", sub "from sections · N chords".
pub(super) fn view_chord_lane_header(r: &Resonance) -> Element<'static, Message> {
    let total: usize = r
        .compose
        .definitions
        .iter()
        .map(|d| d.chords.len())
        .sum();
    let sub = if total == 0 {
        "from sections".to_string()
    } else if total == 1 {
        "from sections · 1 chord".to_string()
    } else {
        format!("from sections · {} chords", total)
    };
    build_global_lane_label(
        fa::MUSIC,
        "Chords",
        sub,
        theme::GLOBAL_TRACK_CHORD_HEIGHT,
        false,
    )
}

/// Tempo lane label — name "Tempo", sub "{BPM} BPM · automated" when
/// >1 tempo event, else "{BPM} BPM".
pub(super) fn view_tempo_lane_header(r: &Resonance) -> Element<'static, Message> {
    let bpm = r.transport.bpm;
    let sub = if r.tempo_events.len() > 1 {
        format!("{:.1} BPM · automated", bpm)
    } else {
        format!("{:.1} BPM", bpm)
    };
    build_global_lane_label(
        fa::WAVE_SQUARE,
        "Tempo",
        sub,
        theme::GLOBAL_TRACK_TEMPO_HEIGHT,
        true,
    )
}

/// Signature lane label — name "Signature", sub "{n}/{d}" (or
/// "Mixed" when multiple distinct signatures exist in the project).
/// When a signature event is selected, surfaces inline pick_lists so
/// the user can edit numerator and denominator without leaving the
/// shelf — keeps the pre-redesign editing affordance intact.
pub(super) fn view_signature_lane_header(r: &Resonance) -> Element<'static, Message> {
    let row_h = theme::GLOBAL_TRACK_SIG_HEIGHT;

    let selected = r.interaction.selected_global_event.and_then(|sel| {
        if sel.kind == state::GlobalTrackKind::Signature {
            r.signature_events.get(sel.index).map(|ev| (sel.index, ev))
        } else {
            None
        }
    });

    // Header: glyph + "Signature" + sub-line OR inline pickers.
    let glyph_box = container(
        theme::icon(fa::SLIDERS)
            .size(11)
            .color(theme::TEXT_2),
    )
    .width(theme::GLOBAL_TRACK_GLYPH_SIZE)
    .height(theme::GLOBAL_TRACK_GLYPH_SIZE)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 0.0,
            radius: 5.0.into(),
        },
        ..Default::default()
    });

    let name_el = text("Signature")
        .size(12)
        .font(theme::UI_FONT_MEDIUM)
        .color(theme::TEXT_1);

    let inner: Element<'static, Message> = if let Some((idx, event)) = selected {
        let num = event.numerator;
        let den = event.denominator;
        let num_picker = pick_list(numerator_options(), Some(Numerator(num)), move |n: Numerator| {
            Message::GlobalTrack(GlobalTrackMessage::UpdateSignatureEvent {
                index: idx,
                numerator: n.0,
                denominator: den,
            })
        })
        .text_size(10)
        .width(38);
        let den_picker =
            pick_list(denominator_options(), Some(Denominator(den)), move |d: Denominator| {
                Message::GlobalTrack(GlobalTrackMessage::UpdateSignatureEvent {
                    index: idx,
                    numerator: num,
                    denominator: d.0,
                })
            })
            .text_size(10)
            .width(38);
        let slash = text("/").size(11).color(theme::TEXT_3);
        row![
            Space::new().width(14),
            glyph_box,
            Space::new().width(9),
            name_el,
            Space::new().width(8),
            num_picker,
            slash,
            den_picker,
            Space::new().width(Length::Fill),
            Space::new().width(8),
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center)
        .height(row_h)
        .into()
    } else {
        let sub_text = format!(
            "{}/{}",
            r.transport.time_sig_num, r.transport.time_sig_den
        );
        let sub_el = text(sub_text)
            .size(10)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_3);
        let name_col = column![name_el, sub_el].spacing(1);
        row![
            Space::new().width(14),
            glyph_box,
            Space::new().width(9),
            name_col,
            Space::new().width(Length::Fill),
            Space::new().width(8),
        ]
        .align_y(alignment::Vertical::Center)
        .height(row_h)
        .into()
    };

    container(inner)
        .width(Length::Fill)
        .height(row_h)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::GLOBAL_TRACK_BG)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}
