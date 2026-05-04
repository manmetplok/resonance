//! Unified right-hand inspector panel for the Compose view.
//!
//! One section, many lanes, each lane has an optional generator, the chord
//! lane is shared harmonic context. Selecting a lane updates this panel.

use iced::widget::{
    button, checkbox, column, container, pick_list, row, slider, text, text_input, tooltip, Space,
};
use iced::{alignment, Element, Length};

use resonance_audio::types::TrackId;
use resonance_music_theory::{
    BassMotifMode, BassMotifPhrase, BassStyle, ContourPreference, Degree, MelodyStyle, Mode,
    PitchClass, Scale, TableRegistry,
};

use crate::compose::drumroll::DrumrollMessage;
use crate::compose::messages::{ChordInspectorMsg, LaneInspectorMsg};
use crate::compose::{
    ComposeMessage, DrumVoiceMode, DrumrollViewState, LaneGeneratorKind, LaneGeneratorKindTag,
    SectionDefinitionState, SelectedLane,
};
use crate::message::*;
use crate::state::{InstrumentType, TrackState};
use crate::theme;

pub const PANEL_WIDTH: f32 = 240.0;

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

/// Wrapper for drum voice mode in pick_list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrumModePick {
    Manual,
    Euclidean,
    Motif,
}

impl DrumModePick {
    const ALL: [DrumModePick; 3] = [
        DrumModePick::Manual,
        DrumModePick::Euclidean,
        DrumModePick::Motif,
    ];
}

impl std::fmt::Display for DrumModePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DrumModePick::Manual => "Manual",
            DrumModePick::Euclidean => "Euclidean",
            DrumModePick::Motif => "Motif",
        })
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

// ===========================================================================
// Top-level inspector
// ===========================================================================

pub fn view<'a>(
    definition: &'a SectionDefinitionState,
    selected_lane: &'a SelectedLane,
    tracks: &'a [TrackState],
    drumroll_state: &'a DrumrollViewState,
    clip_id_for_drum: Option<u64>,
    table_registry: &'a TableRegistry,
) -> Element<'a, Message> {
    // Scale block — always at top, section-global.
    let scale = scale_block(definition);

    // Lane switcher
    let lane_switcher = lane_switcher_row(selected_lane, tracks);

    // Body — varies by selected lane
    let body: Element<'a, Message> = match selected_lane {
        SelectedLane::Chords => chord_body(definition, table_registry),
        SelectedLane::Instrument(track_id) => {
            let track = tracks.iter().find(|t| t.id == *track_id);
            match track {
                Some(t) => instrument_body(definition, t),
                None => text("Track not found")
                    .size(12)
                    .color(theme::TEXT_DIM)
                    .into(),
            }
        }
        SelectedLane::Drums(track_id) => {
            let track = tracks.iter().find(|t| t.id == *track_id);
            match track {
                Some(t) => drum_body(definition, t, drumroll_state, clip_id_for_drum),
                None => text("Track not found")
                    .size(12)
                    .color(theme::TEXT_DIM)
                    .into(),
            }
        }
    };

    let content = column![
        scale,
        Space::with_height(12),
        separator(),
        Space::with_height(8),
        lane_switcher,
        Space::with_height(8),
        separator(),
        Space::with_height(8),
        body,
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

// ===========================================================================
// Scale block (always visible)
// ===========================================================================

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

// ===========================================================================
// Lane switcher
// ===========================================================================

/// Lane names for the dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LanePick {
    lane: SelectedLane,
    label: String,
}

impl std::fmt::Display for LanePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

fn lane_switcher_row<'a>(
    selected: &'a SelectedLane,
    tracks: &'a [TrackState],
) -> Element<'a, Message> {
    let mut options = vec![LanePick {
        lane: SelectedLane::Chords,
        label: "Chords".to_string(),
    }];

    for t in tracks.iter().filter(|t| t.sub_track.is_none()) {
        let lane = if t.instrument_type == InstrumentType::Drum {
            SelectedLane::Drums(t.id)
        } else {
            SelectedLane::Instrument(t.id)
        };
        options.push(LanePick {
            lane,
            label: t.name.clone(),
        });
    }

    let current = options.iter().find(|o| o.lane == *selected).cloned();

    let picker = pick_list(options, current, |pick| {
        Message::Compose(ComposeMessage::SelectLane(pick.lane))
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    column![text("Lane").size(11).color(theme::TEXT_DIM), picker,]
        .spacing(4)
        .into()
}

// ===========================================================================
// Chord lane body
// ===========================================================================

fn chord_body<'a>(
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
    let motif = definition.motif;

    let heading = text("Section motif").size(13).color(theme::ACCENT);

    let complexity_slider = slider(0.0..=1.0, motif.complexity, move |v| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetMotifComplexity(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    let leap_slider = slider(0.0..=1.0, motif.leap_chance, move |v| {
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
        Some(MotifLenPick(motif.motif_len)),
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

    let seed_label = text(format!("Seed: 0x{:X}", motif.seed))
        .size(10)
        .color(theme::TEXT_DIM);

    column![
        heading,
        Space::with_height(4),
        label_with_info(
            format!("Complexity: {:.2}", motif.complexity),
            "How elaborate the motif is. Low values produce short, repetitive cells with simple rhythms; high values give longer cells, varied rhythms, and more developmental phrase transforms."
        ),
        complexity_slider,
        label_with_info(
            format!("Leap chance: {:.2}", motif.leap_chance),
            "Probability of an interval leap (3–7 semitones) versus a step (1–2 semitones) when building the motif. Higher = more angular, lower = more conjunct."
        ),
        leap_slider,
        Space::with_height(4),
        label_with_info(
            "Motif length",
            "Number of notes in the motif cell. Auto picks 2–6 based on Complexity."
        ),
        motif_len_picker,
        Space::with_height(8),
        regen_btn,
        Space::with_height(4),
        seed_label,
    ]
    .spacing(2)
    .into()
}

fn degree_picks_from(table_degrees: &[Degree]) -> Vec<DegreePick> {
    let mut picks = vec![DegreePick(None)];
    for d in table_degrees {
        picks.push(DegreePick(Some(*d)));
    }
    picks
}

// ===========================================================================
// Instrument lane body (bass / melody / pad / manual)
// ===========================================================================

fn instrument_body<'a>(
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

// ===========================================================================
// Drum lane body
// ===========================================================================

fn drum_body<'a>(
    definition: &'a SectionDefinitionState,
    track: &'a TrackState,
    drumroll_state: &'a DrumrollViewState,
    clip_id: Option<u64>,
) -> Element<'a, Message> {
    let definition_id = definition.id;
    let track_id = track.id;

    let heading = text(&track.name).size(13).color(theme::ACCENT);

    // Track name
    let name_input = text_input("Name", &track.name)
        .on_input(move |s| Message::Track(TrackMessage::SetTrackName(track_id, s)))
        .size(12)
        .padding([4, 6])
        .width(Length::Fill);

    // Steps per bar
    let steps_picker = pick_list(
        vec![4u32, 8, 16, 32],
        Some(drumroll_state.steps_per_bar),
        |n| Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetStepsPerBar(n))),
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    // Default velocity
    let vel_slider = slider(0.0..=1.0, drumroll_state.default_velocity, |v| {
        Message::Compose(ComposeMessage::Drumroll(
            DrumrollMessage::SetDefaultVelocity(v),
        ))
    })
    .step(0.01)
    .width(Length::Fill);

    // Per-pad euclidean section
    let selected_pad = drumroll_state.selected_pad;
    let pad_name: String = selected_pad
        .and_then(|i| drumroll_state.pad_map.get(i))
        .map(|p| p.name.to_string())
        .unwrap_or_else(|| "Click a pad row to select".to_string());

    // Get the drum lane config for this track
    let drum_config = definition
        .lane_generators
        .get(&track_id)
        .and_then(|cfg| match &cfg.kind {
            LaneGeneratorKind::Drum(dc) => Some(dc),
            _ => None,
        });

    let voice_mode = selected_pad.and_then(|pi| drum_config.and_then(|dc| dc.voices.get(&pi)));

    let current_mode_pick = match voice_mode {
        Some(DrumVoiceMode::Euclidean { .. }) => DrumModePick::Euclidean,
        Some(DrumVoiceMode::Motif) => DrumModePick::Motif,
        _ => DrumModePick::Manual,
    };

    let mode_picker_msg = selected_pad.map(|pad_index| {
        move |pick: DrumModePick| {
            let mode = match pick {
                DrumModePick::Manual => DrumVoiceMode::Manual,
                DrumModePick::Euclidean => DrumVoiceMode::Euclidean {
                    steps: 16,
                    hits: 4,
                    rotation: 0,
                },
                DrumModePick::Motif => DrumVoiceMode::Motif,
            };
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetDrumVoiceMode { pad_index, mode },
            })
        }
    });

    let mode_picker_el: Element<'a, Message> = if let Some(on_change) = mode_picker_msg {
        pick_list(
            DrumModePick::ALL.to_vec(),
            Some(current_mode_pick),
            on_change,
        )
        .text_size(12)
        .padding([4, 6])
        .width(Length::Fill)
        .into()
    } else {
        text("Select a pad first")
            .size(11)
            .color(theme::TEXT_DIM)
            .into()
    };

    // Euclidean params (if current voice is Euclidean)
    let euclid_controls: Element<'a, Message> = match (selected_pad, voice_mode) {
        (
            Some(pad_index),
            Some(DrumVoiceMode::Euclidean {
                steps,
                hits,
                rotation,
            }),
        ) => {
            let steps_input = text_input("Steps", &steps.to_string())
                .on_input(move |s| {
                    let val = s.parse::<u32>().unwrap_or(16).max(1);
                    Message::Compose(ComposeMessage::LaneInspector {
                        definition_id,
                        track_id,
                        msg: LaneInspectorMsg::SetDrumEuclidSteps {
                            pad_index,
                            steps: val,
                        },
                    })
                })
                .size(12)
                .padding([4, 6])
                .width(Length::Fill);
            let hits_input = text_input("Hits", &hits.to_string())
                .on_input(move |s| {
                    let val = s.parse::<u32>().unwrap_or(4);
                    Message::Compose(ComposeMessage::LaneInspector {
                        definition_id,
                        track_id,
                        msg: LaneInspectorMsg::SetDrumEuclidHits {
                            pad_index,
                            hits: val,
                        },
                    })
                })
                .size(12)
                .padding([4, 6])
                .width(Length::Fill);
            let rot_input = text_input("Rotation", &rotation.to_string())
                .on_input(move |s| {
                    let val = s.parse::<i32>().unwrap_or(0);
                    Message::Compose(ComposeMessage::LaneInspector {
                        definition_id,
                        track_id,
                        msg: LaneInspectorMsg::SetDrumEuclidRotation {
                            pad_index,
                            rotation: val,
                        },
                    })
                })
                .size(12)
                .padding([4, 6])
                .width(Length::Fill);

            // Apply button: generates euclidean pattern for this pad
            let can_apply = clip_id.is_some();
            let apply_msg = if can_apply {
                Some(Message::Compose(ComposeMessage::Drumroll(
                    DrumrollMessage::GenerateEuclideanPad {
                        clip_id: clip_id.unwrap(),
                        pad_index,
                    },
                )))
            } else {
                None
            };
            let mut apply_btn = button(text("Apply").size(12))
                .padding([4, 10])
                .width(Length::Fill)
                .style(|_theme, status| theme::transport_button_style(status));
            if let Some(m) = apply_msg {
                apply_btn = apply_btn.on_press(m);
            }

            column![
                text("Steps").size(10).color(theme::TEXT_DIM),
                steps_input,
                text("Hits").size(10).color(theme::TEXT_DIM),
                hits_input,
                text("Rotation").size(10).color(theme::TEXT_DIM),
                rot_input,
                Space::with_height(4),
                apply_btn,
            ]
            .spacing(2)
            .into()
        }
        (Some(pad_index), Some(DrumVoiceMode::Motif)) => {
            // Motif voices have no per-voice knobs — the rhythm comes
            // straight from the section's shared motif, so the Apply
            // button is the only control needed. Edits to the motif's
            // seed / complexity propagate via `propagate_motif_change`.
            let can_apply = clip_id.is_some();
            let apply_msg = if can_apply {
                Some(Message::Compose(ComposeMessage::Drumroll(
                    DrumrollMessage::GenerateMotifPad {
                        clip_id: clip_id.unwrap(),
                        pad_index,
                    },
                )))
            } else {
                None
            };
            let mut apply_btn = button(text("Apply").size(12))
                .padding([4, 10])
                .width(Length::Fill)
                .style(|_theme, status| theme::transport_button_style(status));
            if let Some(m) = apply_msg {
                apply_btn = apply_btn.on_press(m);
            }
            column![
                text("Plays the section's shared motif rhythm.")
                    .size(10)
                    .color(theme::TEXT_DIM),
                Space::with_height(4),
                apply_btn,
            ]
            .spacing(2)
            .into()
        }
        _ => Space::with_height(0).into(),
    };

    // Clear pad button
    let clear_msg = match (selected_pad, clip_id) {
        (Some(pad_index), Some(cid)) => Some(Message::Compose(ComposeMessage::Drumroll(
            DrumrollMessage::ClearPad {
                clip_id: cid,
                pad_index,
            },
        ))),
        _ => None,
    };
    let mut clear_btn = button(text("Clear pad").size(12))
        .padding([4, 10])
        .width(Length::Fill)
        .style(|_theme, status| theme::transport_button_style(status));
    if let Some(m) = clear_msg {
        clear_btn = clear_btn.on_press(m);
    }

    // Humanize section (kept from drumroll/controls.rs)
    let humanize = humanize_block(drumroll_state, clip_id);

    column![
        heading,
        Space::with_height(6),
        text("Name").size(10).color(theme::TEXT_DIM),
        name_input,
        Space::with_height(8),
        text("Steps per bar").size(10).color(theme::TEXT_DIM),
        steps_picker,
        text(format!("Velocity: {:.2}", drumroll_state.default_velocity))
            .size(10)
            .color(theme::TEXT_DIM),
        vel_slider,
        Space::with_height(10),
        text("Selected pad").size(10).color(theme::TEXT_DIM),
        text(pad_name.clone()).size(13).color(theme::TEXT),
        Space::with_height(4),
        text("Mode").size(10).color(theme::TEXT_DIM),
        mode_picker_el,
        Space::with_height(4),
        euclid_controls,
        Space::with_height(4),
        clear_btn,
        Space::with_height(12),
        humanize,
    ]
    .spacing(2)
    .into()
}

fn humanize_block<'a>(state: &'a DrumrollViewState, clip_id: Option<u64>) -> Element<'a, Message> {
    use crate::compose::drumroll::{AccentPattern, HumanizeScope};

    let hum_vel_slider = slider(0.0..=1.0, state.humanize_velocity, |v| {
        Message::Compose(ComposeMessage::Drumroll(
            DrumrollMessage::SetHumanizeVelocity(v),
        ))
    })
    .step(0.01)
    .width(Length::Fill);

    let hum_timing_slider = slider(0.0..=1.0, state.humanize_timing, |v| {
        Message::Compose(ComposeMessage::Drumroll(
            DrumrollMessage::SetHumanizeTiming(v),
        ))
    })
    .step(0.01)
    .width(Length::Fill);

    let hum_swing_slider = slider(0.0..=1.0, state.humanize_swing, |v| {
        Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetHumanizeSwing(
            v,
        )))
    })
    .step(0.01)
    .width(Length::Fill);

    let accent_picker = pick_list(
        AccentPattern::ALL.to_vec(),
        Some(state.humanize_accent),
        |p| {
            Message::Compose(ComposeMessage::Drumroll(
                DrumrollMessage::SetHumanizeAccent(p),
            ))
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let accent_slider = slider(0.0..=1.0, state.humanize_accent_amount, |v| {
        Message::Compose(ComposeMessage::Drumroll(
            DrumrollMessage::SetHumanizeAccentAmount(v),
        ))
    })
    .step(0.01)
    .width(Length::Fill);

    let scope_picker = pick_list(
        HumanizeScope::ALL.to_vec(),
        Some(state.humanize_scope),
        |s| {
            Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetHumanizeScope(
                s,
            )))
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let humanize_msg = clip_id.map(|cid| {
        Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::ApplyHumanize {
            clip_id: cid,
        }))
    });
    let mut humanize_btn = button(text("Humanize").size(12))
        .padding([4, 10])
        .width(Length::Fill)
        .style(|_theme, status| theme::transport_button_style(status));
    if let Some(m) = humanize_msg {
        humanize_btn = humanize_btn.on_press(m);
    }

    column![
        text("Humanize").size(11).color(theme::ACCENT),
        Space::with_height(4),
        text(format!("Velocity jitter: {:.2}", state.humanize_velocity))
            .size(10)
            .color(theme::TEXT_DIM),
        hum_vel_slider,
        text(format!("Timing jitter: {:.2}", state.humanize_timing))
            .size(10)
            .color(theme::TEXT_DIM),
        hum_timing_slider,
        text(format!("Swing: {:.2}", state.humanize_swing))
            .size(10)
            .color(theme::TEXT_DIM),
        hum_swing_slider,
        Space::with_height(4),
        text("Accent pattern").size(10).color(theme::TEXT_DIM),
        accent_picker,
        text(format!(
            "Accent amount: {:.2}",
            state.humanize_accent_amount
        ))
        .size(10)
        .color(theme::TEXT_DIM),
        accent_slider,
        Space::with_height(4),
        text("Scope").size(10).color(theme::TEXT_DIM),
        scope_picker,
        Space::with_height(6),
        humanize_btn,
    ]
    .spacing(2)
    .into()
}

// ===========================================================================
// Helpers
// ===========================================================================

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

/// Small info-icon (Font Awesome circle-info) that shows `info` on hover.
/// Use via [`label_with_info`] to pair a control label with its explanation.
fn info_icon<'a>(info: &'static str) -> Element<'a, Message> {
    let icon = container(theme::icon(theme::fa::CIRCLE_INFO).size(10).color(theme::TEXT_DIM))
        .padding([0, 2]);
    let tip = container(text(info).size(11).color(theme::TEXT))
        .max_width(220.0)
        .padding(8)
        .style(|_theme: &iced::Theme| container::Style {
            text_color: Some(theme::TEXT),
            background: Some(iced::Background::Color(theme::PANEL_DARK)),
            border: iced::Border {
                color: theme::SEPARATOR,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        });
    tooltip(icon, tip, tooltip::Position::Top).gap(4).into()
}

/// Standard small dim label paired with a hoverable info icon. Drop-in
/// replacement for `text(label).size(11).color(theme::TEXT_DIM)` when
/// the option deserves a one-sentence explanation.
fn label_with_info<'a>(label: impl Into<String>, info: &'static str) -> Element<'a, Message> {
    let label_text = text(label.into()).size(11).color(theme::TEXT_DIM);
    row![label_text, Space::with_width(4), info_icon(info)]
        .align_y(alignment::Vertical::Center)
        .into()
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
