//! Section + placement CRUD plus the new/edit-section dialog form
//! handlers. The dialog confirmations re-enter the parent dispatcher so
//! they share validation with the direct CreateSection / RenameSection /
//! ResizeSection paths.

use std::collections::HashMap;

use resonance_audio::types::{AudioCommand, TICKS_PER_QUARTER_NOTE};

use super::handle as dispatch;
use crate::compose::invariants::{chord_fits_in_section, placement_overlaps};
use crate::compose::{
    ComposeMessage, ComposeState, EditSectionForm, NewSectionForm, SectionDefinitionState,
    SectionPlacementState,
};

/// Stock section names offered in the order Intro → Verse → Chorus → Bridge
/// → Outro. Whichever is not yet present in the project is used as the
/// initial value of the new-section form.
const STOCK_SECTION_NAMES: &[&str] = &["Intro", "Verse", "Chorus", "Bridge", "Outro"];

/// Section colors cycled through by the auto-rotating palette. Indexed by
/// `definitions.len()` modulo the palette size.
const SECTION_PALETTE: &[[u8; 3]] = &[
    [0x5b, 0x8d, 0xef], // blue
    [0xef, 0x8d, 0x5b], // orange
    [0x8d, 0xef, 0x5b], // green
    [0xef, 0x5b, 0x8d], // pink
    [0xbd, 0x8d, 0xef], // purple
    [0xef, 0xef, 0x5b], // yellow
];

fn default_section_name(state: &ComposeState) -> String {
    let existing: std::collections::HashSet<&str> =
        state.definitions.iter().map(|d| d.name.as_str()).collect();
    for name in STOCK_SECTION_NAMES {
        if !existing.contains(*name) {
            return (*name).to_string();
        }
    }
    format!("Section {}", state.definitions.len() + 1)
}

fn next_default_color(state: &ComposeState) -> [u8; 3] {
    SECTION_PALETTE[state.definitions.len() % SECTION_PALETTE.len()]
}

/// Lowest start_bar at which a section of `length_bars` would not overlap
/// any existing placement. Capped at 10_000 to stop the search if the
/// timeline is somehow saturated.
pub(super) fn first_free_bar(state: &ComposeState, length_bars: u32) -> u32 {
    let mut candidate = 0u32;
    loop {
        if !placement_overlaps(
            &state.placements,
            &state.definitions,
            candidate,
            length_bars,
            None,
        ) {
            return candidate;
        }
        candidate += 1;
        if candidate > 10_000 {
            return candidate;
        }
    }
}

pub(super) fn handle_create_midi_clip(
    r: &mut crate::Resonance,
    track_id: resonance_audio::types::TrackId,
    start_sample: u64,
    length_bars: u32,
    time_sig_num: u8,
) {
    let duration_ticks = length_bars as u64 * time_sig_num as u64 * TICKS_PER_QUARTER_NOTE;
    r.engine.send(AudioCommand::CreateMidiClip {
        track_id,
        start_sample,
        duration_ticks,
        name: "MIDI Clip".to_string(),
    });
}

// ---------------------------------------------------------------------------
// Create-section dialog
// ---------------------------------------------------------------------------

pub(super) fn handle_open_create_dialog(r: &mut crate::Resonance) {
    r.compose.edit_section_form = None;
    r.compose.new_section_form = Some(NewSectionForm {
        name: default_section_name(&r.compose),
        length_input: "8".to_string(),
        color: next_default_color(&r.compose),
    });
    r.compose.last_error = None;
}

pub(super) fn handle_cancel_create_dialog(r: &mut crate::Resonance) {
    r.compose.new_section_form = None;
    r.compose.last_error = None;
}

pub(super) fn handle_set_new_name(r: &mut crate::Resonance, name: String) {
    if let Some(form) = r.compose.new_section_form.as_mut() {
        form.name = name;
    }
}

pub(super) fn handle_set_new_length(r: &mut crate::Resonance, input: String) {
    if let Some(form) = r.compose.new_section_form.as_mut() {
        form.length_input = input.chars().filter(|c| c.is_ascii_digit()).collect();
    }
}

pub(super) fn handle_confirm_create(r: &mut crate::Resonance) {
    let Some(form) = r.compose.new_section_form.clone() else {
        return;
    };
    let name = form.name.trim().to_string();
    if name.is_empty() {
        r.compose.last_error = Some("Section name cannot be empty".into());
        return;
    }
    let length_bars: u32 = match form.length_input.parse() {
        Ok(n) if n > 0 => n,
        _ => {
            r.compose.last_error =
                Some("Section length must be a positive whole number of bars".into());
            return;
        }
    };
    r.compose.new_section_form = None;
    dispatch(
        r,
        ComposeMessage::CreateSection {
            name,
            length_bars,
            color: form.color,
        },
    );
}

// ---------------------------------------------------------------------------
// Edit-section dialog
// ---------------------------------------------------------------------------

pub(super) fn handle_open_edit_dialog(r: &mut crate::Resonance, definition_id: u64) {
    let snapshot = match r.compose.find_definition(definition_id) {
        Some(def) => (def.name.clone(), def.length_bars),
        None => return,
    };
    r.compose.new_section_form = None;
    r.compose.edit_section_form = Some(EditSectionForm {
        definition_id,
        name: snapshot.0,
        length_input: snapshot.1.to_string(),
    });
    r.compose.last_error = None;
}

pub(super) fn handle_cancel_edit_dialog(r: &mut crate::Resonance) {
    r.compose.edit_section_form = None;
    r.compose.last_error = None;
}

pub(super) fn handle_set_edit_name(r: &mut crate::Resonance, name: String) {
    if let Some(form) = r.compose.edit_section_form.as_mut() {
        form.name = name;
    }
}

pub(super) fn handle_set_edit_length(r: &mut crate::Resonance, input: String) {
    if let Some(form) = r.compose.edit_section_form.as_mut() {
        form.length_input = input.chars().filter(|c| c.is_ascii_digit()).collect();
    }
}

pub(super) fn handle_confirm_edit(r: &mut crate::Resonance) {
    let Some(form) = r.compose.edit_section_form.clone() else {
        return;
    };
    let name = form.name.trim().to_string();
    if name.is_empty() {
        r.compose.last_error = Some("Section name cannot be empty".into());
        return;
    }
    let length_bars: u32 = match form.length_input.parse() {
        Ok(n) if n > 0 => n,
        _ => {
            r.compose.last_error =
                Some("Section length must be a positive whole number of bars".into());
            return;
        }
    };
    dispatch(
        r,
        ComposeMessage::RenameSection {
            definition_id: form.definition_id,
            name,
        },
    );
    dispatch(
        r,
        ComposeMessage::ResizeSection {
            definition_id: form.definition_id,
            length_bars,
        },
    );
    if r.compose.last_error.is_none() {
        r.compose.edit_section_form = None;
    }
}

pub(super) fn handle_cycle_color(r: &mut crate::Resonance, definition_id: u64) {
    let current = r
        .compose
        .find_definition(definition_id)
        .map(|d| d.color)
        .unwrap_or([0, 0, 0]);
    let next_index = SECTION_PALETTE
        .iter()
        .position(|c| *c == current)
        .map(|i| (i + 1) % SECTION_PALETTE.len())
        .unwrap_or(0);
    let next_color = SECTION_PALETTE[next_index];
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.color = next_color;
        r.compose.last_error = None;
    }
}

// ---------------------------------------------------------------------------
// Section CRUD
// ---------------------------------------------------------------------------

pub(super) fn handle_create(
    r: &mut crate::Resonance,
    name: String,
    length_bars: u32,
    color: [u8; 3],
) {
    if length_bars == 0 {
        r.compose.last_error = Some("Section length must be at least 1 bar".into());
        return;
    }
    let id = r.compose.fresh_id();
    r.compose.definitions.push(SectionDefinitionState {
        id,
        name,
        color,
        length_bars,
        chords: Vec::new(),
        scale: None,
        progression_seed: id.wrapping_mul(0x9E3779B97F4A7C15),
        generate_params: crate::compose::GenerateParams::default(),
        generator_spec: None,
        generator_seed: id.wrapping_mul(0x517CC1B727220A95),
        generated_material: None,
        lane_generators: HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: resonance_music_theory::MotifSource::Generated(
            resonance_music_theory::MotifParams {
                seed: id.wrapping_mul(0x6C62272E07BB0142),
                ..resonance_music_theory::MotifParams::default()
            },
        ),
    });
    let start_bar = first_free_bar(&r.compose, length_bars);
    let placement_id = r.compose.fresh_id();
    r.compose.placements.push(SectionPlacementState {
        id: placement_id,
        definition_id: id,
        start_bar,
    });
    r.compose.placements.sort_by_key(|p| p.start_bar);
    r.compose.selected_placement_id = Some(placement_id);
    r.compose.last_error = None;
}

pub(super) fn handle_rename(r: &mut crate::Resonance, definition_id: u64, name: String) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.name = name;
        r.compose.last_error = None;
    }
}

pub(super) fn handle_resize(
    r: &mut crate::Resonance,
    definition_id: u64,
    length_bars: u32,
    time_sig_num: u8,
) {
    if length_bars == 0 {
        r.compose.last_error = Some("Section length must be at least 1 bar".into());
        return;
    }
    let old_length = match r.compose.find_definition(definition_id) {
        Some(d) => d.length_bars,
        None => return,
    };
    if length_bars > old_length {
        let snapshot = r.compose.placements.clone();
        let definitions = r.compose.definitions.clone();
        for p in snapshot.iter().filter(|p| p.definition_id == definition_id) {
            let others: Vec<SectionPlacementState> =
                snapshot.iter().filter(|q| q.id != p.id).cloned().collect();
            if placement_overlaps(&others, &definitions, p.start_bar, length_bars, None) {
                r.compose.last_error =
                    Some("Cannot grow section: a placement would overlap a neighbour".into());
                return;
            }
        }
    }
    let chords_fit = r
        .compose
        .find_definition(definition_id)
        .map(|d| {
            d.chords.iter().all(|c| {
                chord_fits_in_section(c.start_beat, c.duration_beats, length_bars, time_sig_num)
            })
        })
        .unwrap_or(true);
    if !chords_fit {
        r.compose.last_error =
            Some("Cannot shrink section: chords would fall outside the new length".into());
        return;
    }
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.length_bars = length_bars;
        r.compose.last_error = None;
    }
}

pub(super) fn handle_set_scale(
    r: &mut crate::Resonance,
    definition_id: u64,
    scale: Option<resonance_music_theory::Scale>,
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.scale = scale;
        r.compose.last_error = None;
    }
}

pub(super) fn handle_delete_definition(r: &mut crate::Resonance, definition_id: u64) {
    let in_use = r
        .compose
        .placements
        .iter()
        .any(|p| p.definition_id == definition_id);
    if in_use {
        r.compose.last_error =
            Some("Cannot delete a section while placements still reference it".into());
        return;
    }
    r.compose.definitions.retain(|d| d.id != definition_id);
    r.compose.last_error = None;
}

// ---------------------------------------------------------------------------
// Placement CRUD + selection
// ---------------------------------------------------------------------------

pub(super) fn handle_place(r: &mut crate::Resonance, definition_id: u64, start_bar: u32) {
    let length_bars = match r.compose.find_definition(definition_id) {
        Some(d) => d.length_bars,
        None => return,
    };
    if placement_overlaps(
        &r.compose.placements,
        &r.compose.definitions,
        start_bar,
        length_bars,
        None,
    ) {
        r.compose.last_error = Some("Placement would overlap an existing section".into());
        return;
    }
    let id = r.compose.fresh_id();
    r.compose.placements.push(SectionPlacementState {
        id,
        definition_id,
        start_bar,
    });
    r.compose.placements.sort_by_key(|p| p.start_bar);
    r.compose.selected_placement_id = Some(id);
    r.compose.last_error = None;
}

pub(super) fn handle_delete_placement(r: &mut crate::Resonance, placement_id: u64) {
    r.compose.placements.retain(|p| p.id != placement_id);
    if r.compose.selected_placement_id == Some(placement_id) {
        r.compose.selected_placement_id = r.compose.placements.first().map(|p| p.id);
    }
    r.compose.last_error = None;
}

pub(super) fn handle_select_placement(r: &mut crate::Resonance, placement_id: u64) {
    if r.compose.find_placement(placement_id).is_some() {
        r.compose.selected_placement_id = Some(placement_id);
        r.compose.selected_chord_id = None;
    }
}
