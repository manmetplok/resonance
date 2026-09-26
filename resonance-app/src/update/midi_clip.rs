use iced::Task;
use resonance_audio::types::{AudioCommand, ClipId};

use crate::message::Message;
use crate::state::ClipEdge;
use crate::update::clips;
use crate::Resonance;

#[derive(Debug, Clone)]
pub enum MidiClipMessage {
    DeleteMidiClip(ClipId),
    /// Create an empty MIDI clip with a caller-allocated id (control
    /// endpoint `notes.create_clip`, doc #265, todo #1155). The id is
    /// allocated app-side (derived-clip range) and carried to the engine
    /// via `LoadMidiClipDirect`, which echoes `MidiClipCreated { id }`;
    /// so the control reply returns the id immediately. Undoable
    /// (Record) like a clip deletion.
    CreateEmptyClip {
        clip_id: ClipId,
        track_id: resonance_audio::types::TrackId,
        start_sample: resonance_audio::types::SamplePos,
        duration_ticks: u64,
        name: String,
    },
    /// Move an existing MIDI clip to an absolute timeline position
    /// (control endpoint `notes.move_clip`, ba doc #269 FR-4). The GUI
    /// reaches the same engine command through the drag messages below;
    /// this variant exists because a remote client has no drag gesture,
    /// only a target bar. Undoable (Record).
    MoveClipTo {
        clip_id: ClipId,
        new_start_sample: resonance_audio::types::SamplePos,
    },
    StartMidiClipDrag {
        clip_id: ClipId,
        grab_offset_x: f32,
        start_x: f32,
        start_y: f32,
    },
    UpdateMidiClipDrag(f32, f32),
    EndMidiClipDrag,
    StartMidiClipTrim {
        clip_id: ClipId,
        edge: ClipEdge,
        anchor_x: f32,
    },
    UpdateMidiClipTrim(f32),
    EndMidiClipTrim,
}

pub fn handle(r: &mut Resonance, m: MidiClipMessage) -> Task<Message> {
    match m {
        MidiClipMessage::DeleteMidiClip(id) => {
            let _ = r.engine.send(AudioCommand::DeleteMidiClip { clip_id: id });
            if r.interaction.selected_midi_clip == Some(id) {
                r.interaction.selected_midi_clip = None;
            }
        }
        MidiClipMessage::CreateEmptyClip {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            name,
        } => {
            // Id-hinted create: the engine loads the clip under `clip_id`
            // (empty note list) and echoes `MidiClipCreated { clip_id }`,
            // which mirrors it into `r.midi_clips`. Same path the compose
            // generators use for their derived clips.
            let _ = r.engine.send(AudioCommand::LoadMidiClipDirect {
                clip_id,
                track_id,
                start_sample,
                duration_ticks,
                notes: Vec::new(),
                name: name.clone(),
                trim_start_ticks: 0,
                trim_end_ticks: 0,
            });
            // Mirror the empty clip into `r.midi_clips` synchronously (ba
            // todo #1162): the control `notes.create_clip` returns
            // `clip_id` immediately, and a follow-up `notes.insert` on the
            // very next request resolves the target through
            // `r.midi_clips` — before the async `MidiClipCreated` echo can
            // land. Without this the insert failed `not_found` for an id
            // the create just handed out. The echo's `clip_created` handler
            // is idempotent (skips an id already present), so the round
            // trip stays a no-op once it arrives.
            if !r.midi_clips.iter().any(|c| c.id == clip_id) {
                r.midi_clips.push(crate::state::MidiClipState {
                    id: clip_id,
                    track_id,
                    start_sample,
                    duration_ticks,
                    name,
                    notes: Vec::new(),
                    trim_start_ticks: 0,
                    trim_end_ticks: 0,
                });
            }
        }
        MidiClipMessage::MoveClipTo {
            clip_id,
            new_start_sample,
        } => {
            // Absolute reposition (control `notes.move_clip`). Mirror the
            // new start into app state and tell the engine, mirroring
            // what `end_midi_clip_drag` does at the end of a GUI drag;
            // the `MidiClipMoved` echo re-applies the same value, so the
            // round trip is idempotent. The track is unchanged — moving
            // a clip between tracks is a separate concern.
            if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
                clip.start_sample = new_start_sample;
                let new_track_id = clip.track_id;
                let _ = r.engine.send(AudioCommand::MoveMidiClip {
                    clip_id,
                    new_start_sample,
                    new_track_id,
                });
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
