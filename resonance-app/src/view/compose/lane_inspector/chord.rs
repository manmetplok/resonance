//! Chord-lane inspector body — matches the redesign's right rail:
//! a compact scale picker at the top, then "Chord generator" with the
//! style/length/beat/seventh-chords/start/end controls + a primary
//! lavender Generate action and an ↻ regenerate ghost button + seed
//! footer, then a "Section motif" block with source/complexity/preview.

use iced::widget::{button, canvas, column, pick_list, row, slider, text, Space};
use iced::{alignment, Element, Length};

use resonance_music_theory::{
    Degree, ManualMotifNote, Mode, MotifSource, PitchClass, Scale, TableRegistry,
};

use crate::compose::messages::{ChordInspectorMsg, MotifSourceKind};
use crate::compose::{ComposeMessage, SectionDefinitionState};
use crate::message::*;
use crate::theme;

const TABLE_IDS: &[&str] = &["pop", "modal", "jazz", "post-rock", "metal", "classical"];
const TABLE_NAMES: &[&str] = &["Pop", "Modal", "Jazz", "Post-Rock", "Metal", "Classical"];

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

// ---------------------------------------------------------------------------
// Section header — uppercase letterspaced label with a bottom hairline.
// ---------------------------------------------------------------------------

fn section_header<'a>(title: &'static str) -> Element<'a, Message> {
    column![
        text(title)
            .size(13)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_1),
        Space::with_height(6),
        Space::with_height(1)
            .width(Length::Fill),
        crate::view::compose::lane_inspector::separator(),
    ]
    .spacing(0)
    .into()
}

/// Small uppercase field label, matching the design's letterspaced FIELD
/// captions.
fn field_label<'a>(label: impl Into<String>) -> Element<'a, Message> {
    text(label.into())
        .size(10)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3)
        .into()
}

// ---------------------------------------------------------------------------
// Scale block — compact root + mode pickers.
// ---------------------------------------------------------------------------

pub(super) fn scale_block<'a>(definition: &'a SectionDefinitionState) -> Element<'a, Message> {
    let definition_id = definition.id;
    let current = definition.scale;

    let current_label: Element<'a, Message> = match current {
        Some(scale) => text(scale.to_string())
            .size(15)
            .font(theme::SERIF_ITALIC_FONT)
            .color(theme::ACCENT_SOFT)
            .into(),
        None => text("(no scale set)")
            .size(13)
            .font(theme::SERIF_ITALIC_FONT)
            .color(theme::TEXT_3)
            .into(),
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
    .padding([5, 8])
    .width(Length::Fill);

    let mode_picker = pick_list(modes, Some(current_mode), move |mode| {
        let root = current.map(|s| s.root).unwrap_or(PitchClass::C);
        Message::Compose(ComposeMessage::SetSectionScale {
            definition_id,
            scale: Some(Scale::new(root, mode)),
        })
    })
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let clear_btn = button(text("Clear scale").size(11).color(theme::TEXT_3))
        .on_press(Message::Compose(ComposeMessage::SetSectionScale {
            definition_id,
            scale: None,
        }))
        .padding([5, 10])
        .width(Length::Fill)
        .style(|_theme, status| theme::ghost_button_style(status));

    column![
        section_header("Scale"),
        Space::with_height(8),
        current_label,
        Space::with_height(8),
        field_label("ROOT"),
        Space::with_height(4),
        root_picker,
        Space::with_height(8),
        field_label("MODE"),
        Space::with_height(4),
        mode_picker,
        Space::with_height(8),
        clear_btn,
    ]
    .spacing(0)
    .into()
}

// ---------------------------------------------------------------------------
// Chord generator + section motif body.
// ---------------------------------------------------------------------------

pub(super) fn chord_body<'a>(
    definition: &'a SectionDefinitionState,
    table_registry: &'a TableRegistry,
) -> Element<'a, Message> {
    let definition_id = definition.id;
    let has_scale = definition.scale.is_some();
    let current_table = current_table_id(definition);

    let tables = table_picks();
    let current_pick = tables.iter().find(|t| t.id == current_table).cloned();
    let table_picker = pick_list(tables, current_pick, move |pick| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetTable(pick.id),
        })
    })
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

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
    .padding([5, 8])
    .width(Length::Fill);

    let beats_options: Vec<u32> = vec![1, 2, 4, 8, 16];
    let beats_picker = pick_list(beats_options, Some(definition.beats_per_chord), move |n| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetBeatsPerChord(n),
        })
    })
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

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
    .padding([5, 8])
    .width(Length::Fill);

    let end_picker = pick_list(degree_options, Some(current_end), move |pick| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetEndDegree(pick.0),
        })
    })
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let sevenths = toggle_row(
        "Seventh chords",
        definition.seventh_chords,
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetSeventhChords(!definition.seventh_chords),
        }),
    );

    // Generate primary action + ↻ regenerate ghost.
    let gen_btn: Element<'_, Message> = {
        let label_color = if has_scale {
            theme::BG_0
        } else {
            theme::TEXT_3
        };
        let mut b = button(
            text("Generate")
                .size(12)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(label_color),
        )
        .padding([8, 14])
        .width(Length::Fill)
        .style(move |_theme, status| {
            if has_scale {
                theme::primary_button_style(status)
            } else {
                theme::ghost_button_style(status)
            }
        });
        if has_scale {
            b = b.on_press(Message::Compose(ComposeMessage::ChordInspector {
                definition_id,
                msg: ChordInspectorMsg::Generate,
            }));
        }
        b.into()
    };

    let regen_active = has_scale && definition.generated_material.is_some();
    let regen_btn: Element<'_, Message> = {
        let icon_color = if regen_active {
            theme::TEXT_1
        } else {
            theme::TEXT_3
        };
        let mut b = button(
            iced::widget::container(theme::icon(theme::fa::ARROW_ROTATE_LEFT).size(13).color(icon_color))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .padding(0)
        .width(Length::Fixed(40.0))
        .height(Length::Fixed(34.0))
        .style(|_theme, status| theme::ghost_button_style(status));
        if regen_active {
            b = b.on_press(Message::Compose(ComposeMessage::ChordInspector {
                definition_id,
                msg: ChordInspectorMsg::Regenerate,
            }));
        }
        b.into()
    };

    let action_row = row![gen_btn, Space::with_width(8), regen_btn]
        .spacing(0)
        .align_y(alignment::Vertical::Center);

    let lock_count = definition
        .generated_material
        .as_ref()
        .map(|m| m.chords.iter().filter(|c| c.locked).count())
        .unwrap_or(0);
    let lock_label: Element<'_, Message> = if lock_count > 0 {
        text(format!("{lock_count} chord(s) locked"))
            .size(10)
            .color(theme::TEXT_3)
            .into()
    } else {
        text("Click a chord to toggle lock")
            .size(10)
            .color(theme::TEXT_3)
            .into()
    };

    let scale_helper: Element<'_, Message> = if !has_scale {
        text("Pick a scale above to enable generation.")
            .size(10)
            .color(theme::TEXT_3)
            .into()
    } else {
        Space::with_height(0).into()
    };

    let seed_label = text(format!("seed · 0x{:016X}", definition.generator_seed))
        .size(10)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3);

    let two_cols = |left: Element<'a, Message>, right: Element<'a, Message>| {
        row![
            column![left].width(Length::FillPortion(1)),
            Space::with_width(10),
            column![right].width(Length::FillPortion(1)),
        ]
        .spacing(0)
    };

    let chords_count_block = column![
        field_label("CHORDS"),
        Space::with_height(4),
        count_picker,
    ];
    let beats_block = column![
        field_label("BEATS / CHORD"),
        Space::with_height(4),
        beats_picker,
    ];
    let start_block = column![
        field_label("START °"),
        Space::with_height(4),
        start_picker,
    ];
    let end_block = column![field_label("END °"), Space::with_height(4), end_picker,];

    let motif_block = motif_section_block(definition);

    column![
        section_header("Chord generator"),
        Space::with_height(10),
        field_label("STYLE"),
        Space::with_height(4),
        table_picker,
        Space::with_height(10),
        two_cols(chords_count_block.into(), beats_block.into()),
        Space::with_height(10),
        two_cols(start_block.into(), end_block.into()),
        Space::with_height(10),
        sevenths,
        Space::with_height(12),
        action_row,
        Space::with_height(8),
        seed_label,
        Space::with_height(6),
        lock_label,
        scale_helper,
        Space::with_height(20),
        motif_block,
    ]
    .spacing(0)
    .into()
}

/// Section motif block — Source select + Complexity slider + a small
/// preview card. Matches the design's "Section motif" panel.
fn motif_section_block<'a>(definition: &'a SectionDefinitionState) -> Element<'a, Message> {
    let definition_id = definition.id;
    let params = *definition.motif_source.params();

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
    .padding([5, 8])
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
        section_header("Section motif"),
        Space::with_height(10),
        field_label("SOURCE"),
        Space::with_height(4),
        source_picker,
        Space::with_height(10),
        field_label(format!("COMPLEXITY · {:.2}", params.complexity)),
        Space::with_height(6),
        complexity_slider,
    ]
    .spacing(0);

    // Motif preview card mirroring the design's MOTIF stats + dashes.
    col = col.push(Space::with_height(10));
    col = col.push(motif_preview_card(definition));

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
            .padding([5, 8])
            .width(Length::Fill);

            let regen_btn = button(text("Regenerate motif").size(12).color(theme::TEXT_1))
                .padding([6, 10])
                .width(Length::Fill)
                .style(|_theme, status| theme::ghost_button_style(status))
                .on_press(Message::Compose(ComposeMessage::ChordInspector {
                    definition_id,
                    msg: ChordInspectorMsg::RegenerateMotif,
                }));

            let seed_label = text(format!("seed · 0x{:016X}", params.seed))
                .size(10)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_3);

            col = col
                .push(Space::with_height(10))
                .push(field_label(format!(
                    "LEAP CHANCE · {:.2}",
                    params.leap_chance
                )))
                .push(Space::with_height(6))
                .push(leap_slider)
                .push(Space::with_height(10))
                .push(field_label("MOTIF LENGTH"))
                .push(Space::with_height(4))
                .push(motif_len_picker)
                .push(Space::with_height(10))
                .push(regen_btn)
                .push(Space::with_height(6))
                .push(seed_label)
                .push(Space::with_height(8))
                .push(
                    text("Click a cell to add a note. Right-click to toggle accent. Scroll a note to cycle duration.")
                        .size(10)
                        .color(theme::TEXT_3),
                );
        }
        MotifSource::Manual { notes, .. } => {
            let canvas_widget = manual_motif_canvas(definition_id, notes, definition.scale);
            let clear_btn = button(text("Clear motif").size(12).color(theme::TEXT_1))
                .padding([6, 10])
                .width(Length::Fill)
                .style(|_theme, status| theme::ghost_button_style(status))
                .on_press(Message::Compose(ComposeMessage::ChordInspector {
                    definition_id,
                    msg: ChordInspectorMsg::ClearManualMotif,
                }));

            col = col
                .push(Space::with_height(8))
                .push(canvas_widget)
                .push(Space::with_height(8))
                .push(
                    text("Click empty cell to add a note. Click the bottom row to insert a rest. Click the start of a note/rest to remove it. Right-click to toggle accent. Scroll on a note to cycle its duration.")
                        .size(10)
                        .color(theme::TEXT_3),
                )
                .push(Space::with_height(8))
                .push(clear_btn);
        }
    }

    col.into()
}

/// Small "MOTIF · N notes" preview card with scattered dashes — read-only,
/// purely for at-a-glance density feedback.
fn motif_preview_card<'a>(definition: &'a SectionDefinitionState) -> Element<'a, Message> {
    let note_count = match &definition.motif_source {
        MotifSource::Manual { notes, .. } => notes.len(),
        MotifSource::Generated(_) => {
            let n = definition.motif_source.params().motif_len;
            if n == 0 {
                4
            } else {
                n as usize
            }
        }
    };

    let header = row![
        text("MOTIF")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        Space::with_width(Length::Fill),
        text(format!("{note_count} notes"))
            .size(10)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_3),
    ]
    .align_y(alignment::Vertical::Center);

    // Scattered note dashes — scaled rectangles arranged in a grid via
    // a tiny canvas; deterministic on `seed` so it doesn't churn.
    let preview_canvas = canvas(MotifPreviewCanvas {
        seed: definition.motif_source.params().seed,
        note_count,
    })
    .width(Length::Fill)
    .height(Length::Fixed(56.0));

    iced::widget::container(column![header, Space::with_height(6), preview_canvas].spacing(0))
        .padding([10, 12])
        .width(Length::Fill)
        .style(|_theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_LG.into(),
            },
            ..Default::default()
        })
        .into()
}

struct MotifPreviewCanvas {
    seed: u64,
    note_count: usize,
}

impl<Message> iced::widget::canvas::Program<Message> for MotifPreviewCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry> {
        use iced::widget::canvas::{Frame, Path};
        use iced::{Point, Size};

        let mut frame = Frame::new(renderer, bounds.size());
        if self.note_count == 0 {
            return vec![frame.into_geometry()];
        }
        let n = self.note_count.clamp(1, 24);
        let cell_w = bounds.width / n as f32;
        let row_count = 5;
        let row_h = bounds.height / row_count as f32;
        let mut acc: u64 = self.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        for i in 0..n {
            // simple deterministic shuffle so preview varies with seed
            acc = acc
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let row = (acc >> 13) as usize % row_count;
            let dash_w = (cell_w - 4.0).max(2.0);
            let dash_h = 4.0;
            let x = i as f32 * cell_w + 2.0;
            let y = row as f32 * row_h + (row_h - dash_h) / 2.0;
            let path = Path::rounded_rectangle(
                Point::new(x, y),
                Size::new(dash_w, dash_h),
                2.0.into(),
            );
            frame.fill(&path, theme::ACCENT_SOFT);
        }
        vec![frame.into_geometry()]
    }
}

/// Two-state toggle row — label on the left, pill toggle on the right.
fn toggle_row<'a>(label: &'a str, on: bool, msg: Message) -> Element<'a, Message> {
    let track_color = if on { theme::ACCENT } else { theme::BG_3 };
    let knob_x = if on { 14.0 } else { 1.0 };

    let knob = iced::widget::container(Space::with_width(0))
        .width(12)
        .height(12)
        .style(|_theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(iced::Color::WHITE)),
            border: iced::Border {
                radius: 6.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });

    let track = iced::widget::container(
        row![Space::with_width(knob_x), knob]
            .align_y(alignment::Vertical::Center),
    )
    .width(28)
    .height(16)
    .center_y(Length::Fill)
    .style(move |_theme| iced::widget::container::Style {
        background: Some(iced::Background::Color(track_color)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    });

    let mouse = iced::widget::mouse_area(
        row![
            text(label).size(12).color(theme::TEXT_1),
            Space::with_width(Length::Fill),
            track,
        ]
        .align_y(alignment::Vertical::Center),
    )
    .on_press(msg);

    iced::widget::container(mouse)
        .width(Length::Fill)
        .padding([2, 0])
        .into()
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
