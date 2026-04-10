use resonance_audio::types::{
    AudioCommand, ClipId, MidiNote, TICKS_PER_QUARTER_NOTE,
};

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
        DrumrollMessage::SetEuclidSteps(s) => {
            r.compose.drumroll.euclid_steps_input =
                s.chars().filter(|c| c.is_ascii_digit()).collect();
        }
        DrumrollMessage::SetEuclidHits(s) => {
            r.compose.drumroll.euclid_hits_input =
                s.chars().filter(|c| c.is_ascii_digit()).collect();
        }
        DrumrollMessage::SetEuclidRotation(s) => {
            // Allow a single leading '-' and then digits.
            let mut out = String::new();
            for (i, c) in s.chars().enumerate() {
                if i == 0 && c == '-' {
                    out.push(c);
                } else if c.is_ascii_digit() {
                    out.push(c);
                }
            }
            r.compose.drumroll.euclid_rotation_input = out;
        }

        DrumrollMessage::ToggleStep { clip_id, pad_index, step } => {
            let Some(pad) = r.compose.drumroll.pad_map.get(pad_index).cloned() else {
                return;
            };
            let steps_per_bar = r.compose.drumroll.steps_per_bar;
            let time_sig_num = r.time_sig_num;
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
            let existing = clip.notes.iter().position(|n| {
                n.note == pad.note && (n.start_tick / step_ticks) == step as u64
            });
            if let Some(note_index) = existing {
                r.engine.send(AudioCommand::RemoveMidiNote { clip_id, note_index });
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
            let steps = parse_u32(&r.compose.drumroll.euclid_steps_input, 16).max(1);
            let hits = parse_u32(&r.compose.drumroll.euclid_hits_input, 4).min(steps);
            let rotation = parse_i32(&r.compose.drumroll.euclid_rotation_input, 0);
            let velocity = r.compose.drumroll.default_velocity;

            let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) else {
                return;
            };
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
    }
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
        r.engine
            .send(AudioCommand::RemoveMidiNote { clip_id, note_index });
    }
    for note in adds {
        r.engine
            .send(AudioCommand::AddMidiNote { clip_id, note });
    }
}

pub fn step_ticks_for(steps_per_bar: u32, time_sig_num: u8) -> u64 {
    if steps_per_bar == 0 {
        return 0;
    }
    let ticks_per_bar = TICKS_PER_QUARTER_NOTE * time_sig_num as u64;
    ticks_per_bar / steps_per_bar as u64
}

fn parse_u32(s: &str, default: u32) -> u32 {
    s.parse().unwrap_or(default)
}

fn parse_i32(s: &str, default: i32) -> i32 {
    s.parse().unwrap_or(default)
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
            MidiNote { note: 36, velocity: 0.9, start_tick: 0, duration_ticks: 10 },
            MidiNote { note: 38, velocity: 0.9, start_tick: 10, duration_ticks: 10 },
            MidiNote { note: 36, velocity: 0.9, start_tick: 20, duration_ticks: 10 },
        ];
        let clip = clip_with_notes(notes);
        let new_notes = vec![
            MidiNote { note: 36, velocity: 0.8, start_tick: 5, duration_ticks: 10 },
        ];
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
}
