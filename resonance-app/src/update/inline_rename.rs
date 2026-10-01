//! The inline rename on a channel's name (mixer-cleanup.md §2.3, §3.1):
//! on a track or bus strip's head, and in the inspector header of a
//! track or bus.
//!
//! A double-click on the name swaps it for a `text_input`. The edit
//! buffer lives in `MixerUiState::renaming`, which also names the target
//! (a track or a bus) and the surface the double-click landed on (strip
//! or inspector). One rename is open at a time — opening another commits
//! the first — and only its surface draws the field; the same name on
//! the other surface stays plain text. Nothing touches the channel until
//! the commit, which re-enters `update()` as the existing
//! `TrackMessage::SetTrackName` or `BusMessage::RenameBus`, so undo and
//! the control API's `track.rename` / `bus.rename` stay one path.
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
//! clicked away. Switching views commits too, and so does the inspector
//! moving to another channel while its header field is open ([`settle`]):
//! the field is no longer drawn anywhere.
//!
//! Cancel: Esc. A focused `text_input` captures Esc (and unfocuses
//! itself), so the shortcut reducer hands a captured Esc to [`escape`]
//! before its "a widget captured this key" drop. An Esc the field did
//! not capture (the field lost focus some other way) closes the rename
//! and still means whatever else Esc means. Every other key the field
//! captures is dropped there, so typing a name never fires a global
//! shortcut.
//!
//! The rename never outlives its channel: [`settle`] drops it after any
//! update that removed the track or bus, and undo / redo drop it before
//! they run (the name under the field may be about to change).

use iced::Task;

use crate::message::{BusMessage, Message, TrackMessage, UiMessage};
use crate::state::{RenameState, RenameSurface, RenameTarget};
use crate::Resonance;

/// The widget id of the rename field. One rename is open at a time and
/// only its surface draws the field, so one id serves every surface.
pub(crate) fn input_id() -> iced::widget::Id {
    iced::widget::Id::new("mixer-inline-rename")
}

/// The current name of `target`, or `None` when it cannot be renamed:
/// it is gone, or it is a sub-track (named after its parent's output
/// port, so its strip and inspector offer no rename).
fn current_name(r: &Resonance, target: RenameTarget) -> Option<&str> {
    match target {
        RenameTarget::Track(id) => r
            .registry
            .tracks
            .iter()
            .find(|t| t.id == id && t.sub_track.is_none())
            .map(|t| t.name.as_str()),
        RenameTarget::Bus(id) => r
            .registry
            .busses
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.name.as_str()),
    }
}

pub(crate) fn begin(
    r: &mut Resonance,
    target: RenameTarget,
    surface: RenameSurface,
) -> Task<Message> {
    // A double-click on another name — another channel, or the same
    // channel on the other surface — commits the open one first: one
    // rename at a time.
    let mut task = Task::none();
    if r.ui
        .mixer
        .renaming
        .as_ref()
        .is_some_and(|open| open.target != target || open.surface != surface)
    {
        task = commit(r);
    }
    let Some(name) = current_name(r, target) else {
        return task;
    };
    r.ui.mixer.renaming = Some(RenameState {
        target,
        surface,
        buffer: name.to_string(),
    });
    // The double-click that opened the field landed on the name the
    // field replaces, so the pointer starts out over it. Its mouse area
    // reports any move from here on.
    r.ui.mixer.rename_hovered = true;
    Task::batch([
        task,
        iced::widget::operation::focus(input_id()),
        iced::widget::operation::select_all(input_id()),
    ])
}

pub(crate) fn input(r: &mut Resonance, text: String) {
    if let Some(open) = r.ui.mixer.renaming.as_mut() {
        open.buffer = text;
    }
}

/// Close the field and rename the channel when the trimmed buffer is a
/// real change. An empty name is refused (the field just closes).
pub(crate) fn commit(r: &mut Resonance) -> Task<Message> {
    let Some(open) = r.ui.mixer.renaming.take() else {
        return Task::none();
    };
    r.ui.mixer.rename_hovered = false;
    let name = open.buffer.trim();
    match current_name(r, open.target) {
        Some(current) if !name.is_empty() && name != current => {
            let name = name.to_string();
            r.update(match open.target {
                RenameTarget::Track(id) => Message::Track(TrackMessage::SetTrackName(id, name)),
                RenameTarget::Bus(id) => Message::Bus(BusMessage::RenameBus(id, name)),
            })
        }
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

/// Esc while a rename is open cancels it. `true` when the key is spent:
/// the field captured it, so it was typed into the field and means
/// nothing else. An Esc the field did not capture still closes a rename
/// left open (its field is no longer focused) but returns `false`, so the
/// key goes on to close the topmost overlay or window.
pub(crate) fn escape(r: &mut Resonance, captured: bool) -> bool {
    if r.ui.mixer.renaming.is_none() {
        return false;
    }
    cancel(r);
    captured
}

/// Run after every outermost update. Drops a rename whose channel is gone
/// (removed, or undone away), and commits an inspector-header rename
/// whose channel the inspector no longer shows (the selection moved by a
/// key or a control call, which no press reported): its field is drawn
/// nowhere, so it has been left — a blur. Cheap: only while a rename is
/// open.
pub(crate) fn settle(r: &mut Resonance) -> Task<Message> {
    let Some(open) = r.ui.mixer.renaming.as_ref() else {
        return Task::none();
    };
    let (target, surface) = (open.target, open.surface);
    if current_name(r, target).is_none() {
        cancel(r);
        return Task::none();
    }
    if surface == RenameSurface::Inspector && inspector_target(r) != Some(target) {
        return commit(r);
    }
    Task::none()
}

/// The channel whose name the inspector header shows as renameable: the
/// selected bus, else (no master) the selected track. Mirrors the
/// precedence `view::mixer::inspector::view` draws with.
fn inspector_target(r: &Resonance) -> Option<RenameTarget> {
    if let Some(bus) = r
        .ui
        .mixer
        .selected_bus
        .filter(|id| r.registry.busses.iter().any(|b| b.id == *id))
    {
        return Some(RenameTarget::Bus(bus));
    }
    if r.ui.mixer.selected_master {
        return None;
    }
    r.ui.interaction.selected_track.map(RenameTarget::Track)
}

/// The subscription mapper while a rename is open: every press, captured
/// or not (a button elsewhere captures its own press, and a layer above
/// the strips captures every press on it).
pub fn pointer_event(event: &iced::Event) -> Option<Message> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_))
        | iced::Event::Touch(iced::touch::Event::FingerPressed { .. }) => {
            Some(Message::Ui(UiMessage::RenamePointer))
        }
        _ => None,
    }
}
