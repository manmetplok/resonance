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
        // Comping edits a recorded lane, which only exists inside a
        // project — block while the startup modal owns the screen.
        | Message::Take(_)
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
        | Message::Ui(UiMessage::ExitPerformanceMode)
        | Message::Ui(UiMessage::OpenSettings)
        | Message::Ui(UiMessage::OpenAddTrackMenu)
        // The markers overview is only meaningful with a project open —
        // block it while the startup modal owns the screen.
        | Message::Ui(UiMessage::ToggleMarkersOverview)
        // The track context menu acts on a project's tracks — block it
        // while the startup modal owns the screen (there are no tracks to
        // act on yet), like the other auxiliary overlays.
        | Message::Ui(UiMessage::OpenTrackMenu { .. }) => true,
        // Focus-gated shortcut envelope: allow — the wrapped message
        // re-enters `update()` on resolution and meets this gate then.
        Message::Ui(UiMessage::RequestShortcut(_))
        | Message::Ui(UiMessage::ShortcutResolved { .. })
        | Message::Ui(UiMessage::ShortcutKey { .. })
        | Message::Ui(UiMessage::ShortcutProbed { .. })
        // The palette refuses to open over the startup screen itself
        // (`Overlay::allows_palette`); a row's command meets this gate
        // when it re-enters `update()`.
        | Message::Ui(UiMessage::OpenPalette(_))
        | Message::Ui(UiMessage::ClosePalette)
        | Message::Ui(UiMessage::Palette(_))
        | Message::Ui(UiMessage::Keymap(_)) => false,
        // Benign UI: allow.
        Message::Ui(UiMessage::CloseSettings)
        | Message::Ui(UiMessage::CloseAddTrackMenu)
        | Message::Ui(UiMessage::CloseTrackMenu)
        | Message::Ui(UiMessage::DismissError)
        | Message::Ui(UiMessage::StartNewProject)
        | Message::Ui(UiMessage::NewEmptyProject)
        | Message::Ui(UiMessage::ArmPresetDelete(_))
        | Message::Ui(UiMessage::BpmFieldHovered(_))
        | Message::Ui(UiMessage::BpmFieldPointer)
        | Message::Ui(UiMessage::SelectTrack(_))
        | Message::Ui(UiMessage::SelectBus(_))
        | Message::Ui(UiMessage::SelectMaster)
        | Message::Ui(UiMessage::WindowResized(_))
        // The inline rename's buffer is UI state; its commit re-enters
        // `update()` as `SetTrackName` / `RenameBus` and meets the gate
        // then.
        | Message::Ui(UiMessage::BeginRename(..))
        | Message::Ui(UiMessage::RenameInput(_))
        | Message::Ui(UiMessage::CommitRename)
        | Message::Ui(UiMessage::CancelRename)
        | Message::Ui(UiMessage::RenamePointer)
        | Message::Ui(UiMessage::RenameHovered(_))
        | Message::Ui(UiMessage::ModifiersChanged(_))
        | Message::Ui(UiMessage::ConfirmSaveAndQuit)
        | Message::Ui(UiMessage::ConfirmDiscardAndQuit)
        | Message::Ui(UiMessage::CancelQuit)
        | Message::Ui(UiMessage::ToggleGlobalTracks)
        | Message::Ui(UiMessage::ToggleReferencePanel)
        | Message::Ui(UiMessage::CloseMarkersOverview)
        | Message::Ui(UiMessage::ToggleMixerInspectorGroup(_))
        | Message::Ui(UiMessage::ToggleTakeLane(_))
        | Message::Ui(UiMessage::ToggleFollowPlayhead)
        | Message::Ui(UiMessage::ToggleAutosave)
        | Message::Ui(UiMessage::SetAutosaveInterval(_))
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
        | Message::Ui(UiMessage::DismissImportProgress)
        // Same reasoning for the missing-plugin warning: it reports on
        // an OPEN project's chains, so it cannot be showing while the
        // startup modal is up — and if it somehow is, being able to
        // close it is strictly better than not.
        | Message::Ui(UiMessage::DismissMissingPlugins)
        | Message::Ui(UiMessage::ShowMissingPlugins) => false,
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
        // ... and on the Export modal's stem render.
        Message::Export(ExportMessage::CancelRender) => false,
        // Engine event traffic, project I/O, and the timer tick all
        // need to keep flowing — the bounce relies on `BounceProgress`
        // / `TrackBounceCompleted` events to clear the modal.
        Message::ProjectIo(_) | Message::Tick | Message::WindowCloseRequested(_) => false,
        // Control-endpoint envelope: keep flowing so requests are always
        // answered (read-only introspection stays valid mid-render);
        // synthesized mutating messages re-enter `update()` and are
        // blocked by this gate individually.
        Message::Control(_) => false,
        // Keyboard envelopes: the message a shortcut resolves to re-enters
        // `update()` and meets this gate on its own, so ⌘S still saves
        // mid-render exactly as it did before shortcuts went through the
        // registry.
        Message::Ui(UiMessage::ShortcutKey { .. })
        | Message::Ui(UiMessage::ShortcutProbed { .. })
        | Message::Ui(UiMessage::RequestShortcut(_))
        | Message::Ui(UiMessage::ShortcutResolved { .. }) => false,
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
        | Message::Take(_)
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
        // Keyboard envelopes: the message a shortcut resolves to re-enters
        // `update()` and meets this gate on its own, so ⌘S still saves
        // mid-render exactly as it did before shortcuts went through the
        // registry.
        Message::Ui(UiMessage::ShortcutKey { .. })
        | Message::Ui(UiMessage::ShortcutProbed { .. })
        | Message::Ui(UiMessage::RequestShortcut(_))
        | Message::Ui(UiMessage::ShortcutResolved { .. }) => false,
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
        | Message::Take(_)
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
        // Already done, in the plugin: there is no edit left to refuse,
        // only a mirror to keep honest (the freeze fingerprint, which
        // hashes the mirror, notices the change).
        ParamEditedByPlugin { .. } => None,
        // A preset recall moves parameters, so it changes the rendered
        // signal exactly as the individual writes it replaces would.
        LoadPluginPreset { instance_id, .. } => r.track_of_plugin(*instance_id),
        LoadPluginPresetFromLocation { instance_id, .. } => r.track_of_plugin(*instance_id),
        PresetStep { instance_id, .. } => r.track_of_plugin(*instance_id),
        // Re-keying a detector changes the rendered signal on the
        // plugin's own track, so a frozen track must go stale for it.
        SetPluginSidechain { instance_id, .. } => r.track_of_plugin(*instance_id),
        // Taking a plugin out of the chain changes that track's rendered
        // signal as surely as any parameter does (ba todo #1305).
        SetPluginBypass { instance_id, .. } => r.track_of_plugin(*instance_id),
        AddPluginToTrack(track_id, _) | RemovePluginFromTrack(track_id, _) => Some(*track_id),
        // Swapping the plugin in a slot changes what the chain renders
        // exactly as adding or removing one does, so a frozen track goes
        // stale for it. Resolved through the instance id because the
        // message names no chain — it works on busses and master too,
        // and neither of those freezes, so `track_of_plugin` correctly
        // answers `None` for them.
        ReplacePlugin { instance_id, .. } => r.track_of_plugin(*instance_id),
        AddPluginToTrackWithId { track_id, .. } | MovePluginInTrack { track_id, .. } => {
            Some(*track_id)
        }
        OpenPluginEditor(_)
        | ClosePluginEditor(_)
        | OpenPluginWindow(_)
        | ClosePluginWindow(_)
        | PluginWindowDrag(_)
        | OpenGenericParams(_)
        | FocusSlot(_) => None,
        // View state; an edit it leads to is re-dispatched as its own
        // message and gated as that.
        ChainUi(_) => None,
        // An audition moves the sound as a recall would; the loads that
        // stick are gated as `LoadPluginPreset` / the add they become.
        PresetUi(crate::message::PresetUiMessage::BrowserAudition(_)) => r
            .presets
            .host_browser
            .as_ref()
            .and_then(|b| r.track_of_plugin(b.instance_id)),
        PresetUi(_) => None,
        // Refreshing the catalog edits no track's signal, so it is never
        // gated — a frozen track stays frozen through a rescan.
        RescanPlugins => None,
    }
}

impl crate::Resonance {
    /// True while a control client's OFFLINE measurement job
    /// (`meter.measure` / `meter.stems` with source `"render"`) is still
    /// rendering.
    ///
    /// An offline measurement holds the offline renderer exclusively
    /// (`OfflineRenderGuard::try_acquire_exclusive`) and refuses to start
    /// while a bounce / freeze / export runs — but the file-writing
    /// renderers `mark()` unconditionally, so the exclusion has to be
    /// enforced in the app in this direction too: every bounce / freeze /
    /// export START path checks this and refuses, otherwise two offline
    /// renderers would drive `process()` / `reset()` on the same live
    /// CLAP plugin instances concurrently — exactly the corruption the
    /// guard exists to prevent.
    pub(crate) fn offline_measure_in_progress(&self) -> bool {
        self.control.jobs.has_live_offline_measure()
    }

    /// A stem export from the Export modal is rendering (code review
    /// ARCH2-01). The modal can't close mid-render, so its phase is the
    /// whole truth.
    pub(crate) fn stem_export_in_progress(&self) -> bool {
        self.modals.export_dialog.as_ref().is_some_and(|d| d.is_rendering())
    }

    /// True while ANY offline render owns the engine's plugin instances:
    /// a WAV / FLAC mixdown (`io.bouncing`, GUI or `render.mixdown`), a
    /// stem export, a bounce in place, a freeze (single or batch) or an
    /// offline control measurement.
    pub(crate) fn offline_render_in_progress(&self) -> bool {
        self.io.bouncing
            || self.stem_export_in_progress()
            || self.modals.bounce_in_progress.is_some()
            || self.freeze.any_in_flight()
            || self.offline_measure_in_progress()
    }

    /// Refuse to swap the project out under an offline render (code
    /// review UPD-06): `ClearAll` + replay would drain the plugin map and
    /// the track / clip lists the worker is rendering from. The
    /// `ProjectIo` family is deliberately exempt from the pre-dispatch
    /// gates (saves and render-completion traffic must flow), so the
    /// open / new-project entry points call this themselves. Sets the
    /// error banner and reports `true` when the request must be dropped.
    pub(crate) fn refuse_project_switch_during_render(&mut self) -> bool {
        if !self.offline_render_in_progress() {
            return false;
        }
        self.banners.error_message = Some(
            "An offline render is in progress; open or create a project when it finishes".into(),
        );
        true
    }

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
    /// `plugin_mirror.index` side-table, falling back to a scan so a
    /// desynced index degrades to O(n) instead of a miss (mirrors
    /// `with_plugin_mut`).
    fn track_of_plugin(
        &self,
        instance_id: resonance_audio::types::PluginInstanceId,
    ) -> Option<resonance_audio::types::TrackId> {
        if let Some(crate::state::ChainOwner::Track(track_id)) =
            self.plugin_mirror.index.get(&instance_id).copied()
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
    /// lyrics, plugin params (and their automation lanes), instrument
    /// selection, or the FX-bypass flag
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
                        .then(|| self.ui.interaction.editing_midi_clip.as_ref().map(|e| e.clip_id))
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
            // A plugin-param lane drives the plugin the freeze rendered
            // (and `freeze_content_fingerprint` hashes it), so editing one
            // on a frozen track is an input edit. Gain / pan / mute lanes
            // drive the mixer, which stays live while frozen
            // (automation-control-api.md D2).
            Message::Automation(m) => match m.edited_target() {
                Some(resonance_common::AutomationTarget::PluginParam { instance, .. }) => {
                    self.track_of_plugin(*instance)
                }
                _ => None,
            },
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
        // A WAV / FLAC mixdown (`io.bouncing`) drives the same live plugin
        // instances as a bounce in place, so it gates the same traffic
        // (code review UPD-06; the engine refuses Play / Record on its
        // own — `resonance_audio` MIX-02 — this keeps the GUI honest).
        // A stem export renders through the same offline renderer.
        if (self.modals.bounce_in_progress.is_some()
            || self.io.bouncing
            || self.stem_export_in_progress())
            && bounce_blocks_message(message)
        {
            return true;
        }
        if self.freeze.any_in_flight() && freeze_blocks_message(message) {
            return true;
        }
        if self.plugin_move_is_refused(message) {
            return true;
        }
        if self.take_edit_is_refused(message) {
            return true;
        }
        if self.import_confirm_is_refused(message) {
            return true;
        }
        if self.bus_rename_is_noop(message) {
            return true;
        }
        if self.plugin_param_is_read_only(message) {
            return true;
        }
        false
    }

    /// A `SetPluginParam` on a read-only output (a load progress): the
    /// plugin drops the write, so the handler sends nothing. Gated here
    /// rather than refused there because the message records its undo
    /// entry, marks the project dirty and bumps the revision before
    /// dispatch (code review STATE2-10).
    fn plugin_param_is_read_only(&self, message: &crate::message::Message) -> bool {
        let crate::message::Message::Plugin(crate::message::PluginMessage::SetPluginParam(
            instance_id,
            param_id,
            _,
        )) = message
        else {
            return false;
        };
        self.plugin_slot(*instance_id).is_some_and(|slot| {
            slot.params
                .iter()
                .any(|p| p.id == *param_id && p.read_only)
        })
    }

    /// A `RenameBus` that would change nothing: the trimmed name is empty
    /// or the bus already has it (or the bus is gone). Gated rather than
    /// ignored in the handler because the message records its undo entry
    /// (and marks the project dirty) before dispatch.
    fn bus_rename_is_noop(&self, message: &crate::message::Message) -> bool {
        let crate::message::Message::Bus(crate::message::BusMessage::RenameBus(id, name)) =
            message
        else {
            return false;
        };
        let name = name.trim();
        name.is_empty()
            || self
                .registry
                .busses
                .iter()
                .find(|b| b.id == *id)
                .is_none_or(|b| b.name == name)
    }

    /// A MIDI Import Confirm that cannot import (code review FU-V2a): no
    /// tracks selected, a tempo conflict not yet resolved, a merge target
    /// that is gone or frozen. Gated for the reason spelled out on
    /// [`take_edit_is_refused`](Self::take_edit_is_refused) — Confirm
    /// records its undo entry before dispatch — and decided by
    /// `update::import::confirm_blocker`, the same predicate that
    /// disables the dialog's Import button.
    fn import_confirm_is_refused(&self, message: &crate::message::Message) -> bool {
        matches!(
            message,
            crate::message::Message::Import(crate::message::ImportMessage::Confirm)
        ) && crate::update::import::confirm_blocker(self).is_some()
    }

    /// A take-lane comp edit that would change nothing (epic #15, todo
    /// #411).
    ///
    /// Gated for the reason [`plugin_move_is_refused`](Self::plugin_move_is_refused)
    /// spells out: `update_inner` records the undo snapshot and bumps the
    /// control-API revision *before* dispatch, so an edit refused inside
    /// the handler would still leave an undo entry that restores an
    /// identical snapshot and a revision bump a remote client would read
    /// as a concurrent edit.
    ///
    /// The predicate is `update::takes::plan` itself — the same function
    /// the handler applies — so the gate and the edit can never disagree
    /// about what "changes nothing" means. It covers an unknown group or
    /// take, a solo that is already current, a split off the slot or on an
    /// existing boundary, and a promote clamped away to nothing.
    ///
    /// Deleting a group's **last** take used to be refused here too. Since
    /// ba todo #1401 it removes the lane instead — the engine grew the
    /// commands for it (todo #1397) — so the only deletion this gate still
    /// drops is one naming a group or take that is not there.
    fn take_edit_is_refused(&self, message: &crate::message::Message) -> bool {
        let crate::message::Message::Take(m) = message else {
            return false;
        };
        crate::update::takes::plan(self, m).is_none()
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
