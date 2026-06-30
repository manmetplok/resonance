use iced::Task;
use resonance_audio::quantize::{self, GrooveTemplate};
use resonance_audio::types::{AudioCommand, ClipId, MidiNote};

use crate::message::{Message, MidiEditorMessage};
use crate::update::clips;
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: MidiEditorMessage) -> Task<Message> {
    match m {
        MidiEditorMessage::OpenMidiEditor(clip_id) => {
            clips::open_midi_editor(r, clip_id);
        }
        MidiEditorMessage::OpenSelectedMidiClip => {
            if let Some(clip_id) = r.interaction.selected_midi_clip {
                clips::open_midi_editor(r, clip_id);
            }
        }
        MidiEditorMessage::CloseMidiEditor => {
            r.interaction.editing_midi_clip = None;
        }
        MidiEditorMessage::AddNote {
            clip_id,
            note,
            start_tick,
            duration_ticks,
            velocity,
        } => {
            let _ = r.engine.send(AudioCommand::AddMidiNote {
                clip_id,
                note: MidiNote {
                    note,
                    velocity,
                    start_tick,
                    duration_ticks,
                },
            });
        }
        MidiEditorMessage::RemoveNote {
            clip_id,
            note_index,
        } => {
            let _ = r.engine.send(AudioCommand::RemoveMidiNote {
                clip_id,
                note_index,
            });
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.clear_selection();
            }
        }
        MidiEditorMessage::RemoveSelectedNotes { clip_id } => {
            remove_selected_notes(r, clip_id);
        }
        MidiEditorMessage::MoveNote {
            clip_id,
            note_index,
            new_start_tick,
            new_note,
        } => {
            let _ = r.engine.send(AudioCommand::MoveMidiNote {
                clip_id,
                note_index,
                new_start_tick,
                new_note,
            });
        }
        MidiEditorMessage::ResizeNote {
            clip_id,
            note_index,
            new_duration_ticks,
        } => {
            let _ = r.engine.send(AudioCommand::ResizeMidiNote {
                clip_id,
                note_index,
                new_duration_ticks,
            });
        }
        MidiEditorMessage::SelectNote { note_index } => {
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.select_single(note_index);
            }
        }
        MidiEditorMessage::ToggleNoteSelection { note_index } => {
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.toggle_note(note_index);
            }
        }
        MidiEditorMessage::SelectNotesInRect { indices, additive } => {
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.apply_marquee(indices, additive);
            }
        }
        MidiEditorMessage::SelectAllNotes => {
            if let Some(clip_id) = r.interaction.editing_midi_clip.as_ref().map(|e| e.clip_id) {
                let len = r
                    .midi_clips
                    .iter()
                    .find(|c| c.id == clip_id)
                    .map(|c| c.notes.len())
                    .unwrap_or(0);
                if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                    editor.select_all(len);
                }
            }
        }
        MidiEditorMessage::ClearNoteSelection => {
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.clear_selection();
            }
        }
        MidiEditorMessage::PreviewNote(track_id, note) => {
            let _ = r.engine.send(AudioCommand::SendNoteOn {
                track_id,
                note,
                velocity: 0.8,
            });
        }
        MidiEditorMessage::StopPreview(track_id, note) => {
            let _ = r.engine.send(AudioCommand::SendNoteOff { track_id, note });
        }
        MidiEditorMessage::ScrollY(delta) => {
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.scroll_y = (editor.scroll_y + delta).max(0.0);
            }
        }
        MidiEditorMessage::ToggleSlur { clip_id, note_index } => {
            toggle_slur(r, clip_id, note_index);
        }
        MidiEditorMessage::Quantize {
            grid,
            strength,
            swing,
            mode,
            quantize_ends,
            iterative,
        } => {
            if let Some((clip_id, indices)) = bulk_target(r) {
                let _ = r.engine.send(AudioCommand::QuantizeMidiNotes {
                    clip_id,
                    indices,
                    grid,
                    strength,
                    swing,
                    mode,
                    quantize_ends,
                    iterative,
                });
            }
        }
        MidiEditorMessage::Humanize { timing, vel, seed } => {
            if let Some((clip_id, indices)) = bulk_target(r) {
                // One seed per invocation: a fresh draw when the caller
                // didn't pin one, so the jitter is reproducible within
                // this single (undoable) edit and a new invocation rolls
                // again. Tests pin `seed` for determinism.
                let seed = seed.unwrap_or_else(humanize_seed);
                let _ = r.engine.send(AudioCommand::HumanizeMidiNotes {
                    clip_id,
                    indices,
                    timing_ticks: timing,
                    vel_amt: vel,
                    seed,
                });
            }
        }
        MidiEditorMessage::ApplyGroove {
            template_id,
            strength,
        } => {
            if let Some((clip_id, indices)) = bulk_target(r) {
                // Unknown template id → no-op (no command, no undo entry).
                if let Some(template) = lookup_groove(&template_id) {
                    let _ = r.engine.send(AudioCommand::ApplyGrooveToClip {
                        clip_id,
                        indices,
                        template,
                        strength,
                    });
                }
            }
        }
        MidiEditorMessage::ExtractGroove { grid } => {
            // Extraction reads the whole open clip regardless of selection.
            if let Some(clip_id) = r.interaction.editing_midi_clip.as_ref().map(|e| e.clip_id) {
                let _ = r
                    .engine
                    .send(AudioCommand::ExtractGrooveFromClip { clip_id, grid });
            }
        }
    }
    Task::none()
}

/// Resolve the target of a bulk timing edit: the open editor clip plus
/// the note indices to operate on. Uses the current multi-note selection
/// (#389), or every note in the clip when the selection is empty
/// ("operate on the whole clip if none"). Out-of-range selection indices
/// are dropped. Returns `None` — making the op a no-op — when no clip is
/// open, the clip is empty, or no in-range notes remain.
fn bulk_target(r: &Resonance) -> Option<(ClipId, Vec<usize>)> {
    let editor = r.interaction.editing_midi_clip.as_ref()?;
    let clip_id = editor.clip_id;
    let note_count = r
        .midi_clips
        .iter()
        .find(|c| c.id == clip_id)
        .map(|c| c.notes.len())
        .unwrap_or(0);
    if note_count == 0 {
        return None;
    }
    let indices: Vec<usize> = if editor.selected_notes.is_empty() {
        (0..note_count).collect()
    } else {
        editor
            .selected_notes
            .iter()
            .copied()
            .filter(|&i| i < note_count)
            .collect()
    };
    if indices.is_empty() {
        None
    } else {
        Some((clip_id, indices))
    }
}

/// Draw one fresh, well-mixed humanize seed. Called once per `Humanize`
/// invocation (see the handler) so a single bulk edit's jitter is fixed
/// and reproducible while distinct invocations decorrelate.
fn humanize_seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    crate::util::next_seed(nanos)
}

/// Resolve a groove `template_id` to its [`GrooveTemplate`]. Matches the
/// in-code stock grooves by name; the persisted user-groove library is a
/// later slice (#395), so unknown ids return `None`.
fn lookup_groove(template_id: &str) -> Option<GrooveTemplate> {
    quantize::stock_grooves()
        .into_iter()
        .find(|(name, _)| name == template_id)
        .map(|(_, template)| template)
}

/// Remove every selected note from `clip_id`. Indices are sent to the
/// engine in descending order so each removal can't invalidate the
/// indices of the not-yet-removed notes below it. Selection is cleared
/// afterwards since the indices no longer refer to anything.
fn remove_selected_notes(r: &mut crate::Resonance, clip_id: resonance_audio::types::ClipId) {
    let Some(editor) = r.interaction.editing_midi_clip.as_ref() else {
        return;
    };
    let note_count = r
        .midi_clips
        .iter()
        .find(|c| c.id == clip_id)
        .map(|c| c.notes.len())
        .unwrap_or(0);
    // Descending order: removing a higher index never shifts a lower one.
    let mut indices: Vec<usize> = editor
        .selected_notes
        .iter()
        .copied()
        .filter(|&i| i < note_count)
        .collect();
    indices.sort_unstable_by(|a, b| b.cmp(a));

    for note_index in indices {
        let _ = r.engine.send(AudioCommand::RemoveMidiNote {
            clip_id,
            note_index,
        });
    }

    if let Some(ref mut editor) = r.interaction.editing_midi_clip {
        editor.clear_selection();
    }
}

/// Toggle the OpenUtau slur marker on the i-th note of `clip_id`. The
/// lyric side-table treats `""` as "use the next syllable from the
/// draft", so flipping to `""` reinstates the cursor-driven label
/// flow — every subsequent non-slur note slides its syllable one slot
/// left, and the now-spare syllable at the tail returns to the draft.
/// Flipping to `"+"` does the reverse: the trailing syllables slide
/// right.
fn toggle_slur(
    r: &mut crate::Resonance,
    clip_id: resonance_audio::types::ClipId,
    note_index: usize,
) {
    use resonance_music_theory::g2p;

    let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) else {
        return;
    };
    if note_index >= clip.notes.len() {
        return;
    }
    let note_count = clip.notes.len();

    let entry = r
        .compose
        .vocal_audio
        .clip_lyrics
        .entry(clip_id)
        .or_default();
    if entry.len() < note_count {
        entry.resize(note_count, String::new());
    }
    if g2p::is_slur_lyric(&entry[note_index]) {
        entry[note_index] = String::new();
    } else {
        entry[note_index] = g2p::SLUR_MARKER.to_string();
    }
}
