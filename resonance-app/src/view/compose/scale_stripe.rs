//! Scale stripe shown above the chord lane in the Compose view.
//!
//! Layout per design: a card-style strip showing
//! - "SCALE" letterspaced label
//! - italic-serif scale name (e.g. "B minor", 22px)
//! - meta line ("natural · 7 notes")
//! - a row of 7 note pills on the right, with the root pill highlighted.

use iced::widget::{container, row, text, Space};
use iced::{alignment, Element, Length};

use resonance_music_theory::{Mode, PitchClass, Scale};

use crate::compose::SectionDefinitionState;
use crate::message::Message;
use crate::theme;

pub fn view<'a>(definition: &'a SectionDefinitionState) -> Element<'a, Message> {
    let scale = definition.scale.as_ref();
    let body: Element<'_, Message> = match scale {
        Some(scale) => render_with_scale(scale),
        None => render_no_scale(),
    };

    container(body)
        .width(Length::Fill)
        .padding([10, 16])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_XL.into(),
            },
            ..Default::default()
        })
        .into()
}

fn render_with_scale<'a>(scale: &Scale) -> Element<'a, Message> {
    let label = text("SCALE")
        .size(10)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3);

    let name_str = format!("{} {}", scale.root, mode_word(scale.mode));
    let name = text(name_str)
        .size(22)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::ACCENT_SOFT);

    let count = scale.mode.intervals().len();
    let meta_str = format!("{} · {} notes", mode_descriptor(scale.mode), count);
    let meta = text(meta_str).size(11).color(theme::TEXT_3);

    let left = row![
        label,
        Space::new().width(10),
        name,
        Space::new().width(10),
        meta,
    ]
    .align_y(alignment::Vertical::Bottom)
    .spacing(0);

    let pills = scale_pills(scale);

    row![
        left,
        Space::new().width(Length::Fill),
        pills,
    ]
    .align_y(alignment::Vertical::Center)
    .spacing(0)
    .into()
}

fn render_no_scale<'a>() -> Element<'a, Message> {
    let label = text("SCALE")
        .size(10)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3);
    let name = text("(no scale set)")
        .size(15)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_3);

    row![label, Space::new().width(10), name,]
        .align_y(alignment::Vertical::Bottom)
        .spacing(0)
        .into()
}

fn scale_pills<'a>(scale: &Scale) -> Element<'a, Message> {
    let root_semi = scale.root.to_semitone() as i32;
    let mut pills = row![].spacing(3);
    for (i, &iv) in scale.mode.intervals().iter().enumerate() {
        let pc = PitchClass::from_semitone(((root_semi + iv as i32) % 12) as u8);
        let active = i == 0;
        pills = pills.push(pill(&pretty_pitch(pc), active));
    }
    pills.into()
}

fn pill<'a>(label: &str, active: bool) -> Element<'a, Message> {
    let bg = if active { theme::ACCENT_DIM } else { theme::BG_1 };
    let fg = if active { theme::ACCENT_SOFT } else { theme::TEXT_2 };
    let border = if active { theme::ACCENT_LINE } else { theme::LINE_2 };
    container(
        text(label.to_string())
            .size(11)
            .font(theme::MONO_FONT)
            .color(fg),
    )
    .padding([4, 10])
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(bg)),
        border: iced::Border {
            color: border,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    })
    .into()
}

/// Convert PitchClass's "C#" display to "C♯" so it matches the design.
fn pretty_pitch(pc: PitchClass) -> String {
    pc.as_str().replace('#', "\u{266f}")
}

fn mode_word(mode: Mode) -> &'static str {
    match mode {
        Mode::Chromatic => "chromatic",
        Mode::Major => "major",
        Mode::Minor => "minor",
        Mode::Dorian => "dorian",
        Mode::Phrygian => "phrygian",
        Mode::Lydian => "lydian",
        Mode::Mixolydian => "mixolydian",
        Mode::Locrian => "locrian",
        Mode::HarmonicMinor => "harmonic minor",
        Mode::MelodicMinor => "melodic minor",
    }
}

/// One-word descriptor used in the meta line: "natural" for plain
/// major/minor, the mode name otherwise.
fn mode_descriptor(mode: Mode) -> &'static str {
    match mode {
        Mode::Chromatic => "chromatic",
        Mode::Major | Mode::Minor => "natural",
        Mode::Dorian => "modal",
        Mode::Phrygian => "modal",
        Mode::Lydian => "modal",
        Mode::Mixolydian => "modal",
        Mode::Locrian => "modal",
        Mode::HarmonicMinor => "harmonic",
        Mode::MelodicMinor => "melodic",
    }
}
