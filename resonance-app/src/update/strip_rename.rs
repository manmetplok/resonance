//! The inline rename on a mixer track strip's head (mixer-cleanup.md
//! §2.3).
//!
//! A double-click on a strip's name swaps it for a `text_input`. The edit
//! buffer lives in `MixerUiState::renaming`; nothing touches the track
//! until the commit, which re-enters `update()` as the existing
//! `TrackMessage::SetTrackName`, so undo and the control API's
//! `track_rename` stay one path.
//!
//! Commit: Enter (`on_submit`), or a press anywhere off the field.
//! iced's `text_input` has no blur callback, so while a rename is open
//! the field sits in a mouse area that reports whether the pointer is
//! over it ([`hovered`]) and the app subscribes to mouse presses
//! ([`pointer_event`]). A press while the pointer is off the field
//! commits. The decision is by pointer, not by asking the field whether
//! it still holds focus: a press on a layer above the strips (the
//! floating plugin window, a modal) never reaches the field, which would
//! then report itself focused and keep taking keys after the user
//! clicked away. Switching views commits too.
//!
//! Cancel: Esc. A focused `text_input` captures Esc (and unfocuses
//! itself), so the shortcut reducer hands a captured Esc to [`escape`]
//! before its "a widget captured this key" drop. An Esc the field did
//! not capture (the field lost focus some other way) closes the rename
//! and still means whatever else Esc means. Every other key the field
//! captures is dropped there, so typing a name never fires a global
//! shortcut.
//!
//! The rename never outlives its track: [`prune`] drops it after any
//! update that removed the track, and undo / redo drop it before they
//! run (the name under the field may be about to change).

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
    // The double-click that opened the field landed on the name the
    // field replaces, so the pointer starts out over it. Its mouse area
    // reports any move from here on.
    r.ui.mixer.rename_hovered = true;
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
    r.ui.mixer.rename_hovered = false;
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
    r.ui.mixer.rename_hovered = false;
}

/// A mouse press while the field is open: one off the field commits.
pub(crate) fn pointer(r: &mut Resonance) -> Task<Message> {
    if r.ui.mixer.renaming.is_none() || r.ui.mixer.rename_hovered {
        return Task::none();
    }
    commit(r)
}

/// The pointer entered or left the open field.
pub(crate) fn hovered(r: &mut Resonance, hovered: bool) {
    if r.ui.mixer.renaming.is_some() {
        r.ui.mixer.rename_hovered = hovered;
    }
}

/// Esc while a strip rename is open cancels it. `true` when the key is
/// spent: the field captured it, so it was typed into the field and
/// means nothing else. An Esc the field did not capture still closes a
/// rename left open (its field is no longer focused) but returns `false`,
/// so the key goes on to close the topmost overlay or window.
pub(crate) fn escape(r: &mut Resonance, captured: bool) -> bool {
    if r.ui.mixer.renaming.is_none() {
        return false;
    }
    cancel(r);
    captured
}

/// Drop a rename whose track is gone (removed, or undone away). Cheap: a
/// scan of the track list, only while a rename is open.
pub(crate) fn prune(r: &mut Resonance) {
    if let Some((track_id, _)) = r.ui.mixer.renaming.as_ref() {
        if !r.registry.tracks.iter().any(|t| t.id == *track_id) {
            cancel(r);
        }
    }
}

/// The subscription mapper while a rename is open: every press, captured
/// or not (a button elsewhere captures its own press, and a layer above
/// the strips captures every press on it).
pub fn pointer_event(event: &iced::Event) -> Option<Message> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_))
        | iced::Event::Touch(iced::touch::Event::FingerPressed { .. }) => {
            Some(Message::Ui(UiMessage::StripRenamePointer))
        }
        _ => None,
    }
}
