use resonance_audio::types::{AudioCommand, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::Chord;

use crate::compose::invariants::{chord_fits_in_section, chord_overlaps, placement_overlaps};
use crate::compose::{
    ChordState, ComposeMessage, ComposeState, EditSectionForm, NewSectionForm,
    SectionDefinitionState, SectionPlacementState,
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

pub fn handle(r: &mut crate::Resonance, msg: ComposeMessage) {
    // Convenience: pull time_sig_num up-front for chord-fit checks.
    let time_sig_num = r.transport.time_sig_num;

    match msg {
        ComposeMessage::Drumroll(m) => {
            crate::update::drumroll::handle(r, m);
        }

        ComposeMessage::CreateMidiClipInSection {
            track_id,
            start_sample,
            length_bars,
        } => {
            let duration_ticks =
                length_bars as u64 * time_sig_num as u64 * TICKS_PER_QUARTER_NOTE;
            r.engine.send(AudioCommand::CreateMidiClip {
                track_id,
                start_sample,
                duration_ticks,
                name: "MIDI Clip".to_string(),
            });
        }

        ComposeMessage::OpenCreateSectionDialog => {
            r.compose.edit_section_form = None;
            r.compose.new_section_form = Some(NewSectionForm {
                name: default_section_name(&r.compose),
                length_input: "8".to_string(),
                color: next_default_color(&r.compose),
            });
            r.compose.last_error = None;
        }

        ComposeMessage::OpenEditSectionDialog { definition_id } => {
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

        ComposeMessage::CancelEditSectionDialog => {
            r.compose.edit_section_form = None;
            r.compose.last_error = None;
        }

        ComposeMessage::SetEditSectionName(name) => {
            if let Some(form) = r.compose.edit_section_form.as_mut() {
                form.name = name;
            }
        }

        ComposeMessage::SetEditSectionLength(input) => {
            if let Some(form) = r.compose.edit_section_form.as_mut() {
                form.length_input = input.chars().filter(|c| c.is_ascii_digit()).collect();
            }
        }

        ComposeMessage::ConfirmEditSection => {
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
            // Apply rename via dedicated handler so invariants stay centralized.
            handle(
                r,
                ComposeMessage::RenameSection {
                    definition_id: form.definition_id,
                    name,
                },
            );
            // Attempt resize next; failures surface in last_error and keep
            // the form open so the user can adjust.
            handle(
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

        ComposeMessage::CycleSectionColor { definition_id } => {
            // Rotate through the fixed palette so the user can recolor
            // without needing a color picker widget.
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

        ComposeMessage::CancelCreateSectionDialog => {
            r.compose.new_section_form = None;
            r.compose.last_error = None;
        }

        ComposeMessage::SetNewSectionName(name) => {
            if let Some(form) = r.compose.new_section_form.as_mut() {
                form.name = name;
            }
        }

        ComposeMessage::SetNewSectionLength(input) => {
            if let Some(form) = r.compose.new_section_form.as_mut() {
                // Accept only digits so the field cannot hold garbage.
                form.length_input = input.chars().filter(|c| c.is_ascii_digit()).collect();
            }
        }

        ComposeMessage::ConfirmCreateSection => {
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
            // Delegate to the existing CreateSection handler so the
            // auto-placement + id bookkeeping stay in one place.
            handle(
                r,
                ComposeMessage::CreateSection {
                    name,
                    length_bars,
                    color: form.color,
                },
            );
        }

        ComposeMessage::CreateSection { name, length_bars, color } => {
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
            });
            // Auto-place the new definition at the first free slot so the user
            // immediately sees the section on the strip.
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

        ComposeMessage::RenameSection { definition_id, name } => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.name = name;
                r.compose.last_error = None;
            }
        }

        ComposeMessage::ResizeSection { definition_id, length_bars } => {
            if length_bars == 0 {
                r.compose.last_error = Some("Section length must be at least 1 bar".into());
                return;
            }
            // Resizing a definition affects every placement that references it.
            // Check that none of them would overlap a neighbor at the new length.
            let old_length = match r.compose.find_definition(definition_id) {
                Some(d) => d.length_bars,
                None => return,
            };
            if length_bars > old_length {
                let snapshot = r.compose.placements.clone();
                let definitions = r.compose.definitions.clone();
                for p in snapshot.iter().filter(|p| p.definition_id == definition_id) {
                    // Temporarily treat this placement as if it were the new
                    // length and see if it collides with any other placement.
                    let others: Vec<SectionPlacementState> = snapshot
                        .iter()
                        .filter(|q| q.id != p.id)
                        .cloned()
                        .collect();
                    if placement_overlaps(&others, &definitions, p.start_bar, length_bars, None) {
                        r.compose.last_error = Some(
                            "Cannot grow section: a placement would overlap a neighbour".into(),
                        );
                        return;
                    }
                }
            }
            // Also ensure existing chords still fit in the (possibly smaller) section.
            let chords_fit = r
                .compose
                .find_definition(definition_id)
                .map(|d| {
                    d.chords
                        .iter()
                        .all(|c| chord_fits_in_section(c.start_beat, c.duration_beats, length_bars, time_sig_num))
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

        ComposeMessage::SetSectionScale { definition_id, scale } => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.scale = scale;
                r.compose.last_error = None;
            }
        }

        ComposeMessage::DeleteSectionDefinition { definition_id } => {
            let in_use = r
                .compose
                .placements
                .iter()
                .any(|p| p.definition_id == definition_id);
            if in_use {
                r.compose.last_error = Some(
                    "Cannot delete a section while placements still reference it".into(),
                );
                return;
            }
            r.compose.definitions.retain(|d| d.id != definition_id);
            r.compose.last_error = None;
        }

        ComposeMessage::PlaceSection { definition_id, start_bar } => {
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

        ComposeMessage::DeleteSectionPlacement { placement_id } => {
            r.compose.placements.retain(|p| p.id != placement_id);
            if r.compose.selected_placement_id == Some(placement_id) {
                r.compose.selected_placement_id = r.compose.placements.first().map(|p| p.id);
            }
            r.compose.last_error = None;
        }

        ComposeMessage::SelectSectionPlacement { placement_id } => {
            if r.compose.find_placement(placement_id).is_some() {
                r.compose.selected_placement_id = Some(placement_id);
                r.compose.selected_chord_id = None;
            }
        }

        ComposeMessage::SelectChord { chord_id } => {
            r.compose.selected_chord_id = Some(chord_id);
        }

        ComposeMessage::ClearChordSelection => {
            r.compose.selected_chord_id = None;
        }

        ComposeMessage::SelectInstrumentForDetails { track_id } => {
            // Toggle: clicking the same track's name again clears details.
            r.compose.details_track_id = if r.compose.details_track_id == Some(track_id) {
                None
            } else {
                Some(track_id)
            };
        }

        ComposeMessage::ClearInstrumentDetails => {
            r.compose.details_track_id = None;
        }

        ComposeMessage::AddChord {
            definition_id,
            start_beat,
            duration_beats,
            root,
            quality,
        } => {
            let (length_bars, overlap) = match r.compose.find_definition(definition_id) {
                Some(d) => (
                    d.length_bars,
                    chord_overlaps(&d.chords, start_beat, duration_beats, None),
                ),
                None => return,
            };
            if !chord_fits_in_section(start_beat, duration_beats, length_bars, time_sig_num) {
                r.compose.last_error = Some("Chord does not fit inside the section".into());
                return;
            }
            if overlap {
                r.compose.last_error = Some("Chord overlaps another chord".into());
                return;
            }
            let id = r.compose.fresh_id();
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.chords.push(ChordState {
                    id,
                    start_beat,
                    duration_beats,
                    chord: Chord::new(root, quality),
                });
                def.chords.sort_by_key(|c| c.start_beat);
            }
            r.compose.last_error = None;
        }

        ComposeMessage::EditChord { definition_id, chord_id, chord } => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                if let Some(c) = def.chords.iter_mut().find(|c| c.id == chord_id) {
                    c.chord = chord;
                }
            }
            r.compose.last_error = None;
        }

        ComposeMessage::MoveChord {
            definition_id,
            chord_id,
            start_beat,
        } => {
            let (length_bars, duration_beats, overlap) =
                match r.compose.find_definition(definition_id) {
                    Some(d) => {
                        let c = match d.chords.iter().find(|c| c.id == chord_id) {
                            Some(c) => c,
                            None => return,
                        };
                        (
                            d.length_bars,
                            c.duration_beats,
                            chord_overlaps(&d.chords, start_beat, c.duration_beats, Some(chord_id)),
                        )
                    }
                    None => return,
                };
            if !chord_fits_in_section(start_beat, duration_beats, length_bars, time_sig_num) {
                r.compose.last_error = Some("Chord would move outside the section".into());
                return;
            }
            if overlap {
                r.compose.last_error = Some("Chord would overlap another chord".into());
                return;
            }
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                if let Some(c) = def.chords.iter_mut().find(|c| c.id == chord_id) {
                    c.start_beat = start_beat;
                }
                def.chords.sort_by_key(|c| c.start_beat);
            }
            r.compose.last_error = None;
        }

        ComposeMessage::ResizeChord {
            definition_id,
            chord_id,
            duration_beats,
        } => {
            if duration_beats == 0 {
                r.compose.last_error = Some("Chord duration must be at least 1 beat".into());
                return;
            }
            let (length_bars, start_beat, overlap) =
                match r.compose.find_definition(definition_id) {
                    Some(d) => {
                        let c = match d.chords.iter().find(|c| c.id == chord_id) {
                            Some(c) => c,
                            None => return,
                        };
                        (
                            d.length_bars,
                            c.start_beat,
                            chord_overlaps(&d.chords, c.start_beat, duration_beats, Some(chord_id)),
                        )
                    }
                    None => return,
                };
            if !chord_fits_in_section(start_beat, duration_beats, length_bars, time_sig_num) {
                r.compose.last_error = Some("Chord would extend past the section".into());
                return;
            }
            if overlap {
                r.compose.last_error = Some("Chord would overlap another chord".into());
                return;
            }
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                if let Some(c) = def.chords.iter_mut().find(|c| c.id == chord_id) {
                    c.duration_beats = duration_beats;
                }
            }
            r.compose.last_error = None;
        }

        ComposeMessage::DeleteChord { definition_id, chord_id } => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.chords.retain(|c| c.id != chord_id);
            }
            if r.compose.selected_chord_id == Some(chord_id) {
                r.compose.selected_chord_id = None;
            }
            r.compose.last_error = None;
        }
    }
}

/// Find the earliest bar where a placement of the given length would not
/// collide with any existing placement. Scans bar-by-bar; adequate for
/// interactive use since project lengths are small.
fn first_free_bar(state: &ComposeState, length_bars: u32) -> u32 {
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
            return candidate; // sanity cap; should never hit
        }
    }
}
