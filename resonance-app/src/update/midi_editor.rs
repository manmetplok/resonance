use iced::Task;
use resonance_audio::types::{AudioCommand, MidiNote};

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
    }
    Task::none()
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
