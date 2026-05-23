use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::{Message, MidiClipMessage};
use crate::update::clips;
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: MidiClipMessage) -> Task<Message> {
    match m {
        MidiClipMessage::DeleteMidiClip(id) => {
            let _ = r.engine.send(AudioCommand::DeleteMidiClip { clip_id: id });
            if r.interaction.selected_midi_clip == Some(id) {
                r.interaction.selected_midi_clip = None;
            }
        }
        MidiClipMessage::StartMidiClipDrag {
            clip_id,
            grab_offset_x,
            start_x,
            start_y,
        } => {
            clips::start_midi_clip_drag(r, clip_id, grab_offset_x, start_x, start_y);
        }
        MidiClipMessage::UpdateMidiClipDrag(x, y) => {
            clips::update_midi_clip_drag(r, x, y);
        }
        MidiClipMessage::EndMidiClipDrag => {
            clips::end_midi_clip_drag(r);
        }
        MidiClipMessage::StartMidiClipTrim {
            clip_id,
            edge,
            anchor_x,
        } => {
            clips::start_midi_clip_trim(r, clip_id, edge, anchor_x);
        }
        MidiClipMessage::UpdateMidiClipTrim(x) => {
            clips::update_midi_clip_trim(r, x);
        }
        MidiClipMessage::EndMidiClipTrim => {
            clips::end_midi_clip_trim(r);
        }
    }
    Task::none()
}
