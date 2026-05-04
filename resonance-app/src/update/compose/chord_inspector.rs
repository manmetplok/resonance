//! Handlers for the chord-lane inspector messages: progression generator
//! controls (table / chord count / start-end degree / locks / Generate /
//! Regenerate) plus the section-shared motif knobs.

use resonance_music_theory::{diatonic_chord, GenContext, Generator, GeneratorSpec};

use super::regenerate::{propagate_chord_change, propagate_motif_change};
use crate::compose::messages::ChordInspectorMsg;
use crate::compose::ChordState;

pub(super) fn handle(
    r: &mut crate::Resonance,
    definition_id: u64,
    msg: ChordInspectorMsg,
) {
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
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.generator_seed = def
                    .generator_seed
                    .wrapping_add(0x9E3779B97F4A7C15)
                    .wrapping_add(1);
            }
            generate_chord_lane(r, definition_id, true);
        }

        ChordInspectorMsg::SetMotifComplexity(c) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.motif.complexity = c.clamp(0.0, 1.0);
                r.compose.last_error = None;
            }
            propagate_motif_change(r, definition_id);
        }

        ChordInspectorMsg::SetMotifLen(n) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.motif.motif_len = n;
                r.compose.last_error = None;
            }
            propagate_motif_change(r, definition_id);
        }

        ChordInspectorMsg::SetMotifLeapChance(c) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.motif.leap_chance = c.clamp(0.0, 1.0);
                r.compose.last_error = None;
            }
            propagate_motif_change(r, definition_id);
        }

        ChordInspectorMsg::RegenerateMotif => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.motif.seed = def
                    .motif
                    .seed
                    .wrapping_add(0x9E3779B97F4A7C15)
                    .wrapping_add(1);
                r.compose.last_error = None;
            }
            propagate_motif_change(r, definition_id);
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
        if def.generator_spec.is_none() {
            def.generator_spec = Some(spec);
        }
    }
    r.compose.selected_chord_id = None;
    r.compose.last_error = None;

    propagate_chord_change(r, definition_id);
}
