use std::collections::HashMap;

use resonance_audio::types::{AudioCommand, TrackId, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::{
    diatonic_chord, BassParams, Chord, GenContext, Generator, GeneratorSpec, MelodyParams,
    PadParams,
};

use crate::compose::invariants::{chord_fits_in_section, chord_overlaps, placement_overlaps};
use crate::compose::messages::{ChordInspectorMsg, LaneInspectorMsg};
use crate::compose::{
    generate, ChordState, ComposeMessage, ComposeState, DeriveKind, EditSectionForm,
    LaneGeneratorConfig, LaneGeneratorKind, LaneGeneratorKindTag, NewSectionForm,
    SectionDefinitionState, SectionPlacementState, SelectedLane,
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
            let duration_ticks = length_bars as u64 * time_sig_num as u64 * TICKS_PER_QUARTER_NOTE;
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
            handle(
                r,
                ComposeMessage::RenameSection {
                    definition_id: form.definition_id,
                    name,
                },
            );
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
            handle(
                r,
                ComposeMessage::CreateSection {
                    name,
                    length_bars,
                    color: form.color,
                },
            );
        }

        ComposeMessage::CreateSection {
            name,
            length_bars,
            color,
        } => {
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

        ComposeMessage::RenameSection {
            definition_id,
            name,
        } => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.name = name;
                r.compose.last_error = None;
            }
        }

        ComposeMessage::ResizeSection {
            definition_id,
            length_bars,
        } => {
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
                        r.compose.last_error = Some(
                            "Cannot grow section: a placement would overlap a neighbour".into(),
                        );
                        return;
                    }
                }
            }
            let chords_fit = r
                .compose
                .find_definition(definition_id)
                .map(|d| {
                    d.chords.iter().all(|c| {
                        chord_fits_in_section(
                            c.start_beat,
                            c.duration_beats,
                            length_bars,
                            time_sig_num,
                        )
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

        ComposeMessage::SetSectionScale {
            definition_id,
            scale,
        } => {
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
                r.compose.last_error =
                    Some("Cannot delete a section while placements still reference it".into());
                return;
            }
            r.compose.definitions.retain(|d| d.id != definition_id);
            r.compose.last_error = None;
        }

        ComposeMessage::PlaceSection {
            definition_id,
            start_bar,
        } => {
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
            r.compose.selected_lane = SelectedLane::Chords;
        }

        ComposeMessage::ClearChordSelection => {
            r.compose.selected_chord_id = None;
        }

        ComposeMessage::SelectLane(lane) => {
            r.compose.selected_lane = lane;
        }

        ComposeMessage::ExpandTrack { track_id } => {
            if r.compose.expanded_track_id == Some(track_id) {
                r.compose.expanded_track_id = None;
            } else {
                r.compose.expanded_track_id = Some(track_id);
                // Also select this track's lane.
                let is_drum = r
                    .registry
                    .tracks
                    .iter()
                    .find(|t| t.id == track_id)
                    .map(|t| t.instrument_type == crate::state::InstrumentType::Drum)
                    .unwrap_or(false);
                r.compose.selected_lane = if is_drum {
                    SelectedLane::Drums(track_id)
                } else {
                    SelectedLane::Instrument(track_id)
                };
                r.compose.expanded_scroll_x = 0.0;
                r.compose.expanded_scroll_y = 40.0 * r.compose.expanded_zoom_y;
            }
        }

        ComposeMessage::CollapseTrack => {
            r.compose.expanded_track_id = None;
        }

        ComposeMessage::ExpandedScrollX(delta) => {
            r.compose.expanded_scroll_x = (r.compose.expanded_scroll_x + delta).max(0.0);
        }

        ComposeMessage::ExpandedScrollY(delta) => {
            r.compose.expanded_scroll_y = (r.compose.expanded_scroll_y + delta).max(0.0);
        }

        ComposeMessage::ExpandedZoomY(delta) => {
            r.compose.expanded_zoom_y = (r.compose.expanded_zoom_y + delta).clamp(4.0, 40.0);
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
            propagate_chord_change(r, definition_id);
        }

        ComposeMessage::EditChord {
            definition_id,
            chord_id,
            chord,
        } => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                if let Some(c) = def.chords.iter_mut().find(|c| c.id == chord_id) {
                    c.chord = chord;
                }
            }
            r.compose.last_error = None;
            propagate_chord_change(r, definition_id);
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
            propagate_chord_change(r, definition_id);
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
            let (length_bars, start_beat, overlap) = match r.compose.find_definition(definition_id)
            {
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
            propagate_chord_change(r, definition_id);
        }

        ComposeMessage::DeleteChord {
            definition_id,
            chord_id,
        } => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.chords.retain(|c| c.id != chord_id);
            }
            if r.compose.selected_chord_id == Some(chord_id) {
                r.compose.selected_chord_id = None;
            }
            r.compose.last_error = None;
            propagate_chord_change(r, definition_id);
        }

        // ---- Chord lane inspector ----
        ComposeMessage::ChordInspector { definition_id, msg } => {
            handle_chord_inspector(r, definition_id, msg);
        }

        // ---- Per-track lane inspector ----
        ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg,
        } => {
            handle_lane_inspector(r, definition_id, track_id, msg);
        }

        ComposeMessage::SetTrackRole { track_id, role } => {
            if let Some(track) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) {
                track.role = role;
            }
        }
    }
}

// ===========================================================================
// Chord lane inspector handler
// ===========================================================================

fn handle_chord_inspector(r: &mut crate::Resonance, definition_id: u64, msg: ChordInspectorMsg) {
    match msg {
        ChordInspectorMsg::SetTable(table_id) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                match &mut def.generator_spec {
                    Some(GeneratorSpec::MarkovProgression {
                        table_id: tid,
                        start,
                        end,
                        ..
                    }) => {
                        *tid = table_id;
                        // Clear degree constraints — the new table may have
                        // a different vocabulary.
                        *start = None;
                        *end = None;
                    }
                    None => {
                        def.generator_spec = Some(GeneratorSpec::MarkovProgression {
                            length: def.generate_params.chord_count as u8,
                            table_id,
                            order: 1,
                            start: None,
                            end: None,
                        });
                    }
                }
                r.compose.last_error = None;
            }
        }

        ChordInspectorMsg::SetLength(length) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                match &mut def.generator_spec {
                    Some(GeneratorSpec::MarkovProgression { length: l, .. }) => {
                        *l = length;
                    }
                    None => {
                        def.generator_spec = Some(GeneratorSpec::MarkovProgression {
                            length,
                            table_id: "pop".to_string(),
                            order: 1,
                            start: None,
                            end: None,
                        });
                    }
                }
                // Keep old generate_params in sync for migration.
                def.generate_params.chord_count = length as u32;
                r.compose.last_error = None;
            }
        }

        ChordInspectorMsg::SetBeatsPerChord(beats) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.beats_per_chord = beats.max(1).min(16);
                def.generate_params.beats_per_chord = def.beats_per_chord;
                r.compose.last_error = None;
            }
        }

        ChordInspectorMsg::SetSeventhChords(on) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.seventh_chords = on;
                def.generate_params.seventh_chords = on;
                r.compose.last_error = None;
            }
        }

        ChordInspectorMsg::SetStartDegree(degree) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                if let Some(GeneratorSpec::MarkovProgression { start, .. }) =
                    &mut def.generator_spec
                {
                    *start = degree;
                }
                r.compose.last_error = None;
            }
        }

        ChordInspectorMsg::SetEndDegree(degree) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                if let Some(GeneratorSpec::MarkovProgression { end, .. }) = &mut def.generator_spec
                {
                    *end = degree;
                }
                r.compose.last_error = None;
            }
        }

        ChordInspectorMsg::ToggleLock(index) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                if let Some(ref mut material) = def.generated_material {
                    if let Some(chord) = material.chords.get_mut(index) {
                        chord.locked = !chord.locked;
                    }
                }
                r.compose.last_error = None;
            }
        }

        ChordInspectorMsg::Generate => {
            generate_chord_lane(r, definition_id, false);
        }

        ChordInspectorMsg::Regenerate => {
            // Bump the seed
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.generator_seed = def
                    .generator_seed
                    .wrapping_add(0x9E3779B97F4A7C15)
                    .wrapping_add(1);
            }
            generate_chord_lane(r, definition_id, true);
        }
    }
}

/// Generate a chord progression using the MarkovProgression generator.
fn generate_chord_lane(r: &mut crate::Resonance, definition_id: u64, respect_locks: bool) {
    let time_sig_num = r.transport.time_sig_num;
    let def = match r.compose.find_definition(definition_id) {
        Some(d) => d.clone(),
        None => return,
    };
    let Some(scale) = def.scale else {
        r.compose.last_error = Some("Pick a scale before generating a progression".into());
        return;
    };

    // Ensure we have a generator spec; create a default one if absent.
    let spec = def
        .generator_spec
        .clone()
        .unwrap_or_else(|| GeneratorSpec::MarkovProgression {
            length: def.generate_params.chord_count.max(1) as u8,
            table_id: "pop".to_string(),
            order: 1,
            start: None,
            end: None,
        });

    let length = match &spec {
        GeneratorSpec::MarkovProgression { length, .. } => *length as usize,
    };

    // Build locked positions from existing generated_material
    let locked: Vec<Option<resonance_music_theory::Degree>> = if respect_locks {
        def.generated_material
            .as_ref()
            .map(|m| {
                m.chords
                    .iter()
                    .map(|c| if c.locked { Some(c.degree) } else { None })
                    .collect()
            })
            .unwrap_or_else(|| vec![None; length])
    } else {
        vec![None; length]
    };

    // Pad or truncate locked vector to match requested length
    let mut locked_padded = locked;
    locked_padded.resize(length, None);

    let ctx = GenContext {
        registry: &r.table_registry,
        locked: &locked_padded,
    };

    let material = match spec.generate(def.generator_seed, &ctx) {
        Ok(m) => m,
        Err(e) => {
            r.compose.last_error = Some(format!("Generation failed: {e}"));
            return;
        }
    };

    let beats_per_chord = def.beats_per_chord.max(1);
    let section_beats = def.length_bars * time_sig_num as u32;
    let total_beats = material.chords.len() as u32 * beats_per_chord;
    if total_beats > section_beats {
        r.compose.last_error = Some(format!(
            "Generated {} chords × {} beats won't fit in {} bars",
            material.chords.len(),
            beats_per_chord,
            def.length_bars
        ));
        return;
    }

    // Project degrees to concrete chords using the scale.
    // For diatonic degrees (flat=false), derive the chord quality from the
    // scale's interval pattern so that e.g. degree 1 in B minor produces
    // Bm, not B major. For borrowed chords (flat=true), use the explicit
    // quality stored in the Degree since they're intentionally non-diatonic.
    let mut new_chords = Vec::with_capacity(material.chords.len());
    for (i, gc) in material.chords.iter().enumerate() {
        let id = r.compose.fresh_id();
        let chord = if gc.degree.flat {
            gc.degree.to_chord(scale)
        } else {
            diatonic_chord(scale, gc.degree.root, def.seventh_chords)
        };
        new_chords.push(ChordState {
            id,
            start_beat: i as u32 * beats_per_chord,
            duration_beats: beats_per_chord,
            chord,
        });
    }

    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.chords = new_chords;
        def.generated_material = Some(material);
        // Persist the spec if it was created implicitly.
        if def.generator_spec.is_none() {
            def.generator_spec = Some(spec);
        }
    }
    r.compose.selected_chord_id = None;
    r.compose.last_error = None;

    // Cascade: regenerate all dependent instrument lanes.
    propagate_chord_change(r, definition_id);
}

// ===========================================================================
// Per-track lane inspector handler
// ===========================================================================

fn handle_lane_inspector(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    msg: LaneInspectorMsg,
) {
    match msg {
        LaneInspectorMsg::SetGenerator(tag) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                match tag {
                    LaneGeneratorKindTag::Manual => {
                        def.lane_generators.remove(&track_id);
                    }
                    LaneGeneratorKindTag::Bass => {
                        def.lane_generators.insert(
                            track_id,
                            LaneGeneratorConfig {
                                kind: LaneGeneratorKind::Bass(BassParams::default()),
                                seed: definition_id.wrapping_mul(0x9E3779B97F4A7C15),
                            },
                        );
                    }
                    LaneGeneratorKindTag::Melody => {
                        def.lane_generators.insert(
                            track_id,
                            LaneGeneratorConfig {
                                kind: LaneGeneratorKind::Melody(MelodyParams::default()),
                                seed: definition_id.wrapping_mul(0x517CC1B727220A95),
                            },
                        );
                    }
                    LaneGeneratorKindTag::Pad => {
                        def.lane_generators.insert(
                            track_id,
                            LaneGeneratorConfig {
                                kind: LaneGeneratorKind::Pad(PadParams::default()),
                                seed: definition_id.wrapping_mul(0x6C62272E07BB0142),
                            },
                        );
                    }
                }
                r.compose.last_error = None;
            }
        }

        // Bass parameter updates
        LaneInspectorMsg::SetBassStyle(style) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Bass(p) = kind {
                    p.style = style;
                }
            });
        }
        LaneInspectorMsg::SetBassBaseNote(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Bass(p) = kind {
                    p.base_note = note;
                }
            });
        }
        LaneInspectorMsg::SetBassVelocity(v) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Bass(p) = kind {
                    p.velocity = v;
                }
            });
        }

        // Melody parameter updates
        LaneInspectorMsg::SetMelodyStyle(style) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.style = style;
                }
            });
        }
        LaneInspectorMsg::SetMelodyRegisterLow(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.register.0 = note;
                }
            });
        }
        LaneInspectorMsg::SetMelodyRegisterHigh(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.register.1 = note;
                }
            });
        }
        LaneInspectorMsg::SetMelodyNoteValue(ticks) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.note_value_ticks = ticks;
                }
            });
        }
        LaneInspectorMsg::SetMelodyRestDensity(d) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.rest_density = d;
                }
            });
        }
        LaneInspectorMsg::SetMelodyVelocity(v) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.velocity = v;
                }
            });
        }
        LaneInspectorMsg::SetMelodyComplexity(c) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.complexity = c;
                }
            });
        }
        LaneInspectorMsg::SetMelodyArticulation(a) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.articulation = a;
                }
            });
        }
        LaneInspectorMsg::SetMelodyContour(contour) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.contour = contour;
                }
            });
        }
        LaneInspectorMsg::SetMelodyPhraseLen(len) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.phrase_len = len;
                }
            });
        }
        LaneInspectorMsg::SetMelodyMotifLen(len) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.motif_len = len;
                }
            });
        }
        LaneInspectorMsg::SetMelodyLeapChance(c) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.leap_chance = c;
                }
            });
        }

        // Pad parameter updates
        LaneInspectorMsg::SetPadRegisterLow(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Pad(p) = kind {
                    p.register.0 = note;
                }
            });
        }
        LaneInspectorMsg::SetPadRegisterHigh(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Pad(p) = kind {
                    p.register.1 = note;
                }
            });
        }
        LaneInspectorMsg::SetPadVelocity(v) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Pad(p) = kind {
                    p.velocity = v;
                }
            });
        }

        // Drum voice mode
        LaneInspectorMsg::SetDrumVoiceMode { pad_index, mode } => {
            ensure_drum_config(r, definition_id, track_id);
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Drum(dc) = kind {
                    dc.voices.insert(pad_index, mode);
                }
            });
        }
        LaneInspectorMsg::SetDrumEuclidSteps { pad_index, steps } => {
            update_drum_voice(r, definition_id, track_id, pad_index, |mode| {
                if let crate::compose::DrumVoiceMode::Euclidean { steps: s, .. } = mode {
                    *s = steps.max(1);
                }
            });
        }
        LaneInspectorMsg::SetDrumEuclidHits { pad_index, hits } => {
            update_drum_voice(r, definition_id, track_id, pad_index, |mode| {
                if let crate::compose::DrumVoiceMode::Euclidean { hits: h, steps, .. } = mode {
                    *h = hits.min(*steps);
                }
            });
        }
        LaneInspectorMsg::SetDrumEuclidRotation {
            pad_index,
            rotation,
        } => {
            update_drum_voice(r, definition_id, track_id, pad_index, |mode| {
                if let crate::compose::DrumVoiceMode::Euclidean { rotation: rot, .. } = mode {
                    *rot = rotation;
                }
            });
        }

        LaneInspectorMsg::Regenerate => {
            regenerate_lane(r, definition_id, track_id);
        }
    }
}

/// Helper: mutate a lane's generator kind in-place.
fn update_lane_gen(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    f: impl FnOnce(&mut LaneGeneratorKind),
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        if let Some(cfg) = def.lane_generators.get_mut(&track_id) {
            f(&mut cfg.kind);
        }
        r.compose.last_error = None;
    }
}

/// Ensure a drum lane config exists for the given track.
fn ensure_drum_config(r: &mut crate::Resonance, definition_id: u64, track_id: TrackId) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.lane_generators
            .entry(track_id)
            .or_insert_with(|| LaneGeneratorConfig {
                kind: LaneGeneratorKind::Drum(crate::compose::DrumLaneConfig::default()),
                seed: 0,
            });
    }
}

/// Helper: mutate a specific drum voice's mode in-place.
fn update_drum_voice(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    pad_index: usize,
    f: impl FnOnce(&mut crate::compose::DrumVoiceMode),
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        if let Some(cfg) = def.lane_generators.get_mut(&track_id) {
            if let LaneGeneratorKind::Drum(dc) = &mut cfg.kind {
                if let Some(mode) = dc.voices.get_mut(&pad_index) {
                    f(mode);
                }
            }
        }
        r.compose.last_error = None;
    }
}

// ===========================================================================
// Chord change propagation
// ===========================================================================

/// When chords change, regenerate all instrument lanes that have a
/// chord-reading generator (Bass, Melody, Pad).
fn propagate_chord_change(r: &mut crate::Resonance, definition_id: u64) {
    let track_ids: Vec<TrackId> = r
        .compose
        .find_definition(definition_id)
        .map(|def| {
            def.lane_generators
                .iter()
                .filter(|(_, cfg)| {
                    matches!(
                        cfg.kind,
                        LaneGeneratorKind::Bass(_)
                            | LaneGeneratorKind::Melody(_)
                            | LaneGeneratorKind::Pad(_)
                    )
                })
                .map(|(tid, _)| *tid)
                .collect()
        })
        .unwrap_or_default();

    for tid in track_ids {
        regenerate_lane(r, definition_id, tid);
    }
}

/// Regenerate a single instrument lane, producing MIDI clips for all
/// placements of the section.
fn regenerate_lane(r: &mut crate::Resonance, definition_id: u64, track_id: TrackId) {
    let def = match r.compose.find_definition(definition_id) {
        Some(d) => d.clone(),
        None => return,
    };

    let Some(config) = def.lane_generators.get(&track_id) else {
        return;
    };

    if def.chords.is_empty() {
        return;
    }

    // Determine derive kind from lane generator kind.
    let kind = match &config.kind {
        LaneGeneratorKind::Bass(_) => DeriveKind::Bass,
        LaneGeneratorKind::Melody(_) => DeriveKind::Lead,
        LaneGeneratorKind::Pad(_) => DeriveKind::Pad,
        LaneGeneratorKind::Drum(_) => return, // drums don't read chord context
    };

    // Build ad-hoc GenerateParams from the lane config to feed into derive_notes.
    let gen_params = match &config.kind {
        LaneGeneratorKind::Bass(p) => {
            let mut gp = crate::compose::GenerateParams::default();
            gp.bass = p.clone();
            gp
        }
        LaneGeneratorKind::Melody(p) => {
            let mut gp = crate::compose::GenerateParams::default();
            gp.melody = p.clone();
            gp
        }
        LaneGeneratorKind::Pad(p) => {
            let mut gp = crate::compose::GenerateParams::default();
            gp.pad = p.clone();
            gp
        }
        _ => return,
    };

    let notes = generate::derive_notes(
        kind,
        &def.chords,
        def.scale,
        &gen_params,
        TICKS_PER_QUARTER_NOTE as u32,
        config.seed,
    );

    let time_sig_num = r.transport.time_sig_num;
    let samples_per_bar = compose_samples_per_bar(r.sample_rate, r.transport.bpm, time_sig_num);
    let duration_ticks = def.length_bars as u64 * time_sig_num as u64 * TICKS_PER_QUARTER_NOTE;

    let track_name = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .map(|t| t.name.as_str())
        .unwrap_or("Track");
    let name = format!("{} · {}", def.name, track_name);

    let placements: Vec<(u64, u32)> = r
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .map(|p| (p.id, p.start_bar))
        .collect();

    for (placement_id, start_bar) in placements {
        if let Some(old_id) =
            r.compose
                .derived_clips
                .remove(&(definition_id, placement_id, track_id))
        {
            r.engine
                .send(AudioCommand::DeleteMidiClip { clip_id: old_id });
        }

        let clip_id = r.compose.fresh_derived_clip_id();
        let start_sample = start_bar as u64 * samples_per_bar;
        r.engine.send(AudioCommand::LoadMidiClipDirect {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            notes: notes.clone(),
            name: name.clone(),
            trim_start_ticks: 0,
            trim_end_ticks: 0,
        });
        r.compose
            .derived_clips
            .insert((definition_id, placement_id, track_id), clip_id);
    }

    r.compose.last_error = None;
}

// ===========================================================================
// Helpers
// ===========================================================================

fn compose_samples_per_bar(sample_rate: u32, bpm: f32, time_sig_num: u8) -> u64 {
    let samples_per_beat = sample_rate as f64 * 60.0 / bpm as f64;
    (samples_per_beat * time_sig_num as f64) as u64
}

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
            return candidate;
        }
    }
}
