//! The inline rename on a mixer track strip's head (mixer-cleanup.md
//! §2.3).
//!
//! A double-click on a strip's name swaps it for a `text_input`. The edit
//! buffer lives in `MixerUiState::renaming`; nothing touches the track
//! until the commit, which re-enters `update()` as the existing
//! `TrackMessage::SetTrackName`, so undo and the control API's
//! `track_rename` stay one path.
//!
//! Commit: Enter (`on_submit`), or the field losing focus. iced's
//! `text_input` has no blur callback, so while a rename is open the app
//! subscribes to mouse presses ([`pointer_event`]) and, after each one,
//! probes whether the field still holds focus ([`input_id`]). A press
//! inside the field keeps it; a press anywhere else unfocuses it and
//! commits.
//!
//! Cancel: Esc. A focused `text_input` captures Esc (and unfocuses
//! itself), so the shortcut reducer hands Esc to [`escape`] before its
//! "a widget captured this key" drop. Every other key the field captures
//! is dropped there, so typing a name never fires a global shortcut.

use iced::Task;

use resonance_audio::types::TrackId;

use crate::message::{Message, TrackMessage, UiMessage};
use crate::Resonance;

/// The widget id of the strip rename field. One rename is open at a
/// time, so one id serves every strip.
pub(crate) fn input_id() -> iced::widget::Id {
    iced::widget::Id::new("mixer-strip-rename")
}

pub(crate) fn begin(r: &mut Resonance, track_id: TrackId) -> Task<Message> {
    // A second double-click on another strip commits the first.
    if r.ui
        .mixer
        .renaming
        .as_ref()
        .is_some_and(|(id, _)| *id != track_id)
    {
        let _ = commit(r);
    }
    let Some(track) = r.registry.tracks.iter().find(|t| t.id == track_id) else {
        return Task::none();
    };
    // Sub-tracks are named after their parent's output port; their
    // strips offer no rename.
    if track.sub_track.is_some() {
        return Task::none();
    }
    r.ui.mixer.renaming = Some((track_id, track.name.clone()));
    Task::batch([
        iced::widget::operation::focus(input_id()),
        iced::widget::operation::select_all(input_id()),
    ])
}

pub(crate) fn input(r: &mut Resonance, text: String) {
    if let Some((_, buffer)) = r.ui.mixer.renaming.as_mut() {
        *buffer = text;
    }
}

/// Close the field and rename the track when the trimmed buffer is a
/// real change. An empty name is refused (the field just closes).
pub(crate) fn commit(r: &mut Resonance) -> Task<Message> {
    let Some((track_id, buffer)) = r.ui.mixer.renaming.take() else {
        return Task::none();
    };
    let name = buffer.trim();
    let current = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .map(|t| t.name.as_str());
    match current {
        Some(current) if !name.is_empty() && name != current => r.update(Message::Track(
            TrackMessage::SetTrackName(track_id, name.to_string()),
        )),
        _ => Task::none(),
    }
}

pub(crate) fn cancel(r: &mut Resonance) {
    r.ui.mixer.renaming = None;
}

/// A mouse press while the field is open: probe its focus once the press
/// has been delivered to the widgets.
pub(crate) fn pointer(r: &Resonance) -> Task<Message> {
    if r.ui.mixer.renaming.is_none() {
        return Task::none();
    }
    iced::widget::operation::is_focused(input_id())
        .map(|focused| Message::Ui(UiMessage::StripRenameFocusProbed(focused)))
}

pub(crate) fn focus_probed(r: &mut Resonance, focused: bool) -> Task<Message> {
    if focused {
        return Task::none();
    }
    commit(r)
}

/// Esc while a strip rename is open cancels it. `true` when it did, so
/// the shortcut reducer stops there.
pub(crate) fn escape(r: &mut Resonance) -> bool {
    if r.ui.mixer.renaming.is_none() {
        return false;
    }
    cancel(r);
    true
}

/// The subscription mapper while a rename is open: every left press,
/// captured or not (a button elsewhere captures its own press but still
/// takes focus away from the field).
pub(crate) fn pointer_event(event: &iced::Event) -> Option<Message> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_))
        | iced::Event::Touch(iced::touch::Event::FingerPressed { .. }) => {
            Some(Message::Ui(UiMessage::StripRenamePointer))
        }
        _ => None,
    }
}
