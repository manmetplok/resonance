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
            r.engine.send(AudioCommand::AddMidiNote {
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
            r.engine.send(AudioCommand::RemoveMidiNote {
                clip_id,
                note_index,
            });
        }
        MidiEditorMessage::MoveNote {
            clip_id,
            note_index,
            new_start_tick,
            new_note,
        } => {
            r.engine.send(AudioCommand::MoveMidiNote {
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
            r.engine.send(AudioCommand::ResizeMidiNote {
                clip_id,
                note_index,
                new_duration_ticks,
            });
        }
        MidiEditorMessage::SelectNote { note_index } => {
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.selected_note = note_index;
            }
        }
        MidiEditorMessage::PreviewNote(track_id, note) => {
            r.engine.send(AudioCommand::SendNoteOn {
                track_id,
                note,
                velocity: 0.8,
            });
        }
        MidiEditorMessage::StopPreview(track_id, note) => {
            r.engine.send(AudioCommand::SendNoteOff { track_id, note });
        }
        MidiEditorMessage::ScrollX(delta) => {
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.scroll_x = (editor.scroll_x + delta).max(0.0);
            }
        }
        MidiEditorMessage::ScrollY(delta) => {
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.scroll_y = (editor.scroll_y + delta).max(0.0);
            }
        }
    }
    Task::none()
}
