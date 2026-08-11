//! Short human labels for undo-history entries (ba doc #273, todo
//! #1196).
//!
//! The undo stack is SHARED between the GUI and the control API, so a
//! remote client about to undo has to be able to see what it would back
//! out first — it may not be its own edit. These labels are what
//! `edit.status` / `edit.undo` / `edit.redo` report.
//!
//! Like [`classify`](super::classify::classify) this takes only the
//! message, so it runs at the top of `update()` with no borrow
//! conflicts. Labels are deliberately coarse and honest: they name the
//! kind of edit ("track volume", "add plugin"), never a guess at
//! intent, and never an id the entry cannot vouch for.

use crate::message::Message;

/// A short label for the edit `message` performs, e.g. `"track volume"`.
pub fn describe(message: &Message) -> String {
    use crate::compose::ComposeMessage;
    use crate::message::*;

    let label = match message {
        Message::Track(t) => match t {
            TrackMessage::SetTrackVolume(..) => "track volume",
            TrackMessage::SetTrackPan(..) => "track pan",
            TrackMessage::SetMasterVolume(_) => "master volume",
            TrackMessage::ToggleMute(_) => "track mute",
            TrackMessage::ToggleSolo(_) => "track solo",
            TrackMessage::SetTrackName(..) => "rename track",
            TrackMessage::SetTrackOutput(..) => "track routing",
            TrackMessage::AddControlTrack { .. } | TrackMessage::AddTrackFromPreset(_) => {
                "add track"
            }
            TrackMessage::RequestRemoveTrack(_) | TrackMessage::ConfirmRemoveTrack => {
                "delete track"
            }
            _ => "track edit",
        },
        Message::Master(m) => match m {
            MasterMessage::AddPluginToMaster(_)
            | MasterMessage::AddPluginToMasterWithId { .. } => "add master effect",
            MasterMessage::RemovePluginFromMaster(_) => "remove master effect",
            MasterMessage::MovePluginInMaster { .. } => "reorder master effects",
            MasterMessage::ToggleMasterFxBypass => "master FX bypass",
        },
        Message::Bus(b) => match b {
            BusMessage::AddBus | BusMessage::AddBusWithId { .. } => "add bus",
            BusMessage::RemoveBus(_) => "delete bus",
            BusMessage::SetBusVolume(..) => "bus volume",
            BusMessage::SetBusPan(..) => "bus pan",
            _ => "bus edit",
        },
        Message::Mixer(m) => match m {
            MixerMessage::AddSend { .. } | MixerMessage::AddSendWithId { .. } => "add send",
            MixerMessage::RemoveSend(_) => "remove send",
            MixerMessage::SetSendLevel(..) => "send level",
            _ => "send edit",
        },
        Message::Plugin(p) => match p {
            PluginMessage::AddPluginToTrack(..) => "add plugin",
            PluginMessage::RemovePluginFromTrack(..) => "remove plugin",
            PluginMessage::SetPluginParam(..) => "plugin parameter",
            _ => "plugin edit",
        },
        Message::MidiEditor(_) => "note edit",
        Message::MidiClip(_) => "MIDI clip edit",
        Message::Clip(_) => "clip edit",
        Message::Compose(c) => match c {
            ComposeMessage::CreateSection { .. } | ComposeMessage::ConfirmCreateSection => {
                "create section"
            }
            ComposeMessage::DeleteSectionDefinition { .. } => "delete section",
            ComposeMessage::RenameSection { .. } => "rename section",
            ComposeMessage::ResizeSection { .. } => "resize section",
            _ => "arrangement edit",
        },
        Message::ChordTrack(_) => "chord edit",
        Message::Transport(_) => "transport",
        Message::VocalTuning(_) => "vocal edit",
        Message::Automation(_) => "automation edit",
        Message::Freeze(_) => "freeze",
        Message::Marker(_) | Message::MarkerUi(_) => "marker edit",
        Message::Arrangement(crate::message::ArrangementMessage::InsertBars { .. }) => {
            "insert bars"
        }
        Message::Arrangement(crate::message::ArrangementMessage::RemoveBars { .. }) => {
            "remove bars"
        }
        _ => "edit",
    };
    label.to_owned()
}
