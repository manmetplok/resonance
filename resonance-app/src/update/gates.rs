//! Pre-dispatch message gates.
//!
//! `update()` runs two pre-dispatch gates on every message: the startup
//! modal gate (no active project → block project-mutating messages) and
//! the bounce-in-progress gate (offline bounce running → block anything
//! that would disturb the engine). Both are pure functions over message
//! shape plus one piece of `Resonance` state — a "look at the message
//! variant and decide what to do" pre-pass before dispatch, kin to the
//! undo classifier in `undo.rs`.

/// While the startup modal is up (no active project), swallow messages
/// that would mutate project state. Engine events don't flow through
/// `update()` (see `engine_events.rs`), so this only needs to think
/// about user-initiated variants.
fn is_gated_message(message: &crate::message::Message) -> bool {
    use crate::message::*;
    match message {
        // Interactive user input: block.
        Message::Compose(_)
        | Message::Transport(_)
        | Message::Track(_)
        | Message::Bus(_)
        | Message::Freeze(_)
        | Message::Master(_)
        | Message::Clip(_)
        | Message::MidiClip(_)
        | Message::MidiEditor(_)
        | Message::Plugin(_)
        | Message::Viewport(_)
        | Message::GlobalTrack(_) => true,
        // Tab switches / auxiliary overlays: block so they can't
        // steal focus from the startup modal.
        Message::Ui(UiMessage::SwitchView(_))
        | Message::Ui(UiMessage::TogglePerformanceMode)
        | Message::Ui(UiMessage::RequestPerformanceToggle)
        | Message::Ui(UiMessage::PerformanceToggleResolved { .. })
        | Message::Ui(UiMessage::ExitPerformanceMode)
        | Message::Ui(UiMessage::OpenSettings)
        | Message::Ui(UiMessage::OpenAddTrackMenu) => true,
        // Benign UI: allow.
        Message::Ui(UiMessage::CloseSettings)
        | Message::Ui(UiMessage::CloseAddTrackMenu)
        | Message::Ui(UiMessage::DismissError)
        | Message::Ui(UiMessage::StartNewProject)
        | Message::Ui(UiMessage::SelectTrack(_))
        | Message::Ui(UiMessage::ConfirmSaveAndQuit)
        | Message::Ui(UiMessage::ConfirmDiscardAndQuit)
        | Message::Ui(UiMessage::CancelQuit)
        | Message::Ui(UiMessage::ToggleGlobalTracks)
        | Message::Ui(UiMessage::ToggleMixerInspectorGroup(_))
        | Message::Ui(UiMessage::ToggleMidiClockSend)
        | Message::Ui(UiMessage::SetMidiClockSendDevice(_))
        | Message::Ui(UiMessage::ToggleMidiClockRecv)
        | Message::Ui(UiMessage::SetMidiClockRecvDevice(_)) => false,
        // Project I/O drives the modal itself: always allow.
        Message::ProjectIo(_) => false,
        // Timer tick: harmless, drives VU meters — allow.
        Message::Tick => false,
        // Window close request: always allow so the app can exit.
        Message::WindowCloseRequested(_) => false,
        // Undo/redo need a project to be meaningful — block otherwise.
        Message::Undo | Message::Redo => true,
    }
}

/// True for every user-initiated message we need to drop while a
/// bounce-in-place run is rendering. The Cancel button on the progress
/// modal is the one carve-out: that's how the user actually stops the
/// engine, so it has to flow through.
fn bounce_blocks_message(message: &crate::message::Message) -> bool {
    use crate::message::*;
    match message {
        // Whitelist: cancel button on the in-progress modal.
        Message::Track(TrackMessage::Bounce(BounceMessage::CancelInProgress)) => false,
        // Engine event traffic, project I/O, and the timer tick all
        // need to keep flowing — the bounce relies on `BounceProgress`
        // / `TrackBounceCompleted` events to clear the modal.
        Message::ProjectIo(_) | Message::Tick | Message::WindowCloseRequested(_) => false,
        // Everything else: block.
        Message::Compose(_)
        | Message::Transport(_)
        | Message::Track(_)
        | Message::Bus(_)
        | Message::Freeze(_)
        | Message::Master(_)
        | Message::Clip(_)
        | Message::MidiClip(_)
        | Message::MidiEditor(_)
        | Message::Plugin(_)
        | Message::Viewport(_)
        | Message::GlobalTrack(_)
        | Message::Ui(_)
        | Message::Undo
        | Message::Redo => true,
    }
}

/// True for every user-initiated message we need to drop while a freeze
/// render is in flight (a single freeze or a "freeze all" batch). Mirrors
/// [`bounce_blocks_message`]: the offline freeze renderer shares plugin
/// instances with the live mixer, so any project mutation mid-render could
/// corrupt the cache. The Cancel button is the one carve-out so the user
/// can always stop the run.
fn freeze_blocks_message(message: &crate::message::Message) -> bool {
    use crate::message::*;
    match message {
        // Whitelist: cancelling the in-flight freeze.
        Message::Freeze(FreezeMessage::CancelFreeze) => false,
        // Engine event traffic, project I/O, and the timer tick keep
        // flowing — the freeze relies on the tick to drain `FreezeProgress`
        // / `FreezeCompleted` events that advance the batch and clear state.
        Message::ProjectIo(_) | Message::Tick | Message::WindowCloseRequested(_) => false,
        // Everything else: block.
        Message::Compose(_)
        | Message::Transport(_)
        | Message::Track(_)
        | Message::Bus(_)
        | Message::Freeze(_)
        | Message::Master(_)
        | Message::Clip(_)
        | Message::MidiClip(_)
        | Message::MidiEditor(_)
        | Message::Plugin(_)
        | Message::Viewport(_)
        | Message::GlobalTrack(_)
        | Message::Ui(_)
        | Message::Undo
        | Message::Redo => true,
    }
}

impl crate::Resonance {
    /// Combined pre-dispatch gate. Returns `true` when `message` should
    /// be dropped — either because the startup modal is up and the
    /// message would mutate project state, or because an offline bounce
    /// is in progress and the message would disturb the engine.
    pub(crate) fn gates_message(&self, message: &crate::message::Message) -> bool {
        if !self.io.has_active_project && is_gated_message(message) {
            return true;
        }
        if self.bounce_in_progress.is_some() && bounce_blocks_message(message) {
            return true;
        }
        if self.freeze.any_in_flight() && freeze_blocks_message(message) {
            return true;
        }
        false
    }
}
