//! Handlers for the chord-lane inspector messages: progression generator
//! controls (table / chord count / start-end degree / locks / Generate /
//! Regenerate) plus the section-shared motif knobs.

use resonance_music_theory::{diatonic_chord, GenContext, Generator, GeneratorSpec, SchemaKind};

use super::regenerate::{propagate_chord_change, propagate_motif_change};
use crate::compose::messages::{ChordInspectorMsg, GeneratorKind, MotifSourceKind};
use crate::compose::ChordState;
use crate::util::{bump_seed, GOLDEN_RATIO_SEED};

pub(super) fn handle(
    r: &mut crate::Resonance,
    definition_id: u64,
    msg: ChordInspectorMsg,
) {
    match msg {
        ChordInspectorMsg::SetGeneratorKind(kind) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                // Carry the requested chord count across the switch;
                // everything else takes the target mode's defaults.
                let length = match &def.generator_spec {
                    Some(GeneratorSpec::MarkovProgression { length, .. })
                    | Some(GeneratorSpec::Schema { length, .. })
                    | Some(GeneratorSpec::Pentatonic { length, .. }) => *length,
                    None => def.generate_params.chord_count.max(1) as u8,
                };
                match (kind, &def.generator_spec) {
                    // Already in the requested mode — nothing to do.
                    (GeneratorKind::Markov, Some(GeneratorSpec::MarkovProgression { .. }))
                    | (GeneratorKind::Schema, Some(GeneratorSpec::Schema { .. })) => {}
                    (GeneratorKind::Markov, _) => {
                        def.generator_spec = Some(GeneratorSpec::MarkovProgression {
                            length,
                            table_id: "pop".to_string(),
                            order: 1,
                            start: None,
                            end: None,
                        });
                    }
                    (GeneratorKind::Schema, _) => {
                        def.generator_spec = Some(GeneratorSpec::Schema {
                            schema: SchemaKind::Axis,
                            length,
                            rotation: 0,
                            substitution: 0.0,
                        });
                    }
                }
                def.generate_params.chord_count = length as u32;
                r.compose.last_error = None;
            }
            regenerate_if_materialized(r, definition_id);
        }

        ChordInspectorMsg::SetSchemaKind(kind) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                let length = kind.default_length();
                match &mut def.generator_spec {
                    Some(GeneratorSpec::Schema {
                        schema,
                        length: l,
                        rotation,
                        ..
                    }) => {
                        *schema = kind;
                        // Snap to the schema's natural loop length and
                        // drop the rotation — the old offset is
                        // meaningless against a different loop.
                        *l = length;
                        *rotation = 0;
                    }
                    // No spec yet, or a Markov spec: picking a schema
                    // switches to a Schema spec (mirrors SetTable).
                    _ => {
                        def.generator_spec = Some(GeneratorSpec::Schema {
                            schema: kind,
                            length,
                            rotation: 0,
                            substitution: 0.0,
                        });
                    }
                }
                def.generate_params.chord_count = length as u32;
                r.compose.last_error = None;
            }
            regenerate_if_materialized(r, definition_id);
        }

        ChordInspectorMsg::SetSchemaRotation(rot) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                if let Some(GeneratorSpec::Schema { rotation, .. }) = &mut def.generator_spec {
                    *rotation = rot;
                }
                r.compose.last_error = None;
            }
            regenerate_if_materialized(r, definition_id);
        }

        ChordInspectorMsg::SetSchemaSubstitution(s) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                if let Some(GeneratorSpec::Schema { substitution, .. }) = &mut def.generator_spec {
                    *substitution = s.clamp(0.0, 1.0);
                }
                r.compose.last_error = None;
            }
            regenerate_if_materialized(r, definition_id);
        }

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
                    // No spec yet, or a non-Markov spec (e.g. Schema):
                    // picking a table switches to a Markov spec.
                    _ => {
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
                    Some(GeneratorSpec::MarkovProgression { length: l, .. })
                    | Some(GeneratorSpec::Schema { length: l, .. })
                    | Some(GeneratorSpec::Pentatonic { length: l, .. }) => {
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
                def.generate_params.chord_count = length as u32;
                r.compose.last_error = None;
            }
        }

        ChordInspectorMsg::SetBeatsPerChord(beats) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.beats_per_chord = beats.clamp(1, 16);
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

        ChordInspectorMsg::Generate => {
            generate_chord_lane(r, definition_id, false);
        }

        ChordInspectorMsg::Regenerate => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.generator_seed = bump_seed(def.generator_seed, GOLDEN_RATIO_SEED);
            }
            generate_chord_lane(r, definition_id, true);
        }

        ChordInspectorMsg::SetMotifComplexity(c) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.motif_source.params_mut().complexity = c.clamp(0.0, 1.0);
                r.compose.last_error = None;
            }
            propagate_motif_change(r, definition_id);
        }

        ChordInspectorMsg::SetMotifLen(n) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.motif_source.params_mut().motif_len = n;
                r.compose.last_error = None;
            }
            propagate_motif_change(r, definition_id);
        }

        ChordInspectorMsg::SetMotifLeapChance(c) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.motif_source.params_mut().leap_chance = c.clamp(0.0, 1.0);
                r.compose.last_error = None;
            }
            propagate_motif_change(r, definition_id);
        }

        ChordInspectorMsg::RegenerateMotif => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                let params = def.motif_source.params_mut();
                params.seed = bump_seed(params.seed, GOLDEN_RATIO_SEED);
                r.compose.last_error = None;
            }
            propagate_motif_change(r, definition_id);
        }

        ChordInspectorMsg::SetMotifSourceKind(kind) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                use resonance_music_theory::MotifSource;
                let prev_params = *def.motif_source.params();
                let switched = match (kind, &def.motif_source) {
                    (MotifSourceKind::Generated, MotifSource::Manual { .. }) => {
                        def.motif_source = MotifSource::Generated(prev_params);
                        true
                    }
                    (MotifSourceKind::Manual, MotifSource::Generated(_)) => {
                        def.motif_source = MotifSource::Manual {
                            notes: MotifSource::default_manual_notes(),
                            params: prev_params,
                        };
                        true
                    }
                    _ => false,
                };
                if switched {
                    r.compose.last_error = None;
                }
            }
            propagate_motif_change(r, definition_id);
        }

        ChordInspectorMsg::ToggleManualMotifCell { scale_step, beat_16 } => {
            if let Some(notes) = manual_notes_mut(r, definition_id) {
                resonance_music_theory::toggle_manual_motif_cell(
                    notes,
                    resonance_music_theory::ManualMotifCell::Note { scale_step },
                    beat_16,
                );
                r.compose.last_error = None;
            }
            propagate_motif_change(r, definition_id);
        }

        ChordInspectorMsg::ToggleManualMotifRest { beat_16 } => {
            if let Some(notes) = manual_notes_mut(r, definition_id) {
                resonance_music_theory::toggle_manual_motif_cell(
                    notes,
                    resonance_music_theory::ManualMotifCell::Rest,
                    beat_16,
                );
                r.compose.last_error = None;
            }
            propagate_motif_change(r, definition_id);
        }

        ChordInspectorMsg::CycleManualMotifNoteDuration { index } => {
            if let Some(notes) = manual_notes_mut(r, definition_id) {
                if let Some(n) = notes.get_mut(index) {
                    // Cycle 1 → 2 → 3 → 4 → 1 sixteenths.
                    n.duration_sixteenths = match n.duration_sixteenths {
                        1 => 2,
                        2 => 3,
                        3 => 4,
                        _ => 1,
                    };
                }
                r.compose.last_error = None;
            }
            propagate_motif_change(r, definition_id);
        }

        ChordInspectorMsg::ToggleManualMotifAccent { index } => {
            if let Some(notes) = manual_notes_mut(r, definition_id) {
                if let Some(n) = notes.get_mut(index) {
                    n.accent = !n.accent;
                }
                r.compose.last_error = None;
            }
            propagate_motif_change(r, definition_id);
        }

        ChordInspectorMsg::ClearManualMotif => {
            if let Some(notes) = manual_notes_mut(r, definition_id) {
                notes.clear();
                r.compose.last_error = None;
            }
            propagate_motif_change(r, definition_id);
        }
    }
}

/// Live-preview hook for the schema controls: once the lane has been
/// materialized (Generate pressed at least once), schema edits — kind,
/// rotation, substitution — re-run the generator immediately so the
/// chord lane tracks the controls. Generation is a pure function of
/// (spec, seed), so this is deterministic and cheap. Before the first
/// Generate the spec is just being staged, matching the Markov controls.
fn regenerate_if_materialized(r: &mut crate::Resonance, definition_id: u64) {
    let materialized = r
        .compose
        .find_definition(definition_id)
        .is_some_and(|def| def.generated_material.is_some());
    if materialized {
        generate_chord_lane(r, definition_id, true);
    }
}

/// Borrow the manual-motif note vector on the section identified by
/// `definition_id`. Returns `None` if the definition doesn't exist or its
/// motif is currently in `Generated` mode — every manual-motif handler
/// short-circuits in that case.
fn manual_notes_mut(
    r: &mut crate::Resonance,
    definition_id: u64,
) -> Option<&mut Vec<resonance_music_theory::ManualMotifNote>> {
    r.compose
        .find_definition_mut(definition_id)
        .and_then(|def| def.motif_source.manual_notes_mut())
}

/// Generate a chord progression from the section's `GeneratorSpec`
/// (Markov table or pop schema; defaults to the "pop" Markov table).
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
        GeneratorSpec::Schema { length, .. } => *length as usize,
        GeneratorSpec::Pentatonic { length, .. } => *length as usize,
    };

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
    // Diatonic degrees (flat=false) derive their quality from the scale
    // interval pattern (degree 1 in B minor → Bm, not B). Borrowed chords
    // (flat=true) use the Degree's explicit quality since they're
    // intentionally non-diatonic.
    // Inversions ride on top of either projection as a slash bass
    // (ii6 → Dm/F, cadential 6/4 → C/G); the SATB pass in derive_pad
    // plans the bass from the slash, so the pre-dominant bass idioms
    // and the 6/4's stationary dominant bass come out in the voicing.
    let project = |degree: resonance_music_theory::Degree| {
        let chord = if degree.flat {
            degree.to_chord(scale)
        } else {
            diatonic_chord(scale, degree.root, def.seventh_chords)
        };
        chord.inverted(degree.inversion)
    };
    // Harmonic-rhythm splits from the phrase-model overlay divide a
    // slot in half (`| IV ii |` before the cadential `V`). Slots stay
    // `beats_per_chord` wide; only the chords within them subdivide.
    // Slots too narrow to halve render the front-half chord only.
    let mut new_chords = Vec::with_capacity(material.chords.len() + material.splits.len());
    for (i, gc) in material.chords.iter().enumerate() {
        let slot_start = i as u32 * beats_per_chord;
        let split = material
            .splits
            .iter()
            .find(|s| s.slot as usize == i)
            .filter(|_| beats_per_chord >= 2);
        match split {
            Some(s) => {
                let front = beats_per_chord - beats_per_chord / 2;
                new_chords.push(ChordState {
                    id: r.compose.fresh_id(),
                    start_beat: slot_start,
                    duration_beats: front,
                    chord: project(gc.degree),
                });
                new_chords.push(ChordState {
                    id: r.compose.fresh_id(),
                    start_beat: slot_start + front,
                    duration_beats: beats_per_chord / 2,
                    chord: project(s.degree),
                });
            }
            None => new_chords.push(ChordState {
                id: r.compose.fresh_id(),
                start_beat: slot_start,
                duration_beats: beats_per_chord,
                chord: project(gc.degree),
            }),
        }
    }

    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.chords = new_chords;
        def.generated_material = Some(material);
        if def.generator_spec.is_none() {
            def.generator_spec = Some(spec);
        }
    }
    r.compose.selected_chord_id = None;
    r.compose.last_error = None;

    propagate_chord_change(r, definition_id);
}
