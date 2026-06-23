/// Update logic and subscription for the Resonance application.
use crate::message::*;
use iced::{keyboard, Subscription, Task};

/// Tick interval (ms) for the subscription timer that drains engine events.
pub const TICK_INTERVAL_MS: u64 = 16;

pub mod bus;
pub mod clips;
pub mod compose;
pub mod export;
pub mod gates;
pub mod global_track;
pub mod marker;
pub mod master;
pub mod midi_clip;
pub mod midi_editor;
pub mod plugin;
pub mod project_io;
pub mod reference;
pub mod tick;
pub mod track;
pub mod transport;
pub mod ui;
pub mod viewport;

pub(crate) use project_io::{build_project_file, replay_loaded_project, try_diff_replay};

impl crate::Resonance {
    /// Public entry point invoked by Iced on every message. Wraps the
    /// real orchestrator so derived view state (the transport label
    /// cache) is re-synced after *every* dispatch path — including the
    /// gate and undo/redo early returns — keeping `view()` strictly
    /// read-only.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.update_inner(message);
        // Iced repaints after each update, so refreshing here means the
        // labels are always exact at paint time (no one-frame staleness)
        // without the view layer ever writing state. No-op when the
        // label inputs (playhead, sig, key, loop, bpm) are unchanged.
        self.refresh_transport_labels();
        task
    }

    /// The actual orchestrator: pre-dispatch gates, meta-message
    /// shortcut, undo bookkeeping, dispatch, post-dispatch transaction
    /// commit. The two side helpers live in `update/gates.rs`
    /// (`gates_message`) and `undo.rs` (`record_undo`, alongside the
    /// message classifier).
    fn update_inner(&mut self, message: Message) -> Task<Message> {
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
            Message::Marker(m) => marker::handle(self, m),
            Message::Track(m) => track::handle(self, m),
            Message::Bus(m) => bus::handle(self, m),
            Message::Master(m) => master::handle(self, m),
            Message::Clip(m) => clips::handle(self, m),
            Message::MidiClip(m) => midi_clip::handle(self, m),
            Message::MidiEditor(m) => midi_editor::handle(self, m),
            Message::Plugin(m) => plugin::handle(self, m),
            Message::Viewport(m) => viewport::handle(self, m),
            Message::ProjectIo(m) => project_io::handle(self, m),
            Message::Reference(m) => reference::handle(self, m),
            Message::Export(m) => export::handle(self, m),
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
        let tick = iced::time::every(std::time::Duration::from_millis(TICK_INTERVAL_MS))
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
                        // `F` toggles full-screen Performance mode in/out.
                        // Manual only — never auto-opens on record-arm and
                        // never disturbs transport state. Routed through
                        // `RequestPerformanceToggle` so the toggle is
                        // suppressed while a text field is focused (this
                        // subscription fires even mid-edit; see `crate::focus`).
                        keyboard::Key::Character(ref c)
                            if c.as_str() == "f" || c.as_str() == "F" =>
                        {
                            Some(Message::Ui(UiMessage::RequestPerformanceToggle))
                        }
                        // `Esc` leaves Performance mode (a no-op elsewhere,
                        // handled in the update so it never steals Escape
                        // from other views).
                        keyboard::Key::Named(keyboard::key::Named::Escape) => {
                            Some(Message::Ui(UiMessage::ExitPerformanceMode))
                        }
                        _ => None,
                    }
                }
            }
            _ => None,
        });
        let close_requests = iced::window::close_requests().map(Message::WindowCloseRequested);

        let mut subs = vec![tick, keys, close_requests];

        // Reference drag-drop: while the Mix view is showing, forward
        // dropped audio files (wav/flac/mp3/ogg) to the reference loader.
        // The listener is only attached in the Mix view, so a stray drop
        // in Arrange/Compose never silently loads a reference; iced diffs
        // subscriptions by recipe, so it attaches/detaches as the view
        // mode changes.
        if matches!(self.view_mode, crate::state::ViewMode::Mixer) {
            subs.push(reference_file_drop());

            // Momentary A/B audition: while the reference rail is open with
            // a reference selected, holding `B` monitors the reference and
            // releasing it returns to the prior source. Gated on those
            // conditions (and attached only here) so the key never hijacks
            // typing elsewhere; iced re-diffs subscriptions as state changes.
            if self.mixer.reference_panel_open && self.reference.active_id.is_some() {
                subs.push(reference_momentary_keys());
            }
        }

        Subscription::batch(subs)
    }
}

/// The held key that momentarily auditions the active reference (press →
/// monitor reference, release → restore the prior source).
const MOMENTARY_AUDITION_KEY: &str = "b";

/// Press-and-hold reference audition on [`MOMENTARY_AUDITION_KEY`]. Emits
/// `MomentaryAudition(true)` on key-down and `(false)` on key-up; key-repeat
/// down events are idempotent (the handler guards the restore target). Only
/// attached while the reference rail is open with a reference selected, so it
/// can't steal the key from other surfaces.
fn reference_momentary_keys() -> Subscription<Message> {
    use crate::reference::ReferenceMessage;

    fn is_momentary_key(key: &keyboard::Key) -> bool {
        matches!(key, keyboard::Key::Character(c) if c.as_str().eq_ignore_ascii_case(MOMENTARY_AUDITION_KEY))
    }

    keyboard::listen().filter_map(|event| match event {
        keyboard::Event::KeyPressed { ref key, .. } if is_momentary_key(key) => {
            Some(Message::Reference(ReferenceMessage::MomentaryAudition(true)))
        }
        keyboard::Event::KeyReleased { ref key, .. } if is_momentary_key(key) => {
            Some(Message::Reference(ReferenceMessage::MomentaryAudition(false)))
        }
        _ => None,
    })
}

/// Listen for window file-drop events and forward any dropped file whose
/// extension is an accepted audio container to
/// [`ReferenceMessage::LoadRequested`]. Non-audio drops are ignored.
fn reference_file_drop() -> Subscription<Message> {
    use crate::reference::ReferenceMessage;
    use crate::update::reference::REFERENCE_AUDIO_EXTENSIONS;

    iced::event::listen_with(|event, _status, _window| match event {
        iced::Event::Window(iced::window::Event::FileDropped(path)) => {
            let is_audio = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| {
                    REFERENCE_AUDIO_EXTENSIONS
                        .iter()
                        .any(|ext| e.eq_ignore_ascii_case(ext))
                })
                .unwrap_or(false);
            is_audio.then_some(Message::Reference(ReferenceMessage::LoadRequested(path)))
        }
        _ => None,
    })
}
