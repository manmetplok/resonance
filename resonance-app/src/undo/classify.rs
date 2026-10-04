//! Message classification for the undo system.
//!
//! `classify` maps every incoming `Message` to a `UndoAction` that tells
//! `record_undo` what (if anything) to do with the history stack. The
//! function intentionally takes only the message — no mutable `Resonance`
//! borrow — so it can run at the very top of `update()` without borrow
//! conflicts.

use super::snapshot::CoalesceKey;

/// What the undo system should do with an incoming message. Computed from
/// the message variant alone — no access to state — so it runs at the
/// top of `update()` with no borrow conflicts.
#[derive(Debug, Clone)]
pub enum UndoAction {
    /// Don't touch the history. UI-only, engine echoes, mid-gesture
    /// updates, transient runtime messages.
    Skip,
    /// Record a new atomic undo entry, capturing the pre-dispatch state.
    Record,
    /// Record an entry that coalesces with subsequent edits using the
    /// same `CoalesceKey` — for fader/knob bursts.
    RecordCoalesced(CoalesceKey),
    /// Open a transaction: snapshot the pre-gesture state. Committed by
    /// the matching gesture-end message.
    Begin,
    /// Commit the pending transaction opened by an earlier `Begin`.
    ///
    /// The commit records an entry only when the gesture changed the
    /// snapshot (`commit_undo_gesture`, code review STATE-07). That is
    /// complete for every gesture classified `Begin` below (FU-M4c): the
    /// persisted state they edit — clip position/trim/fade/gain, MIDI clip
    /// position/trim, the loop range, tempo events, automation
    /// breakpoints — is all in the snapshot, and the only other state they
    /// touch is transient interaction state (the drag handles in
    /// `ClipInteractionState` / `transport.dragging_loop`,
    /// `selected_global_event`), which is deliberately not undoable. A new
    /// gesture that edits persisted state outside the `ProjectFile` must
    /// put it in the snapshot, or its edits will leave no entry. Guarded
    /// by `tests/timeline/undo_noop_gesture.rs`.
    Commit,
}

/// Decide how an incoming message should interact with the undo history.
///
/// Every sub-message enum classifies itself in an exhaustive
/// `XMessage::undo_action` beside its handler (ARCH-06 A6-4), so a new
/// variant does not compile until someone decides what undo does with it.
/// The reasoning for each variant lives there; this only dispatches.
/// `tools/arch-invariants` keeps catch-all arms out of both places.
pub fn classify(message: &crate::message::Message) -> UndoAction {
    use crate::message::Message;

    match message {
        // Meta-messages never reach the classifier — update() handles
        // them before calling classify — but be defensive.
        Message::Undo | Message::Redo => UndoAction::Skip,
        // The window close request carries a window id, not a message
        // enum: pure UI flow, no project mutation. The timer tick is
        // engine runtime.
        Message::WindowCloseRequested(..) | Message::Tick => UndoAction::Skip,

        Message::Compose(m) => m.undo_action(),
        Message::GlobalTrack(m) => m.undo_action(),
        Message::ChordTrack(m) => m.undo_action(),
        Message::Transport(m) => m.undo_action(),
        Message::Marker(m) => m.undo_action(),
        Message::Arrangement(m) => m.undo_action(),
        Message::MarkerUi(m) => m.undo_action(),
        Message::Track(m) => m.undo_action(),
        Message::ExternalInstrument(m) => m.undo_action(),
        Message::Bus(m) => m.undo_action(),
        Message::Mixer(m) => m.undo_action(),
        Message::Freeze(m) => m.undo_action(),
        Message::Master(m) => m.undo_action(),
        Message::Clip(m) => m.undo_action(),
        Message::MidiClip(m) => m.undo_action(),
        Message::MidiEditor(m) => m.undo_action(),
        Message::MidiMap(m) => m.undo_action(),
        Message::VocalTuning(m) => m.undo_action(),
        Message::Plugin(m) => m.undo_action(),
        Message::Automation(m) => m.undo_action(),
        Message::Take(m) => m.undo_action(),
        Message::Viewport(m) => m.undo_action(),
        Message::ProjectIo(m) => m.undo_action(),
        Message::Group(m) => m.undo_action(),
        Message::Reference(m) => m.undo_action(),
        Message::Export(m) => m.undo_action(),
        Message::Import(m) => m.undo_action(),
        Message::Pool(m) => m.undo_action(),
        Message::Relink(m) => m.undo_action(),
        Message::Ui(m) => m.undo_action(),
        Message::Browser(m) => m.undo_action(),
        Message::Drag(m) => m.undo_action(),
        Message::Control(m) => m.undo_action(),
    }
}
