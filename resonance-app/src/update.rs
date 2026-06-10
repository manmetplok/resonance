/// Update logic and subscription for the Resonance application.
use crate::message::*;
use crate::theme;
use iced::{keyboard, Subscription, Task};

pub mod bus;
pub mod clips;
pub mod compose;
pub mod gates;
pub mod global_track;
pub mod master;
pub mod midi_clip;
pub mod midi_editor;
pub mod plugin;
pub mod project_io;
pub mod tick;
pub mod track;
pub mod transport;
pub mod ui;
pub mod viewport;

pub(crate) use project_io::{build_project_file, replay_loaded_project, try_diff_replay};

impl crate::Resonance {
    /// Public entry point invoked by Iced on every message. Reads as a
    /// small orchestrator: pre-dispatch gates, meta-message shortcut,
    /// undo bookkeeping, dispatch, post-dispatch transaction commit.
    /// The two side helpers live in `update/gates.rs` (`gates_message`)
    /// and `undo.rs` (`record_undo`, alongside the message classifier).
    pub fn update(&mut self, message: Message) -> Task<Message> {
        if self.gates_message(&message) {
            return Task::none();
        }
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
        let commit_after = self.record_undo(&message);
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
            Message::Tick => tick::handle_tick(self),
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

    pub fn subscription(&self) -> Subscription<Message> {
        let tick = iced::time::every(std::time::Duration::from_millis(theme::TICK_INTERVAL_MS))
            .map(|_| Message::Tick);
        let keys = keyboard::listen().filter_map(|event| match event {
            keyboard::Event::KeyPressed {
                key, modifiers, ..
            } => {
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
                        keyboard::Key::Character(ref c) if c.as_str() == "y" => {
                            Some(Message::Redo)
                        }
                        _ => None,
                    }
                } else {
                    match key {
                        keyboard::Key::Named(keyboard::key::Named::Enter) => Some(
                            Message::MidiEditor(MidiEditorMessage::OpenSelectedMidiClip),
                        ),
                        _ => None,
                    }
                }
            }
            _ => None,
        });
        let close_requests = iced::window::close_requests().map(Message::WindowCloseRequested);
        Subscription::batch([tick, keys, close_requests])
    }
}
