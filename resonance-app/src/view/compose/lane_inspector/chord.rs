//! Chord-lane inspector body: scale picker, chord generator settings,
//! and the section-shared motif knobs.

use iced::widget::{button, canvas, checkbox, column, pick_list, row, slider, text, Space};
use iced::{alignment, Element, Length};

use resonance_music_theory::{
    Degree, ManualMotifNote, Mode, MotifSource, PitchClass, Scale, TableRegistry,
};

use crate::compose::messages::{ChordInspectorMsg, MotifSourceKind};
use crate::compose::{ComposeMessage, SectionDefinitionState};
use crate::message::*;
use crate::theme;

use super::{label_with_info, separator};

/// Table IDs available for the chord generator, in display order.
const TABLE_IDS: &[&str] = &["pop", "modal", "jazz", "post-rock", "metal", "classical"];

/// Display names matching TABLE_IDS.
const TABLE_NAMES: &[&str] = &["Pop", "Modal", "Jazz", "Post-Rock", "Metal", "Classical"];

/// Wrapper for pick_list display.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TablePick {
    id: String,
    label: String,
}

impl std::fmt::Display for TablePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

fn table_picks() -> Vec<TablePick> {
    TABLE_IDS
        .iter()
        .zip(TABLE_NAMES.iter())
        .map(|(id, name)| TablePick {
            id: id.to_string(),
            label: name.to_string(),
        })
        .collect()
}

fn current_table_id(def: &SectionDefinitionState) -> String {
    match &def.generator_spec {
        Some(resonance_music_theory::GeneratorSpec::MarkovProgression { table_id, .. }) => {
            table_id.clone()
        }
        None => "pop".to_string(),
    }
}

/// Degree wrapper for pick_list with Display.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DegreePick(Option<Degree>);

impl std::fmt::Display for DegreePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            None => f.write_str("(any)"),
            Some(d) => write!(f, "{d}"),
        }
    }
}

/// Motif length pick for motif generator (0 = auto).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MotifLenPick(u8);

impl std::fmt::Display for MotifLenPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 == 0 {
            f.write_str("Auto")
        } else {
            write!(f, "{} notes", self.0)
        }
    }
}

// ===========================================================================
// Scale block (always visible)
// ===========================================================================

pub(super) fn scale_block<'a>(definition: &'a SectionDefinitionState) -> Element<'a, Message> {
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

// ===========================================================================
// Chord lane body
// ===========================================================================

pub(super) fn chord_body<'a>(
    definition: &'a SectionDefinitionState,
    table_registry: &'a TableRegistry,
) -> Element<'a, Message> {
    let definition_id = definition.id;
    let has_scale = definition.scale.is_some();
    let current_table = current_table_id(definition);

    let heading = text("Chord generator").size(13).color(theme::ACCENT);

    // Table picker
    let tables = table_picks();
    let current_pick = tables.iter().find(|t| t.id == current_table).cloned();
    let table_picker = pick_list(tables, current_pick, move |pick| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetTable(pick.id),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    // Chord count (length)
    let current_length = match &definition.generator_spec {
        Some(resonance_music_theory::GeneratorSpec::MarkovProgression { length, .. }) => *length,
        None => definition.generate_params.chord_count as u8,
    };
    let count_options: Vec<u8> = (1..=16).collect();
    let count_picker = pick_list(count_options, Some(current_length), move |n| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetLength(n),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    // Beats per chord
    let beats_options: Vec<u32> = vec![1, 2, 4, 8, 16];
    let beats_picker = pick_list(beats_options, Some(definition.beats_per_chord), move |n| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetBeatsPerChord(n),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    // Seventh chords
    let sevenths = checkbox("Seventh chords", definition.seventh_chords)
        .on_toggle(move |on| {
            Message::Compose(ComposeMessage::ChordInspector {
                definition_id,
                msg: ChordInspectorMsg::SetSeventhChords(on),
            })
        })
        .text_size(11)
        .size(14);

    // Start / end degree constraints — only degrees present in the
    // selected table are offered so the constraint is always satisfiable.
    let table_degrees = table_registry
        .get(&current_table)
        .map(|t| t.degrees())
        .unwrap_or_default();
    let degree_options = degree_picks_from(&table_degrees);

    let current_start = match &definition.generator_spec {
        Some(resonance_music_theory::GeneratorSpec::MarkovProgression { start, .. }) => {
            DegreePick(*start)
        }
        None => DegreePick(None),
    };
    let current_end = match &definition.generator_spec {
        Some(resonance_music_theory::GeneratorSpec::MarkovProgression { end, .. }) => {
            DegreePick(*end)
        }
        None => DegreePick(None),
    };

    let start_picker = pick_list(degree_options.clone(), Some(current_start), move |pick| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetStartDegree(pick.0),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let end_picker = pick_list(degree_options, Some(current_end), move |pick| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetEndDegree(pick.0),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    // Generate / Regenerate buttons
    let gen_btn = {
        let btn = button(text("Generate").size(12))
            .padding([4, 10])
            .width(Length::Fill)
            .style(|_theme, status| theme::transport_button_style(status));
        if has_scale {
            btn.on_press(Message::Compose(ComposeMessage::ChordInspector {
                definition_id,
                msg: ChordInspectorMsg::Generate,
            }))
        } else {
            btn
        }
    };

    let regen_btn = {
        let btn = button(text("Regenerate").size(12))
            .padding([4, 10])
            .width(Length::Fill)
            .style(|_theme, status| theme::transport_button_style(status));
        if has_scale && definition.generated_material.is_some() {
            btn.on_press(Message::Compose(ComposeMessage::ChordInspector {
                definition_id,
                msg: ChordInspectorMsg::Regenerate,
            }))
        } else {
            btn
        }
    };

    // Lock info
    let lock_count = definition
        .generated_material
        .as_ref()
        .map(|m| m.chords.iter().filter(|c| c.locked).count())
        .unwrap_or(0);
    let lock_label = if lock_count > 0 {
        text(format!("{lock_count} chord(s) locked"))
            .size(10)
            .color(theme::TEXT_DIM)
    } else {
        text("Click a chord to toggle lock")
            .size(10)
            .color(theme::TEXT_DIM)
    };

    let helper = if !has_scale {
        text("Pick a scale above to enable generation.")
            .size(10)
            .color(theme::TEXT_DIM)
    } else {
        text("").size(1)
    };

    // Seed display
    let seed_label = text(format!("Seed: 0x{:X}", definition.generator_seed))
        .size(10)
        .color(theme::TEXT_DIM);

    let motif_block = motif_section_block(definition);

    column![
        heading,
        Space::with_height(6),
        label_with_info(
            "Table",
            "Markov transition table — picks the genre vocabulary the chord walker draws from. Pop / Modal / Jazz / Post-Rock / Metal / Classical."
        ),
        table_picker,
        Space::with_height(4),
        label_with_info(
            "Chords",
            "How many chords the generator emits per Generate / Regenerate."
        ),
        count_picker,
        Space::with_height(4),
        label_with_info(
            "Beats / chord",
            "How many beats each chord occupies on the section grid. With 4 beats/bar, “4” means one chord per bar."
        ),
        beats_picker,
        Space::with_height(6),
        sevenths,
        Space::with_height(6),
        label_with_info(
            "Start degree",
            "Constrain the first generated chord to a scale degree (e.g. I, V). “(any)” lets the walker pick freely."
        ),
        start_picker,
        Space::with_height(4),
        label_with_info(
            "End degree",
            "Constrain the last generated chord to a scale degree (e.g. I for a tonic resolution). “(any)” lets the walker pick freely."
        ),
        end_picker,
        Space::with_height(8),
        row![gen_btn, Space::with_width(4), regen_btn].align_y(alignment::Vertical::Center),
        Space::with_height(6),
        seed_label,
        Space::with_height(6),
        lock_label,
        helper,
        Space::with_height(12),
        separator(),
        Space::with_height(8),
        motif_block,
    ]
    .spacing(2)
    .into()
}

/// Section-shared motif knobs. Visible in the Chords lane inspector even
/// when no lane currently consumes them, so the user can dial them in
/// before flipping a lane to a Motif style.
fn motif_section_block<'a>(definition: &'a SectionDefinitionState) -> Element<'a, Message> {
    let definition_id = definition.id;
    let params = *definition.motif_source.params();

    let heading = text("Section motif").size(13).color(theme::ACCENT);

    let source_kind = if definition.motif_source.is_manual() {
        MotifSourceKind::Manual
    } else {
        MotifSourceKind::Generated
    };
    let source_picker = pick_list(
        vec![MotifSourceKind::Generated, MotifSourceKind::Manual],
        Some(source_kind),
        move |kind| {
            Message::Compose(ComposeMessage::ChordInspector {
                definition_id,
                msg: ChordInspectorMsg::SetMotifSourceKind(kind),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let complexity_slider = slider(0.0..=1.0, params.complexity, move |v| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetMotifComplexity(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    let mut col = column![
        heading,
        Space::with_height(4),
        label_with_info(
            "Source",
            "Generated: the motif is built procedurally from the knobs below.\nManual: draw the motif by hand on the canvas. Phrase transforms still apply, so a manual motif also develops across the section."
        ),
        source_picker,
        Space::with_height(6),
        label_with_info(
            format!("Complexity: {:.2}", params.complexity),
            "Drives the per-phrase Transform plan (how aggressively the motif is varied across the section). In Generated mode it also drives motif length and rhythm complexity."
        ),
        complexity_slider,
    ]
    .spacing(2);

    match &definition.motif_source {
        MotifSource::Generated(_) => {
            let leap_slider = slider(0.0..=1.0, params.leap_chance, move |v| {
                Message::Compose(ComposeMessage::ChordInspector {
                    definition_id,
                    msg: ChordInspectorMsg::SetMotifLeapChance(v),
                })
            })
            .step(0.01)
            .width(Length::Fill);

            let motif_len_options: Vec<MotifLenPick> = vec![
                MotifLenPick(0),
                MotifLenPick(2),
                MotifLenPick(3),
                MotifLenPick(4),
                MotifLenPick(5),
                MotifLenPick(6),
            ];
            let motif_len_picker = pick_list(
                motif_len_options,
                Some(MotifLenPick(params.motif_len)),
                move |pick| {
                    Message::Compose(ComposeMessage::ChordInspector {
                        definition_id,
                        msg: ChordInspectorMsg::SetMotifLen(pick.0),
                    })
                },
            )
            .text_size(12)
            .padding([4, 6])
            .width(Length::Fill);

            let regen_btn = button(text("Regenerate motif").size(12))
                .padding([4, 10])
                .width(Length::Fill)
                .style(|_theme, status| theme::transport_button_style(status))
                .on_press(Message::Compose(ComposeMessage::ChordInspector {
                    definition_id,
                    msg: ChordInspectorMsg::RegenerateMotif,
                }));

            let seed_label = text(format!("Seed: 0x{:X}", params.seed))
                .size(10)
                .color(theme::TEXT_DIM);

            col = col
                .push(label_with_info(
                    format!("Leap chance: {:.2}", params.leap_chance),
                    "Probability of an interval leap (3–7 semitones) versus a step (1–2 semitones) when building the motif. Higher = more angular, lower = more conjunct.",
                ))
                .push(leap_slider)
                .push(Space::with_height(4))
                .push(label_with_info(
                    "Motif length",
                    "Number of notes in the motif cell. Auto picks 2–6 based on Complexity.",
                ))
                .push(motif_len_picker)
                .push(Space::with_height(8))
                .push(regen_btn)
                .push(Space::with_height(4))
                .push(seed_label);
        }
        MotifSource::Manual { notes, .. } => {
            let canvas_widget = manual_motif_canvas(definition_id, notes, definition.scale);
            let clear_btn = button(text("Clear motif").size(12))
                .padding([4, 10])
                .width(Length::Fill)
                .style(|_theme, status| theme::transport_button_style(status))
                .on_press(Message::Compose(ComposeMessage::ChordInspector {
                    definition_id,
                    msg: ChordInspectorMsg::ClearManualMotif,
                }));

            col = col
                .push(Space::with_height(4))
                .push(
                    text(format!(
                        "Motif: {} note{}",
                        notes.len(),
                        if notes.len() == 1 { "" } else { "s" }
                    ))
                    .size(11)
                    .color(theme::TEXT_DIM),
                )
                .push(canvas_widget)
                .push(text("Click empty cell to add a note. Click the bottom row to insert a rest. Click the start of a note/rest to remove it. Right-click to toggle accent. Scroll on a note to cycle its duration.")
                    .size(10)
                    .color(theme::TEXT_DIM))
                .push(Space::with_height(6))
                .push(clear_btn);
        }
    }

    col.into()
}

fn manual_motif_canvas<'a>(
    definition_id: u64,
    notes: &'a [ManualMotifNote],
    scale: Option<Scale>,
) -> Element<'a, Message> {
    use crate::view::compose::manual_motif_canvas::{CELL_H, CELL_W, GRID_COLS, TOTAL_ROWS};
    let canvas_w = CELL_W * GRID_COLS as f32;
    let canvas_h = CELL_H * TOTAL_ROWS as f32;
    canvas(
        crate::view::compose::manual_motif_canvas::ManualMotifCanvas {
            definition_id,
            notes,
            scale,
        },
    )
    .width(Length::Fixed(canvas_w))
    .height(Length::Fixed(canvas_h))
    .into()
}

fn degree_picks_from(table_degrees: &[Degree]) -> Vec<DegreePick> {
    let mut picks = vec![DegreePick(None)];
    for d in table_degrees {
        picks.push(DegreePick(Some(*d)));
    }
    picks
}
