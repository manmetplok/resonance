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
pub fn classify(message: &crate::message::Message) -> UndoAction {
    use crate::compose::ComposeMessage;
    use crate::message::*;

    match message {
        // Meta-messages never reach the classifier — update() handles
        // them before calling classify — but be defensive.
        Message::Undo | Message::Redo => UndoAction::Skip,

        // Window close request (it carries a window id, not a message
        // enum): pure UI flow, no project mutation. The timer tick is
        // engine runtime.
        Message::WindowCloseRequested(..) | Message::Tick => UndoAction::Skip,

        // Every sub-message enum classifies itself exhaustively, beside
        // its handler (ARCH-06 A6-4) — the reasoning for each variant
        // lives there.
        Message::Control(m) => m.undo_action(),
        Message::Viewport(m) => m.undo_action(),
        Message::Ui(m) => m.undo_action(),
        Message::ProjectIo(m) => m.undo_action(),
        Message::Export(m) => m.undo_action(),
        Message::Import(m) => m.undo_action(),
        Message::Relink(m) => m.undo_action(),
        Message::Browser(m) => m.undo_action(),
        Message::Drag(m) => m.undo_action(),
        // Audio import + placement (doc #175, todo #598) is one undoable
        // action. Recording here — before the import command is even sent —
        // captures the pre-import project (no pool asset, no placed clip, no
        // spawned track); the asset lands asynchronously and mutates state
        // via the engine-event path, which never records undo. So one undo
        // of this single snapshot removes the whole import + placement. Both
        // the pool-only and place variants are reversible (a pool asset
        // rides the `ProjectFile` snapshot just like a clip does).
        //
        // The two entry-point helpers (ba todo #608) are pure intent
        // signals — no state changes at their dispatch time — so they are
        // skipped here. `PickFiles` opens the OS dialog and returns a task
        // that fires `ImportFilesToPool` later (recorded then). `WindowAudioDrop`
        // re-dispatches `ImportAndPlace` inside the handler (recorded then).
        Message::Pool(PoolMessage::PickFiles) | Message::Pool(PoolMessage::WindowAudioDrop(_)) => {
            UndoAction::Skip
        }
        Message::Pool(_) => UndoAction::Record,
        Message::Group(m) => m.undo_action(),
        Message::GlobalTrack(GlobalTrackMessage::SelectEvent(_)) => UndoAction::Skip,
        Message::GlobalTrack(GlobalTrackMessage::StartTempoDrag(_)) => UndoAction::Begin,
        Message::GlobalTrack(GlobalTrackMessage::EndTempoDrag) => UndoAction::Commit,
        Message::GlobalTrack(GlobalTrackMessage::UpdateTempoEvent { .. }) => UndoAction::Skip,
        Message::GlobalTrack(_) => UndoAction::Record,

        // Every chord-track edit is a discrete action (no drag gestures
        // reach the update layer — todo #441), so each records one entry.
        Message::ChordTrack(_) => UndoAction::Record,

        Message::Transport(m) => m.undo_action(),

        // A bar shift touches every timeline collection; one entry.
        Message::Arrangement(_) => UndoAction::Record,
        Message::Marker(m) => m.undo_action(),

        Message::MarkerUi(m) => m.undo_action(),

        Message::Track(t) => match t {
            TrackMessage::SetTrackVolume(id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::TrackVolume(*id))
            }
            TrackMessage::SetTrackPan(id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::TrackPan(*id))
            }
            TrackMessage::SetMasterVolume(_) => {
                UndoAction::RecordCoalesced(CoalesceKey::MasterVolume)
            }
            TrackMessage::ToggleSubTracksVisible(_) => UndoAction::Skip,
            // Dismissing the delete-confirmation dialog is a transient
            // UI gesture — nothing to undo.
            TrackMessage::CancelRemoveTrack => UndoAction::Skip,
            // Only asks: it opens the confirm dialog, or — for an empty
            // track — re-dispatches `ConfirmRemoveTrack`, which records
            // the delete (code review STATE-13).
            TrackMessage::RequestRemoveTrack(_) => UndoAction::Skip,
            // Preset operations that don't mutate project state: a
            // preset is a file on the machine, and saving one leaves the
            // project exactly as it was (ba todo #1303). The prompt
            // around it is transient UI for the same reason.
            TrackMessage::DeleteUserPreset(_)
            | TrackMessage::SaveTrackAsPreset { .. }
            | TrackMessage::OpenSavePresetPrompt(_)
            | TrackMessage::SetSavePresetName(_)
            | TrackMessage::CloseSavePresetPrompt => UndoAction::Skip,
            _ => UndoAction::Record,
        },

        Message::ExternalInstrument(e) => match e {
            // Runtime-only: re-checking devices / re-scanning hardware /
            // revealing the user definitions folder / re-scanning definitions /
            // auto-detecting latency all mutate no project state. The measured
            // offset a detect eventually produces arrives as a separate engine
            // event (mirrored into runtime-only state), not this message.
            ExternalInstrumentMessage::CheckDevices(_)
            | ExternalInstrumentMessage::RescanDevices
            | ExternalInstrumentMessage::RevealUserDefinitionsFolder
            | ExternalInstrumentMessage::RescanDefinitions
            | ExternalInstrumentMessage::DetectLatency(_) => UndoAction::Skip,
            // Every config change (enable/disable, route, patch, latency,
            // monitor, arm, playback source) is a user-meaningful,
            // reversible edit. The playback-source *auto-switch* after a
            // recorded take is event-driven (`RecordingFinished`), not a
            // message, so it never lands an undo entry of its own — only
            // the explicit inspector toggle does.
            _ => UndoAction::Record,
        },

        Message::Bus(b) => match b {
            BusMessage::SetBusVolume(id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::BusVolume(*id))
            }
            BusMessage::SetBusPan(id, _) => UndoAction::RecordCoalesced(CoalesceKey::BusPan(*id)),
            _ => UndoAction::Record,
        },

        Message::Mixer(m) => m.undo_action(),
        Message::Freeze(m) => m.undo_action(),

        Message::Master(_) => UndoAction::Record,

        Message::Plugin(m) => m.undo_action(),

        Message::Automation(m) => m.undo_action(),

        // Take-lane comping (doc #165, todo #411). Every variant is one
        // discrete, atomic edit — there is no gesture here; the canvas
        // drag that *chooses* a promote range is todo #414's and commits
        // by emitting a single `PromoteTakeSegment`. Take groups ride the
        // `ProjectFile` snapshot (todo #412), so the generic Record path
        // reverses a comp edit with no per-message capture; the engine is
        // driven back by `replay_take_groups`'s `RestoreTakeGroups` on
        // both restore paths (todo #1394).
        //
        // An edit that would change nothing never reaches here: it is
        // dropped by `take_edit_is_refused` before `record_undo` runs, so
        // no vacuous entry is recorded.
        Message::Take(_) => UndoAction::Record,

        Message::Clip(m) => m.undo_action(),

        Message::MidiClip(m) => m.undo_action(),

        Message::MidiEditor(m) => m.undo_action(),

        Message::VocalTuning(m) => m.undo_action(),

        Message::Compose(c) => match c {
            // Form input, selections, panel open/close: UI only.
            ComposeMessage::OpenCreateSectionDialog
            | ComposeMessage::CancelCreateSectionDialog
            | ComposeMessage::SetNewSectionName(_)
            | ComposeMessage::SetNewSectionLength(_)
            | ComposeMessage::OpenEditSectionDialog { .. }
            | ComposeMessage::CancelEditSectionDialog
            | ComposeMessage::SetEditSectionName(_)
            | ComposeMessage::SetEditSectionLength(_)
            | ComposeMessage::SelectSectionPlacement { .. }
            | ComposeMessage::SelectChord { .. }
            | ComposeMessage::ClearChordSelection
            | ComposeMessage::SelectLane(_)
            | ComposeMessage::ToggleRailPanel(_)
            | ComposeMessage::ToggleWorkspaceGroup(_)
            | ComposeMessage::ExpandTrack { .. }
            | ComposeMessage::CollapseTrack
            | ComposeMessage::ExpandedScrollX(_)
            | ComposeMessage::ExpandedScrollY(_)
            | ComposeMessage::WorkspaceScrolled { .. }
            | ComposeMessage::ExpandedZoomY(_) => UndoAction::Skip,

            // Selecting an arrangement entry is pure UI state (the right-rail
            // inspector focus); it never mutates the project.
            ComposeMessage::Arrangement(
                crate::compose::messages::ArrangementMessage::SelectEntry { .. },
            ) => UndoAction::Skip,

            // Drum-ribbon span selection and the Expression dock's tool
            // state (active curve, pen, snap) are view state too (code
            // review VIEW-18): recording them wiped the redo stack.
            ComposeMessage::SelectArrangementEntry(_)
            | ComposeMessage::Expression {
                msg:
                    crate::compose::messages::ExpressionMessage::SelectCurve(_)
                    | crate::compose::messages::ExpressionMessage::SetPenMode(_)
                    | crate::compose::messages::ExpressionMessage::SetSnap(_),
                ..
            } => UndoAction::Skip,

            // A vocal render finishing is the tail of the edit that queued
            // it, seconds later — not a new edit, so it must not clear the
            // redo stack an undo in the meantime filled (VIEW-18). An
            // accepted install still marks the project dirty and bumps the
            // revision in its handler: it changed the project's clips.
            ComposeMessage::VocalAudioReady(_)
            | ComposeMessage::VocalAudioFailed { .. }
            | ComposeMessage::VocalAudioUnavailable { .. } => {
                UndoAction::Skip
            }

            // Everything else in Compose mutates project state.
            _ => UndoAction::Record,
        },

        Message::Reference(m) => m.undo_action(),
    }
}
