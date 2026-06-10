//! Instrument-lane inspector body: generator picker + the bass / melody /
//! pad parameter panels.

use std::collections::HashMap;

use iced::widget::{button, column, container, pick_list, row, text, text_input, Space};
use iced::{alignment, Element, Length};

use resonance_audio::types::{TrackId, TrackType};

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::{
    ComposeMessage, LaneGeneratorKind, LaneGeneratorKindTag, SectionDefinitionState,
};
use crate::message::*;
use crate::state::TrackState;
use crate::theme;

mod bass;
mod melody;
mod pad;

use bass::bass_controls;
use melody::melody_controls;
use pad::pad_controls;

/// Wrapper for LaneGeneratorKindTag in pick_list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GeneratorPick(LaneGeneratorKindTag);

impl GeneratorPick {
    /// Generator options for a regular instrument (synth) track. The
    /// vocal generator drives the SVS pipeline and only makes sense on
    /// a vocal track, so it's intentionally absent here.
    const INSTRUMENT: [GeneratorPick; 4] = [
        GeneratorPick(LaneGeneratorKindTag::Manual),
        GeneratorPick(LaneGeneratorKindTag::Bass),
        GeneratorPick(LaneGeneratorKindTag::Melody),
        GeneratorPick(LaneGeneratorKindTag::Pad),
    ];

    /// Generator options for a vocal track. Vocal tracks only ever
    /// drive the vocal generator — no Bass/Melody/Pad fit the SVS
    /// pipeline, and "Manual" would skip generation entirely (which is
    /// not yet supported on the vocal lane).
    const VOCAL: [GeneratorPick; 1] = [GeneratorPick(LaneGeneratorKindTag::Vocal)];
}

impl std::fmt::Display for GeneratorPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            LaneGeneratorKindTag::Manual => "Manual",
            LaneGeneratorKindTag::Bass => "Bass",
            LaneGeneratorKindTag::Melody => "Melody",
            LaneGeneratorKindTag::Pad => "Pad",
            LaneGeneratorKindTag::Vocal => "Vocal",
        })
    }
}

/// MIDI note number → name for pick_list display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NotePick(pub(super) u8);

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

// ---------------------------------------------------------------------------
// Cached `NotePick` option vectors for the register/range dropdowns.
//
// `pick_list` takes its options by value, so building `(36..=84).map(NotePick)`
// every frame allocates a fresh `Vec<NotePick>` per dropdown per repaint
// (and the pad/melody panels show two register pickers each). These
// statics are populated on first access and reused thereafter. See
// view-layer performance memory.
// ---------------------------------------------------------------------------

/// Pad/melody "register low" range: C2..=C6 (MIDI 36..=84).
pub(super) fn register_low_options() -> &'static [NotePick] {
    static V: std::sync::OnceLock<Vec<NotePick>> = std::sync::OnceLock::new();
    V.get_or_init(|| (36..=84).map(NotePick).collect())
}

/// Pad/melody "register high" range: C2..=C7 (MIDI 36..=96).
pub(super) fn register_high_options() -> &'static [NotePick] {
    static V: std::sync::OnceLock<Vec<NotePick>> = std::sync::OnceLock::new();
    V.get_or_init(|| (36..=96).map(NotePick).collect())
}

/// Bass "base note" range: C1..=E3 (MIDI 16..=52).
pub(super) fn bass_base_note_options() -> &'static [NotePick] {
    static V: std::sync::OnceLock<Vec<NotePick>> = std::sync::OnceLock::new();
    V.get_or_init(|| (16..=52).map(NotePick).collect())
}

/// Note value pick for melody note duration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NoteValuePick(pub(super) u32, pub(super) &'static str);

impl std::fmt::Display for NoteValuePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.1)
    }
}

/// Phrase length pick for motif generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PhraseLenPick(pub(super) u8);

impl std::fmt::Display for PhraseLenPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} chords", self.0)
    }
}

pub(in crate::view::compose::lane_inspector) fn instrument_body<'a>(
    definition: &'a SectionDefinitionState,
    track: &'a TrackState,
    vocal_bulk_lyrics: &'a HashMap<(u64, TrackId), iced::widget::text_editor::Content>,
    collapsed_panels: &std::collections::HashSet<crate::compose::RailPanelKey>,
) -> Element<'a, Message> {
    let definition_id = definition.id;
    let track_id = track.id;

    // Track name appears twice elsewhere (the EDITING TRACK header
    // above the inspector body and the editable Name text input
    // below); a third headline here was just visual noise.

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
            LaneGeneratorKind::Vocal(_) => GeneratorPick(LaneGeneratorKindTag::Vocal),
        },
        None => GeneratorPick(LaneGeneratorKindTag::Manual),
    };

    // Filter the picker options by track type — vocal tracks only run
    // the vocal generator, instrument tracks never do.
    let gen_options: &[GeneratorPick] = match track.track_type {
        TrackType::Vocal => &GeneratorPick::VOCAL,
        _ => &GeneratorPick::INSTRUMENT,
    };
    let gen_picker = pick_list(
        gen_options.to_vec(),
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

    // Generator-specific controls. Bass / Melody / Pad parameter panels
    // are collapsible cards keyed per track; their body isn't even
    // built while folded. Vocal lanes get per-card collapse inside
    // `vocal_controls` instead of one big fold here.
    let gen_collapsed = collapsed_panels
        .contains(&crate::compose::RailPanelKey::InstrumentGenerator(track_id));
    let gen_controls: Element<'a, Message> = match definition.lane_generators.get(&track_id) {
        Some(cfg) => match &cfg.kind {
            LaneGeneratorKind::Bass(params) => generator_panel(
                "Bass generator",
                track_id,
                gen_collapsed,
                (!gen_collapsed).then(|| bass_controls(definition_id, track_id, params)),
            ),
            LaneGeneratorKind::Melody(params) => generator_panel(
                "Melody generator",
                track_id,
                gen_collapsed,
                (!gen_collapsed).then(|| melody_controls(definition_id, track_id, params)),
            ),
            LaneGeneratorKind::Pad(params) => generator_panel(
                "Pad generator",
                track_id,
                gen_collapsed,
                (!gen_collapsed).then(|| pad_controls(definition_id, track_id, params)),
            ),
            LaneGeneratorKind::Drum(_) => manual_hint(),
            LaneGeneratorKind::Vocal(params) => super::vocal::vocal_controls(
                definition_id,
                track_id,
                params,
                cfg.seed,
                vocal_bulk_lyrics.get(&(definition_id, track_id)),
                collapsed_panels,
            ),
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
        text("Name").size(11).color(theme::TEXT_DIM),
        name_input,
        Space::new().height(8),
        text("Generator").size(11).color(theme::TEXT_DIM),
        gen_picker,
        Space::new().height(8),
        gen_controls,
    ]
    .spacing(2);

    if has_generator {
        col = col
            .push(Space::new().height(8))
            .push(regen_btn)
            .push(Space::new().height(4))
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

/// Collapsible wrapper around the Bass / Melody / Pad parameter panel —
/// warm dot + title header (matching the vocal rail cards) with the
/// shared collapse caret. Keyed per track so folding one lane's
/// generator doesn't fold another's. `body` is `None` when folded.
fn generator_panel<'a>(
    title: &'static str,
    track_id: TrackId,
    collapsed: bool,
    body: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let dot = container(Space::new().width(Length::Fixed(6.0)).height(Length::Fixed(6.0)))
        .style(|_| container::Style {
            background: Some(iced::Background::Color(theme::WARM)),
            border: iced::Border {
                color: theme::WARM,
                width: 0.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        });
    let title_left: Element<'a, Message> = row![
        dot,
        text(title)
            .size(12.5)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::WARM),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center)
    .into();
    let head = crate::view::compose::lane_inspector::rail_panel_header(
        title_left,
        None,
        crate::compose::RailPanelKey::InstrumentGenerator(track_id),
        collapsed,
    );
    match body {
        Some(body) => column![head, Space::new().height(8), body].spacing(0).into(),
        None => head,
    }
}
