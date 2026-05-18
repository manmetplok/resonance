/// Update logic and subscription for the Resonance application.
use crate::message::*;
use crate::theme;
use iced::{keyboard, Subscription, Task};

pub mod bus;
pub mod clips;
pub mod compose;
pub mod global_track;
pub mod master;
pub mod midi_clip;
pub mod midi_editor;
pub mod plugin;
pub mod project_io;
pub mod track;
pub mod transport;
pub mod ui;
pub mod viewport;

pub(crate) use project_io::{build_project_file, replay_loaded_project};

/// While the startup modal is up (no active project), swallow
/// messages that would mutate project state. Engine events don't
/// True for every user-initiated message we need to drop while a
/// bounce-in-place run is rendering. The Cancel button on the progress
/// modal is the one carve-out: that's how the user actually stops the
/// engine, so it has to flow through.
fn bounce_blocks_message(message: &Message) -> bool {
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

/// flow through `update()` (see `engine_events.rs`), so this only
/// needs to think about user-initiated variants.
fn is_gated_message(message: &Message) -> bool {
    match message {
        // Interactive user input: block.
        Message::Compose(_)
        | Message::Transport(_)
        | Message::Track(_)
        | Message::Bus(_)
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

impl crate::Resonance {
    /// Public entry point invoked by Iced on every message. Handles
    /// undo/redo meta-messages, runs the startup-modal gate, then hands
    /// off to `dispatch` with the undo-history bookkeeping wrapped
    /// around the dispatch — recording atomic entries before mutations
    /// and committing gesture transactions after.
    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        if !self.io.has_active_project && is_gated_message(&message) {
            return Task::none();
        }
        // While a bounce-in-place run is rendering, block every user
        // input that could disturb the engine — transport changes,
        // track / clip / plugin edits, view switches. The progress
        // modal's Cancel button is the one whitelisted exception.
        if self.bounce_in_progress.is_some() && bounce_blocks_message(&message) {
            return Task::none();
        }

        // Meta-messages: walk the history, don't classify.
        match message {
            Message::Undo => {
                self.try_undo();
                return Task::none();
            }
            Message::Redo => {
                self.try_redo();
                return Task::none();
            }
            _ => {}
        }

        // Classify now so we can record/begin before the mutation runs,
        // and commit after. Classification is a pure function over the
        // message shape — no borrow on `self`.
        let action = crate::undo::classify(&message);
        let commit_after = matches!(action, crate::undo::UndoAction::Commit);
        // Mark the project dirty on any state-changing action. This
        // mirrors the undo classification: any action that warrants an
        // undo entry (Record, RecordCoalesced, Begin, Commit) means the
        // project has diverged from the last saved version. The dirty
        // flag is cleared on ProjectSaved(Ok) and on project load.
        if !matches!(action, crate::undo::UndoAction::Skip) {
            self.dirty = true;
        }

        // Skip every history-mutating branch when the app isn't in a
        // state where a snapshot could be restored (no active project,
        // no saved path, mid-restore). Commit still runs on gesture end
        // even if recording was blocked — it'll be a no-op because
        // `begin` was also blocked, so there's no pending transaction.
        if self.can_record_undo() {
            match action {
                crate::undo::UndoAction::Skip | crate::undo::UndoAction::Commit => {}
                crate::undo::UndoAction::Record => {
                    let snap = self.snapshot_for_undo();
                    self.undo.record(snap);
                }
                crate::undo::UndoAction::RecordCoalesced(key) => {
                    let snap = self.snapshot_for_undo();
                    self.undo.record_coalesced(snap, key);
                }
                crate::undo::UndoAction::Begin => {
                    let snap = self.snapshot_for_undo();
                    self.undo.begin(snap);
                }
            }
        }

        let task = self.dispatch(message);

        if commit_after {
            self.undo.commit();
        }

        task
    }

    /// Message router. Each message variant is delegated to the handler
    /// module that owns its concern. See `update/*.rs` for the per-domain
    /// logic.
    fn dispatch(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Compose(m) => compose::handle(self, m),
            Message::GlobalTrack(m) => global_track::handle(self, m),
            Message::Transport(m) => transport::handle(self, m),
            Message::Track(m) => track::handle(self, m),
            Message::Bus(m) => bus::handle(self, m),
            Message::Master(m) => master::handle(self, m),
            Message::Clip(m) => clips::handle(self, m),
            Message::MidiClip(m) => midi_clip::handle(self, m),
            Message::MidiEditor(m) => midi_editor::handle(self, m),
            Message::Plugin(m) => plugin::handle(self, m),
            Message::Viewport(m) => viewport::handle(self, m),
            Message::ProjectIo(m) => project_io::handle(self, m),
            Message::Ui(m) => ui::handle(self, m),
            Message::Tick => viewport::handle_tick(self),
            Message::WindowCloseRequested(id) => {
                if self.dirty && self.io.has_active_project {
                    self.confirm_quit = Some(id);
                    Task::none()
                } else {
                    self.engine.shutdown(std::time::Duration::from_millis(150));
                    iced::window::close(id)
                }
            }
            // Handled by `update()` before dispatch is called.
            Message::Undo | Message::Redo => Task::none(),
        }
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        let tick = iced::time::every(std::time::Duration::from_millis(theme::TICK_INTERVAL_MS))
            .map(|_| Message::Tick);
        let keys = keyboard::on_key_press(|key, modifiers| {
            if modifiers.command() {
                match key {
                    keyboard::Key::Character(ref c) if c.as_str() == "s" => {
                        if modifiers.shift() {
                            Some(Message::ProjectIo(ProjectIoMessage::SaveProjectAs))
                        } else {
                            Some(Message::ProjectIo(ProjectIoMessage::SaveProject))
                        }
                    }
                    keyboard::Key::Character(ref c) if c.as_str() == "o" => {
                        Some(Message::ProjectIo(ProjectIoMessage::OpenProject))
                    }
                    keyboard::Key::Character(ref c) if c.as_str() == "z" => {
                        if modifiers.shift() {
                            Some(Message::Redo)
                        } else {
                            Some(Message::Undo)
                        }
                    }
                    keyboard::Key::Character(ref c) if c.as_str() == "y" => Some(Message::Redo),
                    _ => None,
                }
            } else {
                match key {
                    keyboard::Key::Named(keyboard::key::Named::Enter) => {
                        Some(Message::MidiEditor(MidiEditorMessage::OpenSelectedMidiClip))
                    }
                    _ => None,
                }
            }
        });
        let close_requests = iced::window::close_requests().map(Message::WindowCloseRequested);
        Subscription::batch([tick, keys, close_requests])
    }
}
