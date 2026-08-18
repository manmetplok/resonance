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
        | Message::Arrangement(_)
        | Message::Marker(_)
        | Message::MarkerUi(_)
        | Message::Track(_)
        | Message::ExternalInstrument(_)
        | Message::Bus(_)
        | Message::Mixer(_)
        | Message::Freeze(_)
        | Message::Master(_)
        | Message::Clip(_)
        | Message::MidiClip(_)
        | Message::MidiEditor(_)
        | Message::VocalTuning(_)
        | Message::Plugin(_)
        | Message::Automation(_)
        | Message::Viewport(_)
        | Message::Reference(_)
        | Message::GlobalTrack(_)
        | Message::Group(_)
        | Message::ChordTrack(_)
        // Media-browser navigation / audition is only meaningful with a
        // project open; block it while the startup modal owns the screen.
        | Message::Browser(_)
        // A drag-to-timeline placement can only start once the arrangement
        // is on screen (i.e. a project is open); block it while the startup
        // modal owns the screen, like the other project-mutating gestures.
        | Message::Drag(_) => true,
        // Tab switches / auxiliary overlays: block so they can't
        // steal focus from the startup modal.
        Message::Ui(UiMessage::SwitchView(_))
        | Message::Ui(UiMessage::TogglePerformanceMode)
        | Message::Ui(UiMessage::RequestPerformanceToggle)
        | Message::Ui(UiMessage::PerformanceToggleResolved { .. })
        | Message::Ui(UiMessage::ExitPerformanceMode)
        | Message::Ui(UiMessage::OpenSettings)
        | Message::Ui(UiMessage::OpenAddTrackMenu)
        // Markers overview + marker navigation are only meaningful with a
        // project open — block them while the startup modal owns the screen
        // (the nav variants would otherwise drive gated `Marker` messages).
        | Message::Ui(UiMessage::ToggleMarkersOverview)
        | Message::Ui(UiMessage::RequestMarkerNav { .. })
        | Message::Ui(UiMessage::MarkerNavResolved { .. })
        // The track context menu acts on a project's tracks — block it
        // while the startup modal owns the screen (there are no tracks to
        // act on yet), like the other auxiliary overlays.
        | Message::Ui(UiMessage::OpenTrackMenu { .. }) => true,
        // Benign UI: allow.
        Message::Ui(UiMessage::CloseSettings)
        | Message::Ui(UiMessage::CloseAddTrackMenu)
        | Message::Ui(UiMessage::CloseTrackMenu)
        | Message::Ui(UiMessage::DismissError)
        | Message::Ui(UiMessage::StartNewProject)
        | Message::Ui(UiMessage::SelectTrack(_))
        | Message::Ui(UiMessage::SelectBus(_))
        | Message::Ui(UiMessage::ModifiersChanged(_))
        | Message::Ui(UiMessage::ConfirmSaveAndQuit)
        | Message::Ui(UiMessage::ConfirmDiscardAndQuit)
        | Message::Ui(UiMessage::CancelQuit)
        | Message::Ui(UiMessage::ToggleGlobalTracks)
        | Message::Ui(UiMessage::ToggleReferencePanel)
        | Message::Ui(UiMessage::CloseMarkersOverview)
        | Message::Ui(UiMessage::ToggleMixerInspectorGroup(_))
        | Message::Ui(UiMessage::ToggleMidiClockSend)
        | Message::Ui(UiMessage::SetMidiClockSendDevice(_))
        | Message::Ui(UiMessage::ToggleMidiClockRecv)
        | Message::Ui(UiMessage::SetMidiClockRecvDevice(_))
        // Performance footer selections are pure view state and only
        // reachable from within Performance mode (which needs a project),
        // so they're harmless even if one slips through while the startup
        // modal is up — allow.
        | Message::Ui(UiMessage::SetPerformanceTuning(_))
        | Message::Ui(UiMessage::SetPerformanceCapo(_))
        // Dismissing the import-progress modal is safe from any state
        // (nothing has been imported yet when the startup modal is up, so
        // the tracker is empty); allow it through so the overlay can be
        // cleared if it somehow appears.
        | Message::Ui(UiMessage::DismissImportProgress) => false,
        // Project I/O drives the modal itself: always allow.
        Message::ProjectIo(_) => false,
        // Export modal drives its own overlay; gated at the open site.
        Message::Export(_) => false,
        // The MIDI Import modal is an auxiliary overlay that imports into
        // a project — block it (like Open Settings / Add Track) so it
        // can't steal focus while the startup modal owns the screen.
        Message::Import(_) => true,
        // Audio import + placement mutates the project (adds pool assets /
        // clips / tracks) — block while the startup modal owns the screen.
        Message::Pool(_) => true,
        // Missing-file relink acts on a loaded project's pool — block it
        // while the startup modal owns the screen (there is no project to
        // relink into yet).
        Message::Relink(_) => true,
        // Timer tick: harmless, drives VU meters — allow.
        Message::Tick => false,
        // Control-endpoint envelope: always allow so every request gets
        // a reply (a swallowed envelope would wedge the remote client).
        // Mutating control methods synthesize domain messages that
        // re-enter `update()` and hit this gate individually.
        Message::Control(_) => false,
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
        // Control-endpoint envelope: keep flowing so requests are always
        // answered (read-only introspection stays valid mid-render);
        // synthesized mutating messages re-enter `update()` and are
        // blocked by this gate individually.
        Message::Control(_) => false,
        // Everything else: block.
        Message::Compose(_)
        | Message::Transport(_)
        | Message::Arrangement(_)
        | Message::Marker(_)
        | Message::MarkerUi(_)
        | Message::Track(_)
        | Message::ExternalInstrument(_)
        | Message::Bus(_)
        | Message::Mixer(_)
        | Message::Freeze(_)
        | Message::Master(_)
        | Message::Clip(_)
        | Message::MidiClip(_)
        | Message::MidiEditor(_)
        | Message::VocalTuning(_)
        | Message::Plugin(_)
        | Message::Automation(_)
        | Message::Group(_)
        | Message::Viewport(_)
        | Message::Reference(_)
        | Message::GlobalTrack(_)
        | Message::Import(_)
        | Message::Pool(_)
        // Relinking re-copies audio into the project through the shared
        // decode path — block it mid-render like the rest.
        | Message::Relink(_)
        | Message::ChordTrack(_)
        | Message::Ui(_)
        | Message::Export(_)
        // Auditioning a preview through the engine mid-render would
        // disturb the shared decode path — block it like the rest.
        | Message::Browser(_)
        // A drop mutates the project (imports + places audio); block the
        // whole drag gesture mid-render like the rest.
        | Message::Drag(_)
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
        // Control-endpoint envelope: keep flowing so requests are always
        // answered; synthesized mutating messages re-enter `update()`
        // and are blocked by this gate individually.
        Message::Control(_) => false,
        // Everything else: block.
        Message::Compose(_)
        | Message::Transport(_)
        | Message::Arrangement(_)
        | Message::Marker(_)
        | Message::MarkerUi(_)
        | Message::Track(_)
        | Message::ExternalInstrument(_)
        | Message::Bus(_)
        | Message::Mixer(_)
        | Message::Freeze(_)
        | Message::Master(_)
        | Message::Clip(_)
        | Message::MidiClip(_)
        | Message::MidiEditor(_)
        | Message::VocalTuning(_)
        | Message::Plugin(_)
        | Message::Automation(_)
        | Message::Group(_)
        | Message::Viewport(_)
        | Message::Reference(_)
        | Message::GlobalTrack(_)
        | Message::Import(_)
        | Message::Pool(_)
        // Relinking re-copies audio into the project through the shared
        // decode path — block it mid-render like the rest.
        | Message::Relink(_)
        | Message::ChordTrack(_)
        | Message::Ui(_)
        | Message::Export(_)
        // Auditioning a preview through the engine mid-render would
        // disturb the shared decode path — block it like the rest.
        | Message::Browser(_)
        // A drop mutates the project (imports + places audio); block the
        // whole drag gesture mid-render like the rest.
        | Message::Drag(_)
        | Message::Undo
        | Message::Redo => true,
    }
}

/// The MIDI clip a [`MidiEditorMessage`] *edits*, or `None` for the
/// navigation / selection / preview variants that touch no note data.
/// Used by [`Resonance::frozen_input_edit_target`] to gate note + lyric
/// edits on frozen tracks.
fn midi_editor_edit_clip(
    m: &crate::message::MidiEditorMessage,
) -> Option<resonance_audio::types::ClipId> {
    use crate::message::MidiEditorMessage::*;
    match m {
        AddNote { clip_id, .. }
        | RemoveNote { clip_id, .. }
        | RemoveSelectedNotes { clip_id, .. }
        | MoveNote { clip_id, .. }
        | ResizeNote { clip_id, .. }
        | SetNoteVelocity { clip_id, .. }
        | SetClipNotes { clip_id, .. }
        | ToggleSlur { clip_id, .. } => Some(*clip_id),
        // Open / close / select / preview / scroll don't change note data.
        // The selection variants (toggle / marquee / select-all / clear)
        // only move the highlight, so they're never gated on a frozen track.
        // The Quantize-panel setting variants only mutate panel state, and
        // ExtractGroove only *reads* notes to capture a template; the bulk
        // note mutators (Quantize / Humanize / ApplyGroove) target the open
        // editor clip and are resolved by `midi_editor_edits_open_clip`.
        OpenMidiEditor(_)
        | OpenSelectedMidiClip
        | CloseMidiEditor
        | SelectNote { .. }
        | ToggleNoteSelection { .. }
        | SelectNotesInRect { .. }
        | SelectAllNotes
        | ClearNoteSelection
        | PreviewNote(..)
        | StopPreview(..)
        | ScrollY(_)
        | Quantize { .. }
        | Humanize { .. }
        | ApplyGroove { .. }
        | ExtractGroove { .. }
        | SetQuantizeGrid(_)
        | SetQuantizeStrength(_)
        | SetQuantizeSwing(_)
        | SetQuantizeMode(_)
        | SetQuantizeEnds(_)
        | SetQuantizeIterative(_)
        | SetHumanizeTiming(_)
        | SetHumanizeVelocity(_)
        | SetGrooveName(_)
        | SetGrooveSelection(_)
        | SetGrooveStrength(_) => None,
    }
}

/// The bulk note mutators of the Quantize / Humanize / Groove panel carry
/// no clip id — they operate on the *open* editor clip — so they're gated
/// through the live editor state rather than the message payload.
fn midi_editor_edits_open_clip(m: &crate::message::MidiEditorMessage) -> bool {
    use crate::message::MidiEditorMessage::*;
    matches!(m, Quantize { .. } | Humanize { .. } | ApplyGroove { .. })
}

/// The track a [`PluginMessage`] edits the *render inputs* of — a param
/// change, or adding / removing a plugin (which includes swapping the
/// instrument). The panel-toggle / editor-window variants don't change the
/// rendered signal, so they're never gated.
fn plugin_edit_target(
    r: &crate::Resonance,
    m: &crate::message::PluginMessage,
) -> Option<resonance_audio::types::TrackId> {
    use crate::message::PluginMessage::*;
    match m {
        SetPluginParam(instance_id, ..) => r.track_of_plugin(*instance_id),
        // A preset recall moves parameters, so it changes the rendered
        // signal exactly as the individual writes it replaces would.
        LoadPluginPreset { instance_id, .. } => r.track_of_plugin(*instance_id),
        // Re-keying a detector changes the rendered signal on the
        // plugin's own track, so a frozen track must go stale for it.
        SetPluginSidechain { instance_id, .. } => r.track_of_plugin(*instance_id),
        AddPluginToTrack(track_id, _) | RemovePluginFromTrack(track_id, _) => Some(*track_id),
        AddPluginToTrackWithId { track_id, .. } | MovePluginInTrack { track_id, .. } => {
            Some(*track_id)
        }
        TogglePluginPanel(_) | OpenPluginEditor(_) | ClosePluginEditor(_) => None,
    }
}

impl crate::Resonance {
    /// The track owning a MIDI clip, by clip id.
    fn track_of_midi_clip(
        &self,
        clip_id: resonance_audio::types::ClipId,
    ) -> Option<resonance_audio::types::TrackId> {
        self.midi_clips
            .iter()
            .find(|c| c.id == clip_id)
            .map(|c| c.track_id)
    }

    /// The track owning a plugin instance, by instance id. Prefers the
    /// `plugin_index` side-table, falling back to a scan so a desynced
    /// index degrades to O(n) instead of a miss (mirrors `with_plugin_mut`).
    fn track_of_plugin(
        &self,
        instance_id: resonance_audio::types::PluginInstanceId,
    ) -> Option<resonance_audio::types::TrackId> {
        if let Some(crate::state::PluginLocator::Track(track_id)) =
            self.plugin_index.get(&instance_id).copied()
        {
            return Some(track_id);
        }
        self.registry
            .tracks
            .iter()
            .find(|t| t.plugins.iter().any(|p| p.instance_id == instance_id))
            .map(|t| t.id)
    }

    /// When `message` is an edit to a track's *frozen inputs* — notes,
    /// lyrics, plugin params, instrument selection, or the FX-bypass flag
    /// (all of which the freeze render captured) — return that track id.
    /// `None` for everything else, including the mixer controls (volume /
    /// pan / mute / solo / routing / sends) that stay live while frozen.
    ///
    /// The caller ([`update_inner`](crate::Resonance::update)) uses this to
    /// gate edits on a frozen track: the edit is dropped (no mutation, no
    /// undo) and the freeze flips to `Stale` instead (ba todo #576).
    pub(crate) fn frozen_input_edit_target(
        &self,
        message: &crate::message::Message,
    ) -> Option<resonance_audio::types::TrackId> {
        use crate::message::*;
        match message {
            // Note + lyric edits, keyed by the clip's owning track. The
            // bulk quantize/humanize/groove mutators carry no clip id and
            // apply to the open editor clip instead.
            Message::MidiEditor(m) => midi_editor_edit_clip(m)
                .or_else(|| {
                    midi_editor_edits_open_clip(m)
                        .then(|| self.interaction.editing_midi_clip.as_ref().map(|e| e.clip_id))
                        .flatten()
                })
                .and_then(|clip_id| self.track_of_midi_clip(clip_id)),
            // Deleting a MIDI clip removes its notes from the render.
            Message::MidiClip(MidiClipMessage::DeleteMidiClip(clip_id)) => {
                self.track_of_midi_clip(*clip_id)
            }
            // Plugin params, plugin add/remove, instrument swap.
            Message::Plugin(m) => plugin_edit_target(self, m),
            // Bypassing the FX chain changes the post-FX signal freeze
            // rendered — treat it as an input edit, not a mixer control.
            Message::Track(TrackMessage::ToggleTrackFxBypass(track_id)) => Some(*track_id),
            _ => None,
        }
    }

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
        if self.plugin_move_is_refused(message) {
            return true;
        }
        false
    }

    /// A chain reorder the domain rule refuses (ba todo #1261).
    ///
    /// Gated rather than dropped inside the handler because
    /// `update_inner` records undo and bumps the revision *before*
    /// dispatch: refusing later would spend an undo entry and signal a
    /// revision change for a move that never happened, so a control
    /// client polling `revision` would see a phantom concurrent edit and
    /// the user's next undo would restore an identical snapshot.
    ///
    /// A track or plugin that vanished between message and handler is
    /// refused here too — there is no chain left to reorder.
    fn plugin_move_is_refused(&self, message: &crate::message::Message) -> bool {
        use crate::message::{Message, PluginMessage};
        let Message::Plugin(PluginMessage::MovePluginInTrack {
            track_id,
            instance_id,
            to_index,
        }) = message
        else {
            return false;
        };
        let Some(track) = self.registry.tracks.iter().find(|t| t.id == *track_id) else {
            return true;
        };
        let Some(moving) = track
            .plugins
            .iter()
            .position(|p| p.instance_id == *instance_id)
        else {
            return true;
        };
        let requested = u32::try_from(*to_index).unwrap_or(u32::MAX);
        crate::plugin_chain::resolve_effect_move(self, track, moving as u32, requested).is_err()
    }
}
