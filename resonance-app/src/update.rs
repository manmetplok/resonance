/// Update logic and subscription for the Resonance application.
use crate::message::*;
use iced::{keyboard, Subscription, Task};

/// Fast (active-state) tick interval (ms) for the subscription timer that
/// drains engine events; see `tick::tick_interval` for the idle rate.
pub const TICK_INTERVAL_MS: u64 = 16;

pub mod arrangement;
pub mod automation;
pub mod browser;
pub mod bus;
pub mod chord_track;
pub mod clips;
pub mod compose;
pub mod control;
pub mod drag;
pub mod export;
pub mod external_instrument;
pub mod freeze;
pub mod gates;
pub mod global_track;
pub mod group;
pub mod marker;
pub mod marker_ui;
pub mod import;
pub mod keymap;
pub mod master;
pub mod midi_clip;
pub mod midi_editor;
pub mod mixer;
pub mod palette;
pub mod plugin;
pub mod plugin_replace;
pub mod pool;
pub mod project_io;
pub mod reference;
pub mod relink;
pub mod shortcuts;
pub mod takes;
pub mod tempo_reanchor;
pub mod tick;
pub mod track;
pub mod transport;
pub mod transport_nav;
pub mod ui;
pub mod viewport;
pub mod vocal_tuning;

pub(crate) use project_io::{
    build_project_file, plugin_states_for_save, replay_loaded_project,
};

impl crate::Resonance {
    /// Public entry point invoked by Iced on every message. Wraps the
    /// real orchestrator so derived view state (the transport label
    /// cache) is re-synced after *every* dispatch path — including the
    /// gate and undo/redo early returns — keeping `view()` strictly
    /// read-only.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        // A GUI message that changes what Delete on the timeline would
        // remove grants the timeline the keyboard (code review FU-C2).
        // Decided at the outermost call only; a control call (which may
        // select a global event as an internal step) never grants — a
        // remote client must not aim the user's next Backspace.
        let outermost = self.update_depth == 0;
        let grants = outermost && !matches!(message, Message::Control(_));
        let before = grants.then(|| self.timeline_delete_targets());
        self.update_depth += 1;
        let task = self.update_inner(message);
        self.update_depth -= 1;
        if let Some(before) = before {
            let after = self.timeline_delete_targets();
            let selected = after.0.is_some() || after.1.is_some() || after.2.is_some();
            if selected && after != before {
                self.ui.interaction.timeline_key_grant =
                    self.ui.interaction.timeline_key_grant.wrapping_add(1);
            }
        }
        // Iced repaints after each update, so refreshing here means the
        // labels are always exact at paint time (no one-frame staleness)
        // without the view layer ever writing state. No-op when the
        // label inputs (playhead, sig, key, loop, bpm) are unchanged.
        self.refresh_transport_labels();
        task
    }

    /// The signature track, comparable.
    fn signature_fingerprint(&self) -> Vec<(u32, u8, u8)> {
        self.signature_events
            .iter()
            .map(|e| (e.bar, e.numerator, e.denominator))
            .collect()
    }

    /// What Delete on the timeline acts on.
    fn timeline_delete_targets(
        &self,
    ) -> (
        Option<resonance_audio::types::ClipId>,
        Option<resonance_audio::types::ClipId>,
        Option<crate::state::SelectedGlobalEvent>,
    ) {
        (
            self.ui.interaction.selected_clip,
            self.ui.interaction.selected_midi_clip,
            self.ui.interaction.selected_global_event,
        )
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
        // Read-only gating of frozen inputs (ba todo #576): an edit aimed
        // at a frozen track's inputs (notes, lyrics, plugin params,
        // instrument selection) must not mutate state or enter the undo
        // stack. Drop it before `record_undo`/`dispatch` and invalidate the
        // freeze to `Stale` so the UI surfaces the refreeze affordance.
        if let Some(track_id) = self.frozen_input_edit_target(&message) {
            if self.freeze.status(track_id).is_frozen() {
                self.invalidate_frozen_track(track_id);
                return Task::none();
            }
        }
        let commit_after = self.record_undo(&message);
        // A signature change re-measures every section; its chords are
        // revalidated against the new length in the same dispatch
        // (code review FU-V2b).
        let meter_before = matches!(message, Message::Transport(_) | Message::GlobalTrack(_))
            .then(|| self.signature_fingerprint());
        let mut task = self.dispatch(message);
        if meter_before.is_some_and(|before| before != self.signature_fingerprint()) {
            let revalidate = crate::update::compose::revalidate_chords_after_meter_change(self);
            task = Task::batch([task, revalidate]);
        }
        if commit_after {
            self.commit_undo_gesture();
        }
        // Compose regeneration, bar shifts, tempo edits and engine echoes
        // (drained on Tick) reshape a frozen track's content without
        // passing the gate above (code review UPD-05).
        self.revalidate_frozen_content();
        task
    }

    /// Message router. Each message variant is delegated to the handler
    /// module that owns its concern. See `update/*.rs` for the per-domain
    /// logic.
    pub(crate) fn dispatch(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Compose(m) => compose::handle(self, m),
            Message::GlobalTrack(m) => global_track::handle(self, m),
            Message::ChordTrack(m) => chord_track::handle(self, m),
            Message::Transport(m) => transport::handle(self, m),
            Message::Marker(m) => marker::handle(self, m),
            Message::Arrangement(m) => {
                use crate::message::ArrangementMessage as A;
                let outcome = match m {
                    A::InsertBars { at_bar, count } => {
                        arrangement::insert_bars(self, at_bar, count)
                    }
                    A::RemoveBars { at_bar, count } => {
                        arrangement::remove_bars(self, at_bar, count)
                    }
                };
                // The control layer reads this back to report what moved;
                // see `Resonance::last_arrangement_shift`.
                self.ui.last_arrangement_shift = Some(outcome);
                Task::none()
            }
            Message::MarkerUi(m) => marker_ui::handle(self, m),
            Message::Track(m) => track::handle(self, m),
            Message::ExternalInstrument(m) => external_instrument::handle(self, m),
            Message::Bus(m) => bus::handle(self, m),
            Message::Mixer(m) => mixer::handle(self, m),
            Message::Freeze(m) => freeze::handle(self, m),
            Message::Master(m) => master::handle(self, m),
            Message::Clip(m) => clips::handle(self, m),
            Message::MidiClip(m) => midi_clip::handle(self, m),
            Message::MidiEditor(m) => midi_editor::handle(self, m),
            Message::VocalTuning(m) => vocal_tuning::handle(self, m),
            Message::Plugin(m) => plugin::handle(self, m),
            Message::Automation(m) => automation::handle(self, m),
            Message::Take(m) => takes::handle(self, m),
            Message::Viewport(m) => viewport::handle(self, m),
            Message::ProjectIo(m) => project_io::handle(self, m),
            Message::Group(m) => group::handle(self, m),
            Message::Reference(m) => reference::handle(self, m),
            Message::Export(m) => export::handle(self, m),
            Message::Import(m) => import::handle(self, m),
            Message::Pool(m) => pool::handle(self, m),
            Message::Relink(m) => relink::handle(self, m),
            Message::Ui(m) => ui::handle(self, m),
            Message::Browser(m) => browser::handle(self, m),
            Message::Drag(m) => drag::handle(self, m),
            Message::Control(m) => control::handle(self, m),
            Message::Tick => tick::handle_tick(self),
            Message::WindowCloseRequested(id) => {
                if self.session.dirty && self.io.has_active_project {
                    self.modals.confirm_quit = Some(id);
                    Task::none()
                } else {
                    project_io::recovery::close_session(self);
                    self.engine.shutdown(std::time::Duration::from_millis(150));
                    iced::window::close(id)
                }
            }
            // Handled by `update()` before dispatch is called.
            Message::Undo | Message::Redo => Task::none(),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        // Two-rate tick: frame rate while anything animates or a job is
        // in flight, a slow idle rate otherwise (still draining engine
        // events). iced diffs subscriptions by state, so the rate follows
        // `tick::tick_interval` as activity starts/stops.
        let tick = iced::time::every(tick::tick_interval(self)).map(|_| Message::Tick);
        // Every key press goes to the registry dispatcher
        // (`update::shortcuts`), which needs to know whether a widget
        // already consumed it — hence `listen_with` (it sees captured
        // events too) rather than `keyboard::listen` (ignored ones only).
        let keys = iced::event::listen_with(|event, status, _window| {
            shortcuts::key_event_message(event, status)
        });
        let close_requests = iced::window::close_requests().map(Message::WindowCloseRequested);

        // Window-level file drops: dragging a `.mid`/`.midi` over the
        // window opens the Import modal (showing its drop target) and a
        // drop kicks off the parse. Non-MIDI files are ignored. These
        // route through `update()` like any other message, so the
        // startup-modal gate blocks them until a project is open.
        let file_drops = iced::window::events().filter_map(|(_id, event)| match event {
            iced::window::Event::FileHovered(path) => {
                import::is_midi_path(&path).then_some(Message::Import(ImportMessage::HoverFile))
            }
            iced::window::Event::FileDropped(path) => import::is_midi_path(&path)
                .then_some(Message::Import(ImportMessage::FileDropped(path))),
            iced::window::Event::FilesHoveredLeft => {
                Some(Message::Import(ImportMessage::HoverLeft))
            }
            _ => None,
        });

        let mut subs = vec![tick, keys, close_requests, file_drops];

        // Control-endpoint bridge (doc #265, todo #1147): stream every
        // socket-thread event (connects, disconnects, parsed requests)
        // into `update()` as `Message::Control`. Only attached when the
        // listener actually started, so a disabled endpoint costs
        // nothing; `Subscription::run` keys the recipe on the builder fn,
        // so the stream is created once and lives for the whole app run.
        if self.control.server.is_some() {
            subs.push(Subscription::run(crate::control_socket::bridge_stream));
        }

        // Audio file drop for the Arrange view: when an audio file is
        // dragged from the OS onto the arrangement window, drop it onto a
        // new audio track at the current playhead position. The listener is
        // only attached in the Arrange view so a stray drop in Mixer/Compose
        // never silently imports audio into the pool; iced diffs
        // subscriptions by recipe, so it attaches/detaches as the view mode
        // changes.
        if matches!(self.ui.view_mode, crate::state::ViewMode::Arrange) {
            subs.push(arrange_audio_file_drop());
        }

        // Reference drag-drop: while the Mix view is showing, forward
        // dropped audio files (wav/flac/mp3/ogg) to the reference loader.
        // The listener is only attached in the Mix view, so a stray drop
        // in Arrange/Compose never silently loads a reference; iced diffs
        // subscriptions by recipe, so it attaches/detaches as the view
        // mode changes.
        if matches!(self.ui.view_mode, crate::state::ViewMode::Mixer) {
            subs.push(reference_file_drop());

            // Momentary A/B audition: while the reference rail is open with
            // a reference selected, holding `B` monitors the reference and
            // releasing it returns to the prior source. Gated on those
            // conditions (and attached only here) so the key never hijacks
            // typing elsewhere; iced re-diffs subscriptions as state changes.
            if self.ui.mixer.reference_panel_open && self.reference.active_id.is_some() {
                subs.push(reference_momentary_keys());
            }
        }

        Subscription::batch(subs)
    }
}

/// Wrap a shortcut that is also a typing key so it only fires when no text
/// field holds focus (see [`UiMessage::RequestShortcut`]).
fn focus_gated(message: Message) -> Message {
    Message::Ui(UiMessage::RequestShortcut(Box::new(message)))
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
    keyboard::listen().filter_map(momentary_audition_message)
}

/// The message a key event maps to for the momentary reference audition.
/// The press is focus-gated — typing "b" into a field must not audition
/// the reference (UPD-11); the release is not, and is a no-op in the
/// handler when its press never took effect.
pub fn momentary_audition_message(event: keyboard::Event) -> Option<Message> {
    use crate::reference::ReferenceMessage;

    fn is_momentary_key(key: &keyboard::Key) -> bool {
        matches!(key, keyboard::Key::Character(c) if c.as_str().eq_ignore_ascii_case(MOMENTARY_AUDITION_KEY))
    }

    match event {
        // Bare B only: ⌘B bounces, ⌥⌘B toggles the browser.
        keyboard::Event::KeyPressed {
            ref key, modifiers, ..
        } if is_momentary_key(key) && modifiers.is_empty() => Some(focus_gated(
            Message::Reference(ReferenceMessage::MomentaryAudition(true)),
        )),
        keyboard::Event::KeyReleased { ref key, .. } if is_momentary_key(key) => {
            Some(Message::Reference(ReferenceMessage::MomentaryAudition(false)))
        }
        _ => None,
    }
}

/// Listen for window file-drop events while in the Arrange view and forward
/// any audio file (wav/flac/mp3/ogg) to [`PoolMessage::WindowAudioDrop`] so
/// the handler can place it on a new audio track at the current playhead.
/// Non-audio drops are ignored. Only active in the Arrange view so a drop in
/// the Mixer never silently imports into the pool instead of the reference.
fn arrange_audio_file_drop() -> Subscription<Message> {
    use crate::update::pool::is_pool_audio_path;

    iced::event::listen_with(|event, _status, _window| match event {
        iced::Event::Window(iced::window::Event::FileDropped(path)) => {
            is_pool_audio_path(&path)
                .then_some(Message::Pool(PoolMessage::WindowAudioDrop(path)))
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
