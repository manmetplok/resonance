use iced::widget::{button, checkbox, column, container, pick_list, row, text, Space};
use iced::{alignment, Element, Length};

use resonance_music_theory::{BassStyle, MelodyStyle, Mode, PitchClass, Scale};

use crate::compose::{generate::DeriveKind, ComposeMessage, SectionDefinitionState};
use crate::message::Message;
use crate::state::{TrackRole, TrackState};
use crate::theme;

pub const PANEL_WIDTH: f32 = 240.0;

/// Right-side "section panel" shown when no track is selected in the
/// Compose tab. Contains two blocks: a scale picker (top) and the
/// Generate block (bottom) which drives chord progression generation
/// plus role-based pad/bass/lead derivation.
pub fn view<'a>(
    definition: &'a SectionDefinitionState,
    tracks: &'a [TrackState],
) -> Element<'a, Message> {
    let content = column![
        scale_block(definition),
        Space::with_height(16),
        separator(),
        Space::with_height(12),
        generate_block(definition, tracks),
    ]
    .spacing(0)
    .padding(12);

    container(content)
        .width(Length::Fixed(PANEL_WIDTH))
        .height(Length::Fill)
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

fn scale_block<'a>(definition: &'a SectionDefinitionState) -> Element<'a, Message> {
    let definition_id = definition.id;
    let current = definition.scale;

    let heading = text("Scale").size(13).color(theme::TEXT);
    let current_label: Element<'a, Message> = match current {
        Some(scale) => text(scale.to_string()).size(14).color(theme::ACCENT).into(),
        None => text("(none)").size(14).color(theme::TEXT_DIM).into(),
    };

    let roots: Vec<PitchClass> = PitchClass::ALL.to_vec();
    let modes: Vec<Mode> = Mode::ALL.to_vec();

    let current_root = current.map(|s| s.root).unwrap_or(PitchClass::C);
    let current_mode = current.map(|s| s.mode).unwrap_or(Mode::Major);

    let root_picker = pick_list(roots, Some(current_root), move |root| {
        let mode = current.map(|s| s.mode).unwrap_or(Mode::Major);
        Message::Compose(ComposeMessage::SetSectionScale {
            definition_id,
            scale: Some(Scale::new(root, mode)),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let mode_picker = pick_list(modes, Some(current_mode), move |mode| {
        let root = current.map(|s| s.root).unwrap_or(PitchClass::C);
        Message::Compose(ComposeMessage::SetSectionScale {
            definition_id,
            scale: Some(Scale::new(root, mode)),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let clear_btn = button(text("Clear scale").size(12))
        .on_press(Message::Compose(ComposeMessage::SetSectionScale {
            definition_id,
            scale: None,
        }))
        .padding([4, 10])
        .width(Length::Fill)
        .style(|_theme, status| theme::transport_button_style(status));

    column![
        heading,
        current_label,
        Space::with_height(8),
        text("Root").size(11).color(theme::TEXT_DIM),
        root_picker,
        Space::with_height(6),
        text("Mode").size(11).color(theme::TEXT_DIM),
        mode_picker,
        Space::with_height(10),
        clear_btn,
    ]
    .spacing(4)
    .into()
}

fn generate_block<'a>(
    definition: &'a SectionDefinitionState,
    tracks: &'a [TrackState],
) -> Element<'a, Message> {
    let definition_id = definition.id;
    let params = &definition.generate_params;
    let has_scale = definition.scale.is_some();

    let heading = text("Generate").size(13).color(theme::TEXT);

    // Chord count picker: 1..=16. Keep it narrow.
    let count_options: Vec<u32> = (1..=16).collect();
    let count_picker = pick_list(count_options, Some(params.chord_count), move |n| {
        Message::Compose(ComposeMessage::SetGenerateChordCount {
            definition_id,
            chord_count: n,
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let beats_options: Vec<u32> = vec![1, 2, 4, 8, 16];
    let beats_picker = pick_list(beats_options, Some(params.beats_per_chord), move |n| {
        Message::Compose(ComposeMessage::SetGenerateBeatsPerChord {
            definition_id,
            beats_per_chord: n,
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let sevenths = checkbox("Seventh chords", params.seventh_chords)
        .on_toggle(move |on| {
            Message::Compose(ComposeMessage::SetGenerateSeventhChords {
                definition_id,
                seventh_chords: on,
            })
        })
        .text_size(11)
        .size(14);

    let generate_btn = {
        let btn = button(text("Generate progression").size(12))
            .padding([4, 10])
            .width(Length::Fill)
            .style(|_theme, status| theme::transport_button_style(status));
        if has_scale {
            btn.on_press(Message::Compose(ComposeMessage::GenerateProgression {
                definition_id,
            }))
        } else {
            btn
        }
    };

    let reroll_btn = {
        let btn = button(text("Reroll").size(12))
            .padding([4, 10])
            .width(Length::Fill)
            .style(|_theme, status| theme::transport_button_style(status));
        if has_scale {
            btn.on_press(Message::Compose(ComposeMessage::RerollProgression {
                definition_id,
            }))
        } else {
            btn
        }
    };

    // Bass and melody style pickers so the derive buttons below produce
    // different output without needing a second panel.
    let bass_picker = pick_list(
        BassStyle::ALL.to_vec(),
        Some(params.bass.style),
        move |style| {
            Message::Compose(ComposeMessage::SetBassStyle {
                definition_id,
                style,
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let melody_picker = pick_list(
        MelodyStyle::ALL.to_vec(),
        Some(params.melody.style),
        move |style| {
            Message::Compose(ComposeMessage::SetMelodyStyle {
                definition_id,
                style,
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let can_derive = !definition.chords.is_empty();

    let derive_pad_btn = derive_button(
        definition_id,
        DeriveKind::Pad,
        TrackRole::Pad,
        tracks,
        can_derive,
    );
    let derive_bass_btn = derive_button(
        definition_id,
        DeriveKind::Bass,
        TrackRole::Bass,
        tracks,
        can_derive,
    );
    let derive_lead_btn = derive_button(
        definition_id,
        DeriveKind::Lead,
        TrackRole::Lead,
        tracks,
        can_derive,
    );

    let derive_all_btn = {
        let btn = button(text("Derive all").size(12))
            .padding([4, 10])
            .width(Length::Fill)
            .style(|_theme, status| theme::transport_button_style(status));
        if can_derive {
            btn.on_press(Message::Compose(ComposeMessage::DeriveAllParts {
                definition_id,
            }))
        } else {
            btn
        }
    };

    let helper = if has_scale {
        text("Pad/Bass/Lead targets are the first track tagged with that role in the Instrument panel.")
            .size(10)
            .color(theme::TEXT_DIM)
    } else {
        text("Pick a scale above to enable generation.")
            .size(10)
            .color(theme::TEXT_DIM)
    };

    column![
        heading,
        Space::with_height(6),
        text("Chords").size(11).color(theme::TEXT_DIM),
        count_picker,
        Space::with_height(4),
        text("Beats / chord").size(11).color(theme::TEXT_DIM),
        beats_picker,
        Space::with_height(6),
        sevenths,
        Space::with_height(8),
        row![generate_btn, Space::with_width(4), reroll_btn].align_y(alignment::Vertical::Center),
        Space::with_height(12),
        text("Bass style").size(11).color(theme::TEXT_DIM),
        bass_picker,
        Space::with_height(4),
        text("Melody style").size(11).color(theme::TEXT_DIM),
        melody_picker,
        Space::with_height(8),
        text("Derive parts").size(11).color(theme::TEXT_DIM),
        derive_pad_btn,
        Space::with_height(4),
        derive_bass_btn,
        Space::with_height(4),
        derive_lead_btn,
        Space::with_height(4),
        derive_all_btn,
        Space::with_height(10),
        helper,
    ]
    .spacing(2)
    .into()
}

/// Build a "Derive <role>" button whose label includes the target track's
/// name (or "(no track)" when nothing is tagged for the role). The button
/// disables when there's no target track or the section has no chords
/// yet.
fn derive_button<'a>(
    definition_id: u64,
    kind: DeriveKind,
    role: TrackRole,
    tracks: &'a [TrackState],
    section_has_chords: bool,
) -> Element<'a, Message> {
    let target: Option<&TrackState> = tracks
        .iter()
        .filter(|t| t.sub_track.is_none())
        .find(|t| t.role == Some(role));
    let label = match target {
        Some(t) => format!("Derive {}  →  {}", role.as_str(), t.name),
        None => format!("Derive {}  (no track)", role.as_str()),
    };
    let btn = button(text(label).size(12))
        .padding([4, 10])
        .width(Length::Fill)
        .style(|_theme, status| theme::transport_button_style(status));
    if target.is_some() && section_has_chords {
        btn.on_press(Message::Compose(ComposeMessage::DerivePart {
            definition_id,
            kind,
        }))
        .into()
    } else {
        btn.into()
    }
}

fn separator<'a>() -> Element<'a, Message> {
    container(Space::with_height(1))
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::SEPARATOR)),
            ..Default::default()
        })
        .into()
}
