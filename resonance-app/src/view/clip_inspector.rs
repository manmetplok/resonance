//! Clip fade / gain inspector flyout (epic #18, design doc #153, arch
//! doc #156).
//!
//! A warm card shown for the **selected editable audio clip**, giving
//! precise numeric control that complements the on-canvas drag handles
//! (fade beads + gain bead in `view/timeline/draw.rs`). It exposes:
//!
//! - numeric fade-in / fade-out length (ms),
//! - per-clip gain (dB) with `−`/`+` steppers,
//! - a per-fade curve picker (Linear / Equal-power / Exp),
//! - a **Reset to default** action ("reset to generated").
//!
//! All edits emit the shared `ClipMessage` edits handled by the update
//! todo (#317) — `SetClipFadeInMs` / `SetClipFadeOutMs` / `SetClipGainDb`
//! / `SetClipFadeInCurve` / `SetClipFadeOutCurve` / `ResetClipFadeGain` —
//! which mutate the same `ClipState` mirror the on-canvas drags use, so
//! the flyout and the canvas always agree.
//!
//! The full flyout only appears for editable audio clips. For a frozen
//! track or a clip with no editable sample source the fade controls are
//! hidden and a `BAD`-toned banner explains that gain still applies but
//! fades are disabled until the clip is editable again (design doc #153,
//! "Unsupported clip" state).

use crate::message::*;
use crate::state::ClipState;
use crate::theme;
use iced::widget::{button, column, container, row, text, text_input, Space};
use iced::{alignment, Color, Element, Length};
use resonance_audio::types::{ClipId, FadeCurve};

/// Width of the flyout card. Wide enough for the three labelled rows and
/// the curve segmented control without wrapping.
const FLYOUT_WIDTH: f32 = 248.0;

/// Gain limits (dB) for the steppers / numeric field. Matches the
/// on-canvas gain drag range so the flyout and the bead can't disagree.
const GAIN_MIN_DB: f32 = -60.0;
const GAIN_MAX_DB: f32 = 12.0;
/// One stepper press nudges the gain by this many dB.
const GAIN_STEP_DB: f32 = 0.5;

impl crate::Resonance {
    /// The clip inspector flyout, or `None` when there is no selected
    /// audio clip to inspect (no selection, or the selection is a MIDI
    /// clip). Stacked over the arrange area by `view_main_area`.
    pub(crate) fn view_clip_inspector_flyout(&self) -> Option<Element<'_, Message>> {
        let clip_id = self.interaction.selected_clip?;
        let clip = self.clips.iter().find(|c| c.id == clip_id)?;

        // A clip is editable for fades when its track is live (not frozen)
        // and it has an editable sample source. Frozen / source-less clips
        // still honour gain but can't take fades (design doc #153).
        let frozen = self.freeze.status(clip.track_id).is_frozen();
        let has_source = clip.total_frames > 0;
        let editable = !frozen && has_source;

        let card = if editable {
            self.editable_flyout(clip)
        } else {
            self.degraded_flyout(clip, frozen)
        };

        Some(
            container(card)
                .width(Length::Fixed(FLYOUT_WIDTH))
                .padding(12)
                .style(theme::editing_header_card_warm_style)
                .into(),
        )
    }

    /// The full fade/gain flyout for an editable audio clip.
    fn editable_flyout<'a>(&self, clip: &'a ClipState) -> Element<'a, Message> {
        let id = clip.id;
        let sample_rate = self.sample_rate.max(1);

        let fade_in_ms = frames_to_ms(clip.fade_in_frames, sample_rate);
        let fade_out_ms = frames_to_ms(clip.fade_out_frames, sample_rate);

        let fade_in_row = ms_field_row(
            "Fade in",
            fade_in_ms,
            move |ms| Message::Clip(ClipMessage::SetClipFadeInMs { clip_id: id, ms }),
        );
        let fade_in_curve = curve_picker(clip.fade_in_curve, move |curve| {
            Message::Clip(ClipMessage::SetClipFadeInCurve { clip_id: id, curve })
        });

        let fade_out_row = ms_field_row(
            "Fade out",
            fade_out_ms,
            move |ms| Message::Clip(ClipMessage::SetClipFadeOutMs { clip_id: id, ms }),
        );
        let fade_out_curve = curve_picker(clip.fade_out_curve, move |curve| {
            Message::Clip(ClipMessage::SetClipFadeOutCurve { clip_id: id, curve })
        });

        let gain_row = gain_row(id, clip.gain_db);

        let reset = button(
            text("Reset to default")
                .size(12)
                .font(theme::UI_FONT_MEDIUM),
        )
        .width(Length::Fill)
        .padding([6, 8])
        .on_press(Message::Clip(ClipMessage::ResetClipFadeGain { clip_id: id }))
        .style(|_theme, status| theme::ghost_button_style(status));

        column![
            header(clip),
            section_label("FADE IN"),
            fade_in_row,
            fade_in_curve,
            section_label("FADE OUT"),
            fade_out_row,
            fade_out_curve,
            section_label("GAIN"),
            gain_row,
            Space::new().height(Length::Fixed(2.0)),
            reset,
        ]
        .spacing(7)
        .into()
    }

    /// The degraded flyout for a frozen / source-less clip: a `BAD`-toned
    /// banner plus the gain control (gain still applies); fades hidden.
    fn degraded_flyout<'a>(&self, clip: &'a ClipState, frozen: bool) -> Element<'a, Message> {
        let reason = if frozen {
            "Track is frozen — fades are disabled until you unfreeze it."
        } else {
            "Clip has no editable source — fades are disabled."
        };

        let banner = container(
            column![
                text("Fades unavailable")
                    .size(12)
                    .font(theme::UI_FONT_SEMIBOLD)
                    .color(theme::BAD),
                text(reason).size(11).color(theme::TEXT_2),
                text("Clip gain still applies.")
                    .size(11)
                    .color(theme::TEXT_2),
            ]
            .spacing(3),
        )
        .width(Length::Fill)
        .padding([7, 9])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(Color { a: 0.10, ..theme::BAD })),
            border: iced::Border {
                color: Color { a: 0.45, ..theme::BAD },
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        });

        column![
            header(clip),
            banner,
            section_label("GAIN"),
            gain_row(clip.id, clip.gain_db),
        ]
        .spacing(8)
        .into()
    }
}

/// Card header: warm "CLIP" pill + the clip's name in mono.
fn header(clip: &ClipState) -> Element<'_, Message> {
    let pill = container(
        text("CLIP")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD),
    )
    .padding([2, 7])
    .style(theme::editing_pill_warm_style);

    row![
        pill,
        text(clip.name.as_str())
            .size(12)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_1),
    ]
    .spacing(7)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// A dim uppercase section label.
fn section_label<'a>(label: &'a str) -> Element<'a, Message> {
    text(label)
        .size(9)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3)
        .into()
}

/// A labelled `<value> ms` numeric row. `current_ms` is the live clip
/// value; `on_ms` builds the edit message from a freshly-parsed value.
///
/// The field is digit-only (ms are whole numbers), so every keystroke
/// parses cleanly and the controlled value stays in sync with the model
/// without a separate text buffer. An empty field reads as `0 ms` (no
/// fade), and `0` renders as an empty field with a `0` placeholder so the
/// user can type a fresh value without fighting a leading zero.
fn ms_field_row<'a, F>(label: &'a str, current_ms: u64, on_ms: F) -> Element<'a, Message>
where
    F: Fn(f32) -> Message + 'a,
{
    let value = if current_ms == 0 {
        String::new()
    } else {
        current_ms.to_string()
    };

    let field = text_input("0", &value)
        .on_input(move |s| {
            let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
            let ms = digits.parse::<u64>().unwrap_or(0);
            on_ms(ms as f32)
        })
        .width(Length::Fixed(64.0))
        .size(13)
        .font(theme::MONO_FONT)
        .align_x(alignment::Horizontal::Right)
        .padding([4, 6])
        .style(numeric_field_style);

    row![
        text(label).size(12).color(theme::TEXT_2),
        Space::new().width(Length::Fill),
        field,
        unit("ms"),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// The gain row: `−` / `+` steppers flanking a `dB` numeric field. The
/// steppers give robust bipolar adjustment (including into negative dB,
/// which a controlled text field can't start typing), while the field
/// accepts direct entry of any value that parses.
fn gain_row<'a>(clip_id: ClipId, gain_db: f32) -> Element<'a, Message> {
    // Labels carry the step size both for clarity and to keep them
    // distinct from the bare "+" add-track button elsewhere in the view.
    let dec = button(
        text(format!("\u{2212}{GAIN_STEP_DB}")) // U+2212 MINUS SIGN
            .size(11)
            .font(theme::MONO_FONT),
    )
    .padding([3, 6])
    .on_press(Message::Clip(ClipMessage::SetClipGainDb {
        clip_id,
        gain_db: (gain_db - GAIN_STEP_DB).clamp(GAIN_MIN_DB, GAIN_MAX_DB),
    }))
    .style(|_theme, status| theme::small_button_style(status));

    let inc = button(
        text(format!("+{GAIN_STEP_DB}"))
            .size(11)
            .font(theme::MONO_FONT),
    )
    .padding([3, 6])
    .on_press(Message::Clip(ClipMessage::SetClipGainDb {
        clip_id,
        gain_db: (gain_db + GAIN_STEP_DB).clamp(GAIN_MIN_DB, GAIN_MAX_DB),
    }))
    .style(|_theme, status| theme::small_button_style(status));

    let value = format!("{gain_db:.1}");
    let field = text_input("0.0", &value)
        .on_input(move |s| {
            // Keep a leading sign + digits + a single dot. Anything that
            // fully parses commits; partial input (e.g. a lone "-") falls
            // back to the current value so the field never jumps to junk.
            let gain = sanitize_signed_decimal(&s)
                .parse::<f32>()
                .unwrap_or(gain_db)
                .clamp(GAIN_MIN_DB, GAIN_MAX_DB);
            Message::Clip(ClipMessage::SetClipGainDb {
                clip_id,
                gain_db: gain,
            })
        })
        .width(Length::Fixed(56.0))
        .size(13)
        .font(theme::MONO_FONT)
        .align_x(alignment::Horizontal::Right)
        .padding([4, 6])
        .style(numeric_field_style);

    row![
        text("Gain").size(12).color(theme::TEXT_2),
        Space::new().width(Length::Fill),
        dec,
        field,
        inc,
        unit("dB"),
    ]
    .spacing(5)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// A small mono unit suffix (`ms`, `dB`).
fn unit<'a>(u: &'a str) -> Element<'a, Message> {
    text(u)
        .size(11)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3)
        .into()
}

/// A three-way curve segmented control (Linear / Eq-pow / Exp). The
/// active segment is warm-highlighted; pressing a segment emits the
/// curve edit built by `on_pick`.
fn curve_picker<'a, F>(active: FadeCurve, on_pick: F) -> Element<'a, Message>
where
    F: Fn(FadeCurve) -> Message + 'a,
{
    let seg = |label: &'a str, curve: FadeCurve| -> Element<'a, Message> {
        let is_active = curve == active;
        button(
            text(label)
                .size(10)
                .font(theme::UI_FONT_MEDIUM)
                .align_x(alignment::Horizontal::Center)
                .width(Length::Fill),
        )
        .width(Length::Fill)
        .padding([4, 4])
        .on_press(on_pick(curve))
        .style(move |_theme, status| curve_segment_style(is_active, status))
        .into()
    };

    row![
        seg("Linear", FadeCurve::Linear),
        seg("Eq-pow", FadeCurve::EqualPower),
        seg("Exp", FadeCurve::Exp),
    ]
    .spacing(4)
    .into()
}

/// Keep an optional leading `-`, the digits, and at most one `.` from a
/// raw field string so it parses as an `f32` decimal.
fn sanitize_signed_decimal(s: &str) -> String {
    let mut out = String::new();
    let mut seen_dot = false;
    for (i, c) in s.chars().enumerate() {
        match c {
            '-' if i == 0 => out.push('-'),
            '.' if !seen_dot => {
                seen_dot = true;
                out.push('.');
            }
            c if c.is_ascii_digit() => out.push(c),
            _ => {}
        }
    }
    out
}

/// Convert a fade length in frames to whole milliseconds (rounded).
fn frames_to_ms(frames: u64, sample_rate: u32) -> u64 {
    if sample_rate == 0 {
        return 0;
    }
    ((frames as f64 * 1000.0) / sample_rate as f64).round() as u64
}

/// Compact bordered numeric field style — warm value text on a dark
/// inset so the readouts read as editable without shouting.
fn numeric_field_style(
    _theme: &iced::Theme,
    status: text_input::Status,
) -> text_input::Style {
    let border_color = match status {
        text_input::Status::Focused { .. } => theme::WARM_LINE,
        _ => theme::LINE,
    };
    text_input::Style {
        background: iced::Background::Color(theme::BG_1),
        border: iced::Border {
            color: border_color,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        icon: theme::TEXT_3,
        placeholder: Color { a: 0.4, ..theme::TEXT_3 },
        value: theme::WARM,
        selection: Color { a: 0.35, ..theme::WARM },
    }
}

/// Segmented curve-button style: warm fill + warm text when active, a
/// quiet hairline when not.
fn curve_segment_style(active: bool, status: button::Status) -> button::Style {
    if active {
        button::Style {
            background: Some(iced::Background::Color(Color { a: 0.16, ..theme::WARM })),
            text_color: theme::WARM,
            border: iced::Border {
                color: theme::WARM_LINE,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        }
    } else {
        let bg = match status {
            button::Status::Hovered => theme::BG_3,
            button::Status::Pressed => theme::BG_2,
            _ => theme::BG_1,
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: theme::TEXT_2,
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        }
    }
}
