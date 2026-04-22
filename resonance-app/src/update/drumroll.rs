use resonance_audio::types::{AudioCommand, ClipId, MidiNote, TICKS_PER_QUARTER_NOTE};

use crate::compose::drumroll::humanize::{self, HumanizeParams, HumanizeScope};
use crate::compose::drumroll::{euclidean, DrumrollMessage};
use crate::state::MidiClipState;

pub fn handle(r: &mut crate::Resonance, msg: DrumrollMessage) {
    match msg {
        DrumrollMessage::SelectPad { pad_index } => {
            if r.compose.drumroll.pad_map.get(pad_index).is_some() {
                r.compose.drumroll.selected_pad = Some(pad_index);
            }
        }
        DrumrollMessage::SetStepsPerBar(n) => {
            if matches!(n, 4 | 8 | 16 | 32) {
                r.compose.drumroll.steps_per_bar = n;
            }
        }
        DrumrollMessage::SetDefaultVelocity(v) => {
            r.compose.drumroll.default_velocity = v.clamp(0.0, 1.0);
        }
        DrumrollMessage::ToggleStep {
            clip_id,
            pad_index,
            step,
        } => {
            let Some(pad) = r.compose.drumroll.pad_map.get(pad_index).cloned() else {
                return;
            };
            let steps_per_bar = r.compose.drumroll.steps_per_bar;
            let time_sig_num = r.transport.time_sig_num;
            let velocity = r.compose.drumroll.default_velocity;
            let step_ticks = step_ticks_for(steps_per_bar, time_sig_num);
            if step_ticks == 0 {
                return;
            }
            let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) else {
                return;
            };
            let target_tick = step as u64 * step_ticks;
            if target_tick >= clip.duration_ticks {
                return;
            }
            // Find an existing note on this pad that lies inside this step.
            let existing = clip
                .notes
                .iter()
                .position(|n| n.note == pad.note && (n.start_tick / step_ticks) == step as u64);
            if let Some(note_index) = existing {
                r.engine.send(AudioCommand::RemoveMidiNote {
                    clip_id,
                    note_index,
                });
            } else {
                r.engine.send(AudioCommand::AddMidiNote {
                    clip_id,
                    note: MidiNote {
                        note: pad.note,
                        velocity,
                        start_tick: target_tick,
                        duration_ticks: step_ticks,
                    },
                });
            }
        }

        DrumrollMessage::GenerateEuclideanPad { clip_id, pad_index } => {
            let Some(pad) = r.compose.drumroll.pad_map.get(pad_index).cloned() else {
                return;
            };
            let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) else {
                return;
            };

            // Read euclidean parameters from the lane generator config
            // (the model that the UI text inputs actually update).
            let track_id = clip.track_id;
            let euclid_params = r
                .compose
                .selected_placement()
                .and_then(|p| r.compose.find_definition(p.definition_id))
                .and_then(|def| def.lane_generators.get(&track_id))
                .and_then(|cfg| match &cfg.kind {
                    crate::compose::LaneGeneratorKind::Drum(dc) => {
                        dc.voices.get(&pad_index)
                    }
                    _ => None,
                })
                .and_then(|mode| match mode {
                    crate::compose::DrumVoiceMode::Euclidean {
                        steps,
                        hits,
                        rotation,
                    } => Some((*steps, *hits, *rotation)),
                    _ => None,
                });

            let (steps, hits, rotation) = euclid_params.unwrap_or((16, 4, 0));
            let steps = steps.max(1);
            let hits = hits.min(steps);
            let velocity = r.compose.drumroll.default_velocity;

            let pattern = euclidean::bjorklund(steps, hits, rotation);
            let new_notes =
                euclidean::pattern_to_notes(&pattern, pad.note, velocity, clip.duration_ticks);
            let (removals, adds) = euclid_edits_for(clip, pad.note, new_notes);
            send_pad_edits(r, clip_id, removals, adds);
        }

        DrumrollMessage::ClearPad { clip_id, pad_index } => {
            let Some(pad) = r.compose.drumroll.pad_map.get(pad_index).cloned() else {
                return;
            };
            let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) else {
                return;
            };
            let removals: Vec<usize> = clip
                .notes
                .iter()
                .enumerate()
                .filter_map(|(i, n)| (n.note == pad.note).then_some(i))
                .collect();
            send_pad_edits(r, clip_id, removals, Vec::new());
        }

        DrumrollMessage::SetHumanizeVelocity(v) => {
            r.compose.drumroll.humanize_velocity = v.clamp(0.0, 1.0);
        }
        DrumrollMessage::SetHumanizeTiming(v) => {
            r.compose.drumroll.humanize_timing = v.clamp(0.0, 1.0);
        }
        DrumrollMessage::SetHumanizeSwing(v) => {
            r.compose.drumroll.humanize_swing = v.clamp(0.0, 1.0);
        }
        DrumrollMessage::SetHumanizeAccent(pattern) => {
            r.compose.drumroll.humanize_accent = pattern;
        }
        DrumrollMessage::SetHumanizeAccentAmount(v) => {
            r.compose.drumroll.humanize_accent_amount = v.clamp(0.0, 1.0);
        }
        DrumrollMessage::SetHumanizeScope(scope) => {
            r.compose.drumroll.humanize_scope = scope;
        }
        DrumrollMessage::ApplyHumanize { clip_id } => {
            let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) else {
                return;
            };
            let steps_per_bar = r.compose.drumroll.steps_per_bar;
            let time_sig_num = r.transport.time_sig_num;
            let step_ticks = step_ticks_for(steps_per_bar, time_sig_num);
            if step_ticks == 0 {
                return;
            }
            let selected_pad_note = r
                .compose
                .drumroll
                .selected_pad
                .and_then(|i| r.compose.drumroll.pad_map.get(i))
                .map(|p| p.note);
            if r.compose.drumroll.humanize_scope == HumanizeScope::SelectedPad
                && selected_pad_note.is_none()
            {
                return;
            }
            let params = HumanizeParams {
                velocity_amount: r.compose.drumroll.humanize_velocity,
                timing_amount: r.compose.drumroll.humanize_timing,
                swing: r.compose.drumroll.humanize_swing,
                accent_pattern: r.compose.drumroll.humanize_accent,
                accent_amount: r.compose.drumroll.humanize_accent_amount,
                selected_pad_note,
                scope: r.compose.drumroll.humanize_scope,
                step_ticks,
                steps_per_beat: steps_per_bar / time_sig_num.max(1) as u32,
                steps_per_bar,
                clip_length_ticks: clip.duration_ticks,
                seed: fresh_seed(),
            };
            let (removals, adds) = humanize_edits_for(clip, &params);
            send_pad_edits(r, clip_id, removals, adds);
        }
    }
}

/// Pure helper: returns `(removal_indices_desc, replacement_notes)` for
/// replacing every in-scope note in `clip` with its humanized version.
pub fn humanize_edits_for(
    clip: &MidiClipState,
    params: &HumanizeParams,
) -> (Vec<usize>, Vec<MidiNote>) {
    let humanized = humanize::humanize(&clip.notes, params);
    // Determine which original notes were in scope: those are the ones
    // that have to be removed and re-added. Out-of-scope notes were
    // copied verbatim and can stay.
    let mut removals = Vec::new();
    let mut adds = Vec::new();
    for (i, (orig, new)) in clip.notes.iter().zip(humanized.iter()).enumerate() {
        let in_scope = match params.scope {
            HumanizeScope::AllPads => true,
            HumanizeScope::SelectedPad => params.selected_pad_note == Some(orig.note),
        };
        if in_scope {
            removals.push(i);
            adds.push(new.clone());
        }
    }
    removals.sort_unstable_by(|a, b| b.cmp(a));
    (removals, adds)
}

fn fresh_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E3779B97F4A7C15)
}

/// Compute the (removals, additions) diff for replacing every hit on
/// `pad_note` in `clip` with `new_notes`. Removal indices come back sorted
/// descending so the caller can remove them in order without invalidating
/// subsequent indices.
pub fn euclid_edits_for(
    clip: &MidiClipState,
    pad_note: u8,
    new_notes: Vec<MidiNote>,
) -> (Vec<usize>, Vec<MidiNote>) {
    let mut removals: Vec<usize> = clip
        .notes
        .iter()
        .enumerate()
        .filter_map(|(i, n)| (n.note == pad_note).then_some(i))
        .collect();
    removals.sort_unstable_by(|a, b| b.cmp(a));
    (removals, new_notes)
}

fn send_pad_edits(
    r: &mut crate::Resonance,
    clip_id: ClipId,
    removals: Vec<usize>,
    adds: Vec<MidiNote>,
) {
    for note_index in removals {
        r.engine.send(AudioCommand::RemoveMidiNote {
            clip_id,
            note_index,
        });
    }
    for note in adds {
        r.engine.send(AudioCommand::AddMidiNote { clip_id, note });
    }
}

pub fn step_ticks_for(steps_per_bar: u32, time_sig_num: u8) -> u64 {
    if steps_per_bar == 0 {
        return 0;
    }
    let ticks_per_bar = TICKS_PER_QUARTER_NOTE * time_sig_num as u64;
    ticks_per_bar / steps_per_bar as u64
}


#[cfg(test)]
mod tests {
    use super::*;
    use resonance_audio::types::ClipId;

    fn clip_with_notes(notes: Vec<MidiNote>) -> MidiClipState {
        MidiClipState {
            id: 1 as ClipId,
            track_id: 1,
            start_sample: 0,
            duration_ticks: 16 * 120,
            name: "t".into(),
            notes,
            trim_start_ticks: 0,
            trim_end_ticks: 0,
        }
    }

    #[test]
    fn euclid_edits_removes_only_target_pad() {
        let notes = vec![
            MidiNote {
                note: 36,
                velocity: 0.9,
                start_tick: 0,
                duration_ticks: 10,
            },
            MidiNote {
                note: 38,
                velocity: 0.9,
                start_tick: 10,
                duration_ticks: 10,
            },
            MidiNote {
                note: 36,
                velocity: 0.9,
                start_tick: 20,
                duration_ticks: 10,
            },
        ];
        let clip = clip_with_notes(notes);
        let new_notes = vec![MidiNote {
            note: 36,
            velocity: 0.8,
            start_tick: 5,
            duration_ticks: 10,
        }];
        let (removals, adds) = euclid_edits_for(&clip, 36, new_notes);
        // Removals target indices 0 and 2 (note 36), sorted descending.
        assert_eq!(removals, vec![2, 0]);
        assert_eq!(adds.len(), 1);
        assert_eq!(adds[0].note, 36);
    }

    #[test]
    fn step_ticks_16_in_44_is_120() {
        // TPQN=480, 4 beats/bar, 16 steps/bar → 480*4/16 = 120
        assert_eq!(step_ticks_for(16, 4), 120);
    }

    #[test]
    fn step_ticks_zero_is_zero() {
        assert_eq!(step_ticks_for(0, 4), 0);
    }

    #[test]
    fn humanize_edits_selected_pad_only_touches_matching_notes() {
        let notes = vec![
            MidiNote {
                note: 36,
                velocity: 0.9,
                start_tick: 0,
                duration_ticks: 120,
            },
            MidiNote {
                note: 38,
                velocity: 0.9,
                start_tick: 120,
                duration_ticks: 120,
            },
            MidiNote {
                note: 36,
                velocity: 0.9,
                start_tick: 240,
                duration_ticks: 120,
            },
        ];
        let clip = clip_with_notes(notes);
        let params = HumanizeParams {
            velocity_amount: 0.5,
            timing_amount: 0.0,
            swing: 0.0,
            accent_pattern: humanize::AccentPattern::None,
            accent_amount: 0.0,
            selected_pad_note: Some(36),
            scope: HumanizeScope::SelectedPad,
            step_ticks: 120,
            steps_per_beat: 4,
            steps_per_bar: 16,
            clip_length_ticks: 120 * 16,
            seed: 1,
        };
        let (removals, adds) = humanize_edits_for(&clip, &params);
        // Only indices 0 and 2 (the note-36 hits) should be rewritten.
        assert_eq!(removals, vec![2, 0]);
        assert_eq!(adds.len(), 2);
        assert!(adds.iter().all(|n| n.note == 36));
    }
}
