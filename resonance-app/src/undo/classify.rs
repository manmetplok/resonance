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
    use crate::reference::ReferenceMessage;

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

        Message::Transport(t) => match t {
            TransportMessage::StartLoopDrag(_) => UndoAction::Begin,
            TransportMessage::EndLoopDrag => UndoAction::Commit,
            TransportMessage::UpdateLoopDrag(_) => UndoAction::Skip,
            TransportMessage::Play
            | TransportMessage::Record
            | TransportMessage::Pause
            | TransportMessage::Stop
            | TransportMessage::SkipBack
            | TransportMessage::SkipForward
            | TransportMessage::SeekToSample(_)
            | TransportMessage::SetBpmText(_) => UndoAction::Skip,
            TransportMessage::CommitBpm
            | TransportMessage::ToggleMetronome
            | TransportMessage::CycleTimeSignature
            // Direct control-endpoint setters (doc #265): undoable like
            // their GUI counterparts (cycle / loop toggle+drag).
            | TransportMessage::SetTimeSignature { .. }
            | TransportMessage::SetLoopRange { .. }
            | TransportMessage::ToggleLoop => UndoAction::Record,
        },

        // A bar shift touches every timeline collection; one entry.
        Message::Arrangement(_) => UndoAction::Record,
        Message::Marker(m) => match m {
            // Mutating edits: record an undo entry capturing the
            // pre-edit marker set (markers ride the ProjectFile
            // snapshot/replay path). `LoopToRegion` mutates the loop
            // range, matching `ToggleLoop`'s classification.
            MarkerMessage::AddAtPlayhead
            | MarkerMessage::Rename(_, _)
            | MarkerMessage::Recolor(_, _)
            | MarkerMessage::Delete(_)
            | MarkerMessage::LoopToRegion(_)
            | MarkerMessage::SeedFromSections => UndoAction::Record,
            // Drag gestures: a marker move or a region-edge resize fires
            // one message per pointer step, so coalesce each gesture into a
            // single undo entry keyed by marker id (mirrors fader / knob
            // bursts). A one-off convert-to-region / point still records a
            // lone entry — nothing to coalesce it with.
            MarkerMessage::MoveStart(id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::MarkerMove(*id))
            }
            MarkerMessage::SetRegionEnd(id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::MarkerResize(*id))
            }
            // Navigation only — moves the playhead / starts playback,
            // no project mutation, mirroring `SeekToSample` / `Play`.
            MarkerMessage::JumpToNext
            | MarkerMessage::JumpToPrev
            | MarkerMessage::JumpTo(_)
            | MarkerMessage::PlayFromMarker(_) => UndoAction::Skip,
        },

        // Committing an inline rename edits the persisted marker name, so it
        // records an undo entry exactly like `MarkerMessage::Rename` above.
        Message::MarkerUi(MarkerUiMessage::CommitRename) => UndoAction::Record,
        // Every other marker-interaction message (selection, menu open/close,
        // rename begin/change/cancel) is pure view state — never undoable.
        Message::MarkerUi(_) => UndoAction::Skip,

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

        // Aux-send edits. A level drag coalesces into one entry per
        // gesture (like the volume/pan faders); every other send action is
        // a discrete, atomic edit. The send graph rides the `ProjectFile`
        // snapshot since ba todo #1269, so these entries restore
        // end-to-end; here we only classify the bookkeeping (dirty-mark +
        // redo-clear).
        Message::Mixer(m) => match m {
            MixerMessage::SetSendLevel(send_id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::SendLevel(*send_id))
            }
            MixerMessage::AddSend { .. }
            | MixerMessage::AddSendWithId { .. }
            | MixerMessage::RemoveSend(_)
            | MixerMessage::SetSendDest(_, _)
            | MixerMessage::ToggleSendPreFader(_)
            | MixerMessage::ToggleSendEnabled(_)
            | MixerMessage::SetBusReturnRole(_, _)
            | MixerMessage::CreateReturnFromSend { .. } => UndoAction::Record,
        },
        // Freeze edits. Freeze / unfreeze / refreeze / batch-freeze are
        // discrete, atomic transitions worth an undo entry; the rendered
        // cache is deliberately excluded from history (see `UndoExtras` and
        // `apply_freeze_restore`). Cancelling an in-flight render is a
        // transient abort, not a project mutation — skip it.
        Message::Freeze(f) => match f {
            FreezeMessage::CancelFreeze => UndoAction::Skip,
            // Opening the cache directory in the file manager reads state
            // only — never a project mutation (ba todo #581).
            FreezeMessage::RevealFreezeCache => UndoAction::Skip,
            FreezeMessage::FreezeTrack(_)
            | FreezeMessage::UnfreezeTrack(_)
            | FreezeMessage::RefreezeTrack(_)
            | FreezeMessage::FreezeSelectedTracks
            | FreezeMessage::FreezeAllTracks => UndoAction::Record,
        },

        Message::Master(_) => UndoAction::Record,

        Message::Plugin(p) => match p {
            PluginMessage::AddPluginToTrack(_, _)
            | PluginMessage::AddPluginToTrackWithId { .. }
            | PluginMessage::RemovePluginFromTrack(_, _)
            | PluginMessage::MovePluginInTrack { .. } => UndoAction::Record,
            // Replacing a slot's plugin is a chain edit like any other,
            // and one the user must be able to take back: a swap
            // DISCARDS the outgoing plugin's preserved state (ba todo
            // #1308), and the snapshot taken before this message is the
            // only place that state still exists afterwards. Never
            // coalesced — two replaces in a row are two decisions.
            PluginMessage::ReplacePlugin { .. } => UndoAction::Record,
            // Routing a key is a project edit like any other insert
            // change, so it takes an undo entry of its own.
            PluginMessage::SetPluginSidechain { .. } => UndoAction::Record,
            // Bypassing a slot is a project edit and it persists, so it
            // takes an entry. Deliberately NOT coalesced: a knob drag
            // emits one message per frame and wants collapsing, but two
            // bypass toggles are two decisions a user expects to undo
            // separately (ba todo #1305).
            PluginMessage::SetPluginBypass { .. } => UndoAction::Record,
            // A preset recall is one gesture, so it takes one entry —
            // and it must NOT coalesce with anything: coalescing a recall
            // into a neighbouring knob edit would make the two undo
            // together (ba todo #1333).
            PluginMessage::LoadPluginPreset { .. } => UndoAction::Record,
            PluginMessage::SetPluginParam(instance_id, param_id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::PluginParam {
                    instance_id: *instance_id,
                    param_id: *param_id,
                })
            }
            PluginMessage::TogglePluginPanel(_)
            | PluginMessage::OpenPluginEditor(_)
            // A rescan changes what the machine offers, not what the
            // project contains — there is nothing to undo (todo #1307).
            | PluginMessage::RescanPlugins
            | PluginMessage::ClosePluginEditor(_) => UndoAction::Skip,
        },

        Message::Automation(a) => match a {
            // A breakpoint drag is one gesture → one undo entry.
            AutomationMessage::StartBreakpointDrag { .. } => UndoAction::Begin,
            AutomationMessage::EndBreakpointDrag => UndoAction::Commit,
            AutomationMessage::DragBreakpoint { .. } => UndoAction::Skip,
            // Chip-cycle of the shown lane is transient view state (todo
            // #1095) — never an undo entry.
            AutomationMessage::CycleTrackLane(_) => UndoAction::Skip,
            // Expanding a track's lane sub-rows is transient view state
            // (doc #256, todo #1096) — never an undo entry.
            AutomationMessage::ToggleTrackExpanded(_) => UndoAction::Skip,
            // Every discrete lane / breakpoint edit is atomic.
            AutomationMessage::AddLane(_)
            | AutomationMessage::RemoveLane(_)
            | AutomationMessage::ToggleRead(_)
            | AutomationMessage::AddBreakpoint { .. }
            | AutomationMessage::DeleteBreakpoint { .. }
            | AutomationMessage::SetCurveKind { .. } => UndoAction::Record,
        },

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

        Message::Clip(c) => match c {
            ClipMessage::StartClipDrag { .. } | ClipMessage::StartClipTrim { .. } => {
                UndoAction::Begin
            }
            ClipMessage::StartClipFadeDrag { .. } | ClipMessage::StartClipGainDrag { .. } => {
                UndoAction::Begin
            }
            ClipMessage::EndClipDrag | ClipMessage::EndClipTrim => UndoAction::Commit,
            ClipMessage::EndClipFadeDrag | ClipMessage::EndClipGainDrag => UndoAction::Commit,
            ClipMessage::UpdateClipDrag(_, _) | ClipMessage::UpdateClipTrim(_) => UndoAction::Skip,
            ClipMessage::UpdateClipFadeDrag(_) | ClipMessage::UpdateClipGainDrag(_) => {
                UndoAction::Skip
            }
            ClipMessage::DeleteClip(_) => UndoAction::Record,
            // Inspector flyout edits (todo #319): each is one discrete,
            // atomic edit — record a single undo entry per change, like the
            // numeric edits elsewhere. The drag gestures above coalesce via
            // Begin/Commit; these don't.
            ClipMessage::SetClipFadeInMs { .. }
            | ClipMessage::SetClipFadeOutMs { .. }
            | ClipMessage::SetClipGainDb { .. }
            | ClipMessage::SetClipFadeInCurve { .. }
            | ClipMessage::SetClipFadeOutCurve { .. }
            | ClipMessage::ResetClipFadeGain { .. } => UndoAction::Record,
            // Control-endpoint placement edits (`clip.move` / `clip.trim`):
            // atomic and already resolved, so one undo entry each — the
            // Begin/Commit coalescing above exists only for pointer drags.
            ClipMessage::MoveClipTo { .. }
            | ClipMessage::TrimClipTo { .. }
            | ClipMessage::SplitClipAt { .. } => UndoAction::Record,
        },

        Message::MidiClip(c) => match c {
            MidiClipMessage::StartMidiClipDrag { .. }
            | MidiClipMessage::StartMidiClipTrim { .. } => UndoAction::Begin,
            MidiClipMessage::EndMidiClipDrag | MidiClipMessage::EndMidiClipTrim => {
                UndoAction::Commit
            }
            MidiClipMessage::UpdateMidiClipDrag(_, _) | MidiClipMessage::UpdateMidiClipTrim(_) => {
                UndoAction::Skip
            }
            MidiClipMessage::DeleteMidiClip(_)
            | MidiClipMessage::CreateEmptyClip { .. }
            | MidiClipMessage::MoveClipTo { .. } => UndoAction::Record,
        },

        Message::MidiEditor(e) => match e {
            MidiEditorMessage::AddNote { .. }
            | MidiEditorMessage::RemoveNote { .. }
            | MidiEditorMessage::RemoveSelectedNotes { .. }
            | MidiEditorMessage::MoveNote { .. }
            | MidiEditorMessage::ResizeNote { .. }
            | MidiEditorMessage::SetNoteVelocity { .. }
            | MidiEditorMessage::ToggleSlur { .. }
            // Bulk control write (doc #269 FR-5): the pre-dispatch
            // snapshot of the prior notes makes the whole batch one
            // undo step — the entire point of the method.
            | MidiEditorMessage::SetClipNotes { .. }
            // Bulk timing edits (doc #163): each rewrites the clip's note
            // array, so the pre-dispatch snapshot of the prior notes is
            // the single undo step. Humanize draws its seed in the handler,
            // so re-doing rolls a new feel — undo still restores the exact
            // prior notes via the snapshot, which is what matters.
            | MidiEditorMessage::Quantize { .. }
            | MidiEditorMessage::Humanize { .. }
            | MidiEditorMessage::ApplyGroove { .. } => UndoAction::Record,
            // Groove *extraction* reads the clip and produces a template;
            // it never mutates the notes, so there's nothing to undo here.
            // Library persistence/undo is a separate slice (#395).
            MidiEditorMessage::ExtractGroove { .. } => UndoAction::Skip,
            MidiEditorMessage::OpenMidiEditor(_)
            | MidiEditorMessage::OpenSelectedMidiClip
            | MidiEditorMessage::CloseMidiEditor
            | MidiEditorMessage::SelectNote { .. }
            | MidiEditorMessage::ToggleNoteSelection { .. }
            | MidiEditorMessage::SelectNotesInRect { .. }
            | MidiEditorMessage::SelectAllNotes
            | MidiEditorMessage::ClearNoteSelection
            | MidiEditorMessage::PreviewNote(_, _)
            | MidiEditorMessage::StopPreview(_, _)
            | MidiEditorMessage::ScrollY(_)
            // Quantize-panel control edits (todo #392) just mutate view
            // state — the actual note edit is the `Quantize` message above.
            | MidiEditorMessage::SetQuantizeGrid(_)
            | MidiEditorMessage::SetQuantizeStrength(_)
            | MidiEditorMessage::SetQuantizeSwing(_)
            | MidiEditorMessage::SetQuantizeMode(_)
            | MidiEditorMessage::SetQuantizeEnds(_)
            | MidiEditorMessage::SetQuantizeIterative(_)
            // Humanize-panel control edits (todo #393) likewise just mutate
            // view state — the note edit is the `Humanize` message above.
            | MidiEditorMessage::SetHumanizeTiming(_)
            | MidiEditorMessage::SetHumanizeVelocity(_)
            // Groove-panel control edits (todo #394) just mutate view state —
            // the note edit is the `ApplyGroove` message; extract is read-only.
            | MidiEditorMessage::SetGrooveName(_)
            | MidiEditorMessage::SetGrooveSelection(_)
            | MidiEditorMessage::SetGrooveStrength(_) => UndoAction::Skip,
        },

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

        // Reference-track (A/B). Only the content-changing actions named
        // in the design (load / remove / set-active / loudness-match /
        // trim) are reversible; the trim drag coalesces. The monitoring
        // toggles, markers, scrub, and error dismissal are transient.
        Message::Reference(rm) => match rm {
            ReferenceMessage::LoadRequested(_)
            // A picked file ends in the same load path as a drag-drop, so
            // a successful pick is just as reversible; a cancelled pick
            // (`None`) changes nothing.
            | ReferenceMessage::FilePicked(Some(_))
            | ReferenceMessage::Remove(_)
            | ReferenceMessage::SetActive(_)
            | ReferenceMessage::ToggleLoudnessMatch => UndoAction::Record,
            ReferenceMessage::TrimChanged(_) => {
                UndoAction::RecordCoalesced(CoalesceKey::ReferenceTrim)
            }
            // Opening the picker and a cancelled pick are pure UI / no-ops;
            // the monitoring toggles, markers, scrub, and error dismissal
            // are transient.
            ReferenceMessage::PickFile
            | ReferenceMessage::FilePicked(None)
            | ReferenceMessage::ToggleAbSource
            | ReferenceMessage::SetAbSource(_)
            | ReferenceMessage::MomentaryAudition(_)
            | ReferenceMessage::AddMarker { .. }
            | ReferenceMessage::RemoveMarker { .. }
            | ReferenceMessage::Scrub { .. }
            | ReferenceMessage::ToggleLoopToMix
            | ReferenceMessage::DismissError => UndoAction::Skip,
        },
    }
}
