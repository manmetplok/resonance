//! Instrument-lane inspector body: generator picker + the bass / melody /
//! pad parameter panels.

use iced::widget::{button, column, pick_list, slider, text, text_input, Space};
use iced::{Element, Length};

use resonance_audio::types::TrackId;
use resonance_music_theory::{
    BassMotifMode, BassMotifPhrase, BassStyle, ContourPreference, MelodyStyle,
};

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::{
    ComposeMessage, LaneGeneratorKind, LaneGeneratorKindTag, SectionDefinitionState,
};
use crate::message::*;
use crate::state::TrackState;
use crate::theme;

use super::label_with_info;

/// Wrapper for LaneGeneratorKindTag in pick_list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GeneratorPick(LaneGeneratorKindTag);

impl GeneratorPick {
    const ALL: [GeneratorPick; 4] = [
        GeneratorPick(LaneGeneratorKindTag::Manual),
        GeneratorPick(LaneGeneratorKindTag::Bass),
        GeneratorPick(LaneGeneratorKindTag::Melody),
        GeneratorPick(LaneGeneratorKindTag::Pad),
    ];
}

impl std::fmt::Display for GeneratorPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            LaneGeneratorKindTag::Manual => "Manual",
            LaneGeneratorKindTag::Bass => "Bass",
            LaneGeneratorKindTag::Melody => "Melody",
            LaneGeneratorKindTag::Pad => "Pad",
        })
    }
}

/// MIDI note number → name for pick_list display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NotePick(u8);

impl std::fmt::Display for NotePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const NAMES: [&str; 12] = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        let name = NAMES[(self.0 % 12) as usize];
        let octave = (self.0 as i8 / 12) - 1;
        write!(f, "{name}{octave}")
    }
}

/// Note value pick for melody note duration.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NoteValuePick(u32, &'static str);

impl std::fmt::Display for NoteValuePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.1)
    }
}

/// Phrase length pick for motif generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PhraseLenPick(u8);

impl std::fmt::Display for PhraseLenPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} chords", self.0)
    }
}

pub(super) fn instrument_body<'a>(
    definition: &'a SectionDefinitionState,
    track: &'a TrackState,
) -> Element<'a, Message> {
    let definition_id = definition.id;
    let track_id = track.id;

    let heading = text(&track.name).size(13).color(theme::ACCENT);

    // Track details: name, type, icon, role
    let name_input = text_input("Name", &track.name)
        .on_input(move |s| Message::Track(TrackMessage::SetTrackName(track_id, s)))
        .size(12)
        .padding([4, 6])
        .width(Length::Fill);

    // Generator type picker
    let current_gen = match definition.lane_generators.get(&track_id) {
        Some(cfg) => match &cfg.kind {
            LaneGeneratorKind::Bass(_) => GeneratorPick(LaneGeneratorKindTag::Bass),
            LaneGeneratorKind::Melody(_) => GeneratorPick(LaneGeneratorKindTag::Melody),
            LaneGeneratorKind::Pad(_) => GeneratorPick(LaneGeneratorKindTag::Pad),
            LaneGeneratorKind::Drum(_) => GeneratorPick(LaneGeneratorKindTag::Manual),
        },
        None => GeneratorPick(LaneGeneratorKindTag::Manual),
    };

    let gen_picker = pick_list(
        GeneratorPick::ALL.to_vec(),
        Some(current_gen),
        move |pick| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetGenerator(pick.0),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    // Generator-specific controls
    let gen_controls: Element<'a, Message> = match definition.lane_generators.get(&track_id) {
        Some(cfg) => match &cfg.kind {
            LaneGeneratorKind::Bass(params) => bass_controls(definition_id, track_id, params),
            LaneGeneratorKind::Melody(params) => melody_controls(definition_id, track_id, params),
            LaneGeneratorKind::Pad(params) => pad_controls(definition_id, track_id, params),
            LaneGeneratorKind::Drum(_) => manual_hint(),
        },
        None => manual_hint(),
    };

    // Regenerate button (only for non-manual lanes)
    let has_generator = definition.lane_generators.contains_key(&track_id);
    let has_scale = definition.scale.is_some();
    let has_chords = !definition.chords.is_empty();
    let can_regen = has_generator && has_scale && has_chords;

    let regen_btn = {
        let btn = button(text("Regenerate").size(12))
            .padding([4, 10])
            .width(Length::Fill)
            .style(|_theme, status| theme::transport_button_style(status));
        if can_regen {
            btn.on_press(Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::Regenerate,
            }))
        } else {
            btn
        }
    };

    // Seed display
    let seed_text = definition
        .lane_generators
        .get(&track_id)
        .map(|cfg| format!("Seed: 0x{:X}", cfg.seed))
        .unwrap_or_default();

    let mut col = column![
        heading,
        Space::with_height(6),
        text("Name").size(11).color(theme::TEXT_DIM),
        name_input,
        Space::with_height(8),
        text("Generator").size(11).color(theme::TEXT_DIM),
        gen_picker,
        Space::with_height(8),
        gen_controls,
    ]
    .spacing(2);

    if has_generator {
        col = col
            .push(Space::with_height(8))
            .push(regen_btn)
            .push(Space::with_height(4))
            .push(text(seed_text).size(10).color(theme::TEXT_DIM));

        if !has_chords {
            col = col.push(
                text("Add chords to enable generation.")
                    .size(10)
                    .color(theme::TEXT_DIM),
            );
        }
    }

    col.into()
}

fn manual_hint<'a>() -> Element<'a, Message> {
    text("No generator — edit notes directly on the piano roll.")
        .size(10)
        .color(theme::TEXT_DIM)
        .into()
}

fn bass_controls<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &'a resonance_music_theory::BassParams,
) -> Element<'a, Message> {
    let style_picker = pick_list(BassStyle::ALL.to_vec(), Some(params.style), move |style| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetBassStyle(style),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let base_note_options: Vec<u8> = (16..=52).collect(); // C1 to E3
    let base_note_picker = pick_list(
        base_note_options
            .iter()
            .map(|n| NotePick(*n))
            .collect::<Vec<_>>(),
        Some(NotePick(params.base_note)),
        move |pick| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetBassBaseNote(pick.0),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let vel_slider = slider(0.0..=1.0, params.velocity, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetBassVelocity(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    let mut col = column![
        label_with_info(
            "Style",
            "Bass voicing pattern.\n\u{2022} Root hold: one note per chord, full duration\n\u{2022} Root pulse: root on every beat\n\u{2022} Root + fifth: alternating root/fifth per beat\n\u{2022} Octave: root and root+12 alternating\n\u{2022} Walking: stepwise scale line into next chord (needs a scale)\n\u{2022} Motif: render the section’s shared motif in the bass register"
        ),
        style_picker,
        Space::with_height(4),
        label_with_info(
            "Base note",
            "MIDI floor for bass roots. Each chord’s root is moved to the nearest pitch at or above this note."
        ),
        base_note_picker,
        Space::with_height(4),
        label_with_info(
            format!("Velocity: {:.2}", params.velocity),
            "MIDI velocity (0–1) for emitted notes. Accented motif notes get a small +0.05 boost on top."
        ),
        vel_slider,
    ]
    .spacing(2);

    if params.style == BassStyle::Motif {
        let mode_picker = pick_list(
            BassMotifMode::ALL.to_vec(),
            Some(params.motif_mode),
            move |mode| {
                Message::Compose(ComposeMessage::LaneInspector {
                    definition_id,
                    track_id,
                    msg: LaneInspectorMsg::SetBassMotifMode(mode),
                })
            },
        )
        .text_size(12)
        .padding([4, 6])
        .width(Length::Fill);

        let phrase_picker = pick_list(
            BassMotifPhrase::ALL.to_vec(),
            Some(params.motif_phrase),
            move |phrase| {
                Message::Compose(ComposeMessage::LaneInspector {
                    definition_id,
                    track_id,
                    msg: LaneInspectorMsg::SetBassMotifPhrase(phrase),
                })
            },
        )
        .text_size(12)
        .padding([4, 6])
        .width(Length::Fill);

        col = col
            .push(Space::with_height(8))
            .push(label_with_info(
                "Motif mode",
                "How the bass renders the section motif.\n\u{2022} Same intervals: literal motif at the bass anchor\n\u{2022} Augmented: same intervals, each note 2× longer (slow line under the melody)\n\u{2022} Rhythm only: motif rhythm + accents, pitch is the chord root\n\u{2022} First note only: one note per chord on the chord root"
            ))
            .push(mode_picker)
            .push(Space::with_height(4))
            .push(label_with_info(
                "Phrase development",
                "How per-phrase Transforms are picked.\n\u{2022} Simple: Identity every phrase — predictable foundation\n\u{2022} Mirror melody: same Transform per phrase as the melody motif lane (locked together)\n\u{2022} Restricted: random Identity/Augment per phrase, independent of melody"
            ))
            .push(phrase_picker)
            .push(Space::with_height(4))
            .push(
                text("Motif knobs live in the Chords lane.")
                    .size(10)
                    .color(theme::TEXT_DIM),
            );
    }

    col.into()
}

fn melody_controls<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &'a resonance_music_theory::MelodyParams,
) -> Element<'a, Message> {
    let style_picker = pick_list(
        MelodyStyle::ALL.to_vec(),
        Some(params.style),
        move |style| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetMelodyStyle(style),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let reg_lo_options: Vec<u8> = (36..=84).collect();
    let reg_hi_options: Vec<u8> = (36..=96).collect();

    let reg_lo_picker = pick_list(
        reg_lo_options
            .iter()
            .map(|n| NotePick(*n))
            .collect::<Vec<_>>(),
        Some(NotePick(params.register.0)),
        move |pick| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetMelodyRegisterLow(pick.0),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let reg_hi_picker = pick_list(
        reg_hi_options
            .iter()
            .map(|n| NotePick(*n))
            .collect::<Vec<_>>(),
        Some(NotePick(params.register.1)),
        move |pick| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetMelodyRegisterHigh(pick.0),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    // Note value as a user-friendly pick list
    let note_values = vec![
        NoteValuePick(480, "Quarter"),
        NoteValuePick(240, "Eighth"),
        NoteValuePick(120, "Sixteenth"),
    ];
    let current_nv = note_values
        .iter()
        .find(|nv| nv.0 == params.note_value_ticks)
        .cloned()
        .unwrap_or(NoteValuePick(params.note_value_ticks, "Custom"));

    let nv_picker = pick_list(note_values, Some(current_nv), move |pick| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetMelodyNoteValue(pick.0),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let rest_slider = slider(0.0..=1.0, params.rest_density, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetMelodyRestDensity(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    let vel_slider = slider(0.0..=1.0, params.velocity, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetMelodyVelocity(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    let mut col = column![
        label_with_info(
            "Style",
            "Melodic generator.\n\u{2022} Arp up / down / up-down: cycle through chord tones\n\u{2022} Motif: develop a short cell across phrases (uses the section motif knobs)"
        ),
        style_picker,
        Space::with_height(4),
        label_with_info(
            "Register low",
            "Lowest MIDI note this melody is allowed to play."
        ),
        reg_lo_picker,
        label_with_info(
            "Register high",
            "Highest MIDI note this melody is allowed to play."
        ),
        reg_hi_picker,
        Space::with_height(4),
    ]
    .spacing(2);

    // Arp-only controls
    if params.style != MelodyStyle::Motif {
        col = col
            .push(label_with_info(
                "Note value",
                "Length of one melody note (arp styles only). Quarter / Eighth / Sixteenth at the project tempo."
            ))
            .push(nv_picker);
    }

    col = col
        .push(Space::with_height(4))
        .push(label_with_info(
            format!("Rest density: {:.2}", params.rest_density),
            "Probability that any given slot is silent. 0 = no rests. Higher values produce sparser, more breathing melodies."
        ))
        .push(rest_slider)
        .push(label_with_info(
            format!("Velocity: {:.2}", params.velocity),
            "Base MIDI velocity (0–1). Motif accents add a small +0.05 boost on top."
        ))
        .push(vel_slider);

    // Motif-specific controls — only those that are lane-local. The
    // motif's own knobs (complexity / motif length / leap chance) live
    // on the section so every Motif lane shares one identity.
    if params.style == MelodyStyle::Motif {
        let articulation_slider = slider(0.0..=1.0, params.articulation, move |v| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetMelodyArticulation(v),
            })
        })
        .step(0.01)
        .width(Length::Fill);

        let contour_picker = pick_list(
            ContourPreference::ALL.to_vec(),
            Some(params.contour),
            move |c| {
                Message::Compose(ComposeMessage::LaneInspector {
                    definition_id,
                    track_id,
                    msg: LaneInspectorMsg::SetMelodyContour(c),
                })
            },
        )
        .text_size(12)
        .padding([4, 6])
        .width(Length::Fill);

        let phrase_len_options = vec![
            PhraseLenPick(2),
            PhraseLenPick(4),
            PhraseLenPick(8),
        ];
        let phrase_len_picker = pick_list(
            phrase_len_options,
            Some(PhraseLenPick(params.phrase_len)),
            move |pick| {
                Message::Compose(ComposeMessage::LaneInspector {
                    definition_id,
                    track_id,
                    msg: LaneInspectorMsg::SetMelodyPhraseLen(pick.0),
                })
            },
        )
        .text_size(12)
        .padding([4, 6])
        .width(Length::Fill);

        col = col
            .push(Space::with_height(4))
            .push(label_with_info(
                format!("Articulation: {:.2}", params.articulation),
                "How short each note sounds relative to its rhythmic slot. 0 = legato (full slot), 1 = staccato (about 45% of the slot)."
            ))
            .push(articulation_slider)
            .push(Space::with_height(4))
            .push(label_with_info(
                "Contour",
                "Preferred melodic shape per phrase. Auto picks one per phrase from research-weighted distributions; the others pin every phrase to the chosen shape."
            ))
            .push(contour_picker)
            .push(label_with_info(
                "Phrase length",
                "How many chords belong to one phrase. Each phrase gets its own contour and Transform."
            ))
            .push(phrase_len_picker)
            .push(Space::with_height(4))
            .push(
                text("Motif knobs live in the Chords lane.")
                    .size(10)
                    .color(theme::TEXT_DIM),
            );
    }

    col.into()
}

fn pad_controls<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &'a resonance_music_theory::PadParams,
) -> Element<'a, Message> {
    let reg_lo_options: Vec<u8> = (36..=84).collect();
    let reg_hi_options: Vec<u8> = (36..=96).collect();

    let reg_lo_picker = pick_list(
        reg_lo_options
            .iter()
            .map(|n| NotePick(*n))
            .collect::<Vec<_>>(),
        Some(NotePick(params.register.0)),
        move |pick| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetPadRegisterLow(pick.0),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let reg_hi_picker = pick_list(
        reg_hi_options
            .iter()
            .map(|n| NotePick(*n))
            .collect::<Vec<_>>(),
        Some(NotePick(params.register.1)),
        move |pick| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetPadRegisterHigh(pick.0),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let vel_slider = slider(0.0..=1.0, params.velocity, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetPadVelocity(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    column![
        label_with_info(
            "Register low",
            "Lowest MIDI note the pad voicings can reach. Voices that fall below this float up an octave."
        ),
        reg_lo_picker,
        label_with_info(
            "Register high",
            "Highest MIDI note the pad voicings can reach. Voices that rise above this drop an octave."
        ),
        reg_hi_picker,
        Space::with_height(4),
        label_with_info(
            format!("Velocity: {:.2}", params.velocity),
            "MIDI velocity (0–1) for every emitted pad voice."
        ),
        vel_slider,
    ]
    .spacing(2)
    .into()
}
