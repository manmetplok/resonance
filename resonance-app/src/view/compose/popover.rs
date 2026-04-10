use iced::widget::{button, container, pick_list, row, text, Space};
use iced::{alignment, Element, Length};

use resonance_music_theory::{Chord, ChordQuality, PitchClass};

use crate::compose::{ComposeMessage, SectionDefinitionState};
use crate::message::Message;
use crate::theme;

/// Inline editor row shown under the chord lane whenever a chord is selected.
/// Exposes root / quality / optional bass pickers and a delete button. A
/// floating popover can replace this later if a richer form is needed.
pub fn view<'a>(definition: &'a SectionDefinitionState, chord_id: u64) -> Element<'a, Message> {
    let Some(chord_state) = definition.chords.iter().find(|c| c.id == chord_id) else {
        return empty();
    };

    let current = chord_state.chord;
    let definition_id = definition.id;

    let roots: Vec<PitchClass> = PitchClass::ALL.to_vec();
    let qualities: Vec<ChordQuality> = ChordQuality::ALL.to_vec();
    let basses: Vec<BassOption> = std::iter::once(BassOption::None)
        .chain(PitchClass::ALL.iter().copied().map(BassOption::Some))
        .collect();

    let root_picker = pick_list(roots, Some(current.root), move |root| {
        Message::Compose(ComposeMessage::EditChord {
            definition_id,
            chord_id,
            chord: Chord {
                root,
                quality: current.quality,
                bass: current.bass,
            },
        })
    })
    .text_size(12)
    .padding([4, 6]);

    let quality_picker = pick_list(qualities, Some(current.quality), move |quality| {
        Message::Compose(ComposeMessage::EditChord {
            definition_id,
            chord_id,
            chord: Chord {
                root: current.root,
                quality,
                bass: current.bass,
            },
        })
    })
    .text_size(12)
    .padding([4, 6]);

    let current_bass = match current.bass {
        Some(pc) => BassOption::Some(pc),
        None => BassOption::None,
    };
    let bass_picker = pick_list(basses, Some(current_bass), move |bass| {
        let bass = match bass {
            BassOption::None => None,
            BassOption::Some(pc) => Some(pc),
        };
        Message::Compose(ComposeMessage::EditChord {
            definition_id,
            chord_id,
            chord: Chord {
                root: current.root,
                quality: current.quality,
                bass,
            },
        })
    })
    .text_size(12)
    .padding([4, 6]);

    let preview = text(current.to_string()).size(14).color(theme::TEXT);

    let delete_btn = button(text("Delete chord").size(12))
        .on_press(Message::Compose(ComposeMessage::DeleteChord {
            definition_id,
            chord_id,
        }))
        .padding([4, 10])
        .style(|_theme, status| theme::record_armed_button_style(status));

    let close_btn = button(text("Close").size(12))
        .on_press(Message::Compose(ComposeMessage::ClearChordSelection))
        .padding([4, 10])
        .style(|_theme, status| theme::transport_button_style(status));

    container(
        row![
            preview,
            Space::with_width(12),
            text("Root").size(11).color(theme::TEXT_DIM),
            root_picker,
            text("Quality").size(11).color(theme::TEXT_DIM),
            quality_picker,
            text("Bass").size(11).color(theme::TEXT_DIM),
            bass_picker,
            Space::with_width(Length::Fill),
            delete_btn,
            close_btn,
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center)
        .height(36),
    )
    .width(Length::Fill)
    .padding([0, 10])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::PANEL)),
        border: iced::Border {
            color: theme::SEPARATOR,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .into()
}

fn empty<'a>() -> Element<'a, Message> {
    container(Space::with_height(0)).width(Length::Fill).into()
}

/// Wrapper so `pick_list` can render "(none)" as an option alongside the
/// twelve pitch classes for the optional bass note field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BassOption {
    None,
    Some(PitchClass),
}

impl std::fmt::Display for BassOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BassOption::None => f.write_str("(none)"),
            BassOption::Some(pc) => write!(f, "{}", pc),
        }
    }
}
