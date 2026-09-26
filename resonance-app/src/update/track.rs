use iced::Task;
use resonance_audio::types::{AudioCommand, TrackId, TrackOutput};

use crate::message::Message;
use crate::presets::TrackPreset;
use crate::state::TrackState;
use crate::util::db_to_gain;
use crate::Resonance;

#[derive(Debug, Clone)]
pub enum TrackMessage {
    AddTrack,
    AddInstrumentTrack,
    /// Create an instrument track that starts already in external-instrument
    /// mode (doc #251 gap 1 affordance 2). Allocates the track id app-side so
    /// the external state lands atomically with the track in one undo step;
    /// the menu entry that dispatches it is a separate view todo.
    AddExternalInstrumentTrack,
    AddVocalTrack,
    /// Add a track with a caller-allocated id (control endpoint, doc
    /// #265, todo #1152). Unlike the GUI adds, the id is allocated
    /// *before* this message is built (not inside the handler), so the
    /// control reply can return the real `track_id` immediately; `drums`
    /// comes up as an instrument track whose instrument type is set to
    /// Drum when the engine echo mirrors it. Undoable as one step, like
    /// the other adds.
    AddControlTrack {
        id: TrackId,
        kind: crate::state::ControlTrackKind,
        name: Option<String>,
    },
    /// User clicked delete on a track — may require confirmation if it
    /// has content.
    RequestRemoveTrack(TrackId),
    /// User confirmed removal in the "track has content" dialog.
    ConfirmRemoveTrack,
    /// User cancelled the "track has content" dialog.
    CancelRemoveTrack,
    SetTrackVolume(TrackId, f32),
    SetTrackPan(TrackId, f32),
    SetMasterVolume(f32),
    ToggleMute(TrackId),
    ToggleSolo(TrackId),
    ToggleRecordArm(TrackId),
    ToggleMonitor(TrackId),
    ToggleTrackMono(TrackId),
    ToggleTrackFxBypass(TrackId),
    /// Rename a track (edited from the Compose instrument details panel).
    SetTrackName(TrackId, String),
    SetTrackInputDevice(TrackId, Option<String>),
    SetTrackInputPort(TrackId, u16),
    /// Pick the hardware MIDI input device for an instrument track.
    SetTrackMidiInputDevice(TrackId, Option<String>),
    /// Pick the hardware MIDI output device for an instrument track.
    SetTrackMidiOutputDevice(TrackId, Option<String>),
    /// Pick the input channel filter (`None` = omni / accept all).
    SetTrackMidiInputChannel(TrackId, Option<u8>),
    /// Pick the output channel (`None` = default to channel 1).
    SetTrackMidiOutputChannel(TrackId, Option<u8>),
    /// Toggle whether a parent track's sub-tracks are shown in the mixer.
    ToggleSubTracksVisible(TrackId),
    SetTrackOutput(TrackId, TrackOutput),
    /// Create a new track from a preset template.
    ///
    /// `id_hint`, if given, is the caller-allocated track id (the control
    /// endpoint's path), so the caller can address the new track without
    /// waiting for the engine's `*TrackAdded` echo. `None` is the GUI's
    /// path — the handler allocates a fresh id itself (ARCH-04 D-4; the
    /// engine has no allocator of its own left for either case any more).
    /// `name` overrides the preset's own name for the track only — the
    /// preset keeps its name in the library (ba todo #1303).
    AddTrackFromPreset {
        preset: Box<TrackPreset>,
        id_hint: Option<TrackId>,
        name: Option<String>,
    },
    /// Delete a user preset by name.
    DeleteUserPreset(String),
    /// Open the "Save track as preset" name prompt, seeded with the
    /// track's own name (ba todo #1303, finding P1).
    OpenSavePresetPrompt(TrackId),
    /// Live edit of the name in that prompt.
    SetSavePresetName(String),
    /// Dismiss the prompt without saving.
    CloseSavePresetPrompt,
    /// Capture a track — its mixer settings, its instrument identity and
    /// its whole plugin chain including each plugin's opaque state — as
    /// a reusable user preset (ba todo #1303, finding P1; control method
    /// `track.save_preset`).
    ///
    /// The capture pipeline behind this has always worked; nothing ever
    /// started it, so the preset menu could only list presets a user had
    /// hand-written as JSON. Saving is a two-step: this arms
    /// `pending_preset_save` and asks the engine for the plugins' state
    /// blobs, and the `AllPluginStatesSaved` echo writes the file.
    ///
    /// `overwrite` is the destructive-operation flag: without it, a name
    /// that already exists is refused rather than replaced.
    SaveTrackAsPreset {
        track_id: TrackId,
        name: String,
        overwrite: bool,
    },
    /// "Bounce in place" — render this instrument track to a fresh
    /// audio track and mute the source. Routes to either the offline
    /// bounce (for tracks with an internal synth) or the bounce
    /// dialog (for external-MIDI tracks that need a real-time record
    /// from a chosen audio input).
    BounceInPlace(TrackId),
    /// The internal-synth half of `BounceInPlace`, dispatched by its own
    /// handler once `classify_bounce` has resolved the route to
    /// `BounceMode::Internal` (FU-A10a). `BounceInPlace` only *asks* — it
    /// may end up opening the realtime dialog instead, where nothing
    /// records until `Bounce(BounceMessage::Confirm)` — so it classifies
    /// `Skip`, the same idiom `RequestRemoveTrack`/`ConfirmRemoveTrack`
    /// uses for a decision the classifier can't make from the message
    /// alone. This variant is the confirmed one-and-only edit: the
    /// offline render that actually mutates the project.
    BounceInPlaceOffline(TrackId),
    /// Sub-flow for the realtime "Bounce in place" dialog (external
    /// MIDI tracks). Grouped under one variant so the top-level
    /// `TrackMessage` doesn't accumulate dialog plumbing.
    Bounce(BounceMessage),
}

impl TrackMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::{CoalesceKey, UndoAction};
        match self {
            Self::SetTrackVolume(id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::TrackVolume(*id))
            }
            Self::SetTrackPan(id, _) => UndoAction::RecordCoalesced(CoalesceKey::TrackPan(*id)),
            Self::SetMasterVolume(_) => UndoAction::RecordCoalesced(CoalesceKey::MasterVolume),
            Self::SetTrackName(id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::TrackName(*id))
            }
            Self::ToggleSubTracksVisible(_) => UndoAction::Skip,
            // Dismissing the delete-confirmation dialog is a transient
            // UI gesture — nothing to undo.
            Self::CancelRemoveTrack => UndoAction::Skip,
            // Only asks: it opens the confirm dialog, or — for an empty
            // track — re-dispatches `ConfirmRemoveTrack`, which records
            // the delete (code review STATE-13).
            Self::RequestRemoveTrack(_) => UndoAction::Skip,
            // Only asks, same idiom as `RequestRemoveTrack`: routes to
            // either the realtime dialog (external MIDI — nothing records
            // until `Bounce(BounceMessage::Confirm)`) or re-dispatches
            // `BounceInPlaceOffline`, which records the one committed edit
            // (FU-A10a — this used to record unconditionally, so an
            // external-track bounce recorded twice: once here for opening
            // the dialog, empty since nothing had changed yet, and once
            // more on Confirm).
            Self::BounceInPlace(_) => UndoAction::Skip,
            // Preset operations that don't mutate project state: a
            // preset is a file on the machine, and saving one leaves the
            // project exactly as it was (ba todo #1303). The prompt
            // around it is transient UI for the same reason.
            Self::DeleteUserPreset(_)
            | Self::SaveTrackAsPreset { .. }
            | Self::OpenSavePresetPrompt(_)
            | Self::SetSavePresetName(_)
            | Self::CloseSavePresetPrompt => UndoAction::Skip,
            // Every other variant is a discrete, persisted edit.
            Self::AddTrack
            | Self::AddInstrumentTrack
            | Self::AddExternalInstrumentTrack
            | Self::AddVocalTrack
            | Self::AddControlTrack { .. }
            | Self::ConfirmRemoveTrack
            | Self::ToggleMute(..)
            | Self::ToggleSolo(..)
            | Self::ToggleRecordArm(..)
            | Self::ToggleMonitor(..)
            | Self::ToggleTrackMono(..)
            | Self::ToggleTrackFxBypass(..)
            | Self::SetTrackInputDevice(..)
            | Self::SetTrackInputPort(..)
            | Self::SetTrackMidiInputDevice(..)
            | Self::SetTrackMidiOutputDevice(..)
            | Self::SetTrackMidiInputChannel(..)
            | Self::SetTrackMidiOutputChannel(..)
            | Self::SetTrackOutput(..)
            | Self::AddTrackFromPreset { .. }
            | Self::BounceInPlaceOffline(..) => UndoAction::Record,
            Self::Bounce(m) => m.undo_action(),
        }
    }
}

/// User actions in the realtime bounce-in-place dialog (only shown for
/// external-MIDI instrument tracks). The dialog lifecycle: open →
/// `PickDevice` / `PickPort` → `Confirm` (kicks off the realtime bounce)
/// or `Cancel` (closes without side effects).
#[derive(Debug, Clone)]
pub enum BounceMessage {
    /// User picked an audio input device.
    PickDevice(Option<String>),
    /// User picked the starting input channel. In stereo mode the right
    /// channel is `port + 1`; in mono mode the same channel is captured
    /// to both L and R.
    PickPort(u16),
    /// Toggle stereo (`false`) vs mono (`true`) capture.
    SetMono(bool),
    /// User confirmed — kick off the realtime bounce.
    Confirm,
    /// User cancelled the dialog.
    Cancel,
    /// User clicked Cancel on the in-progress modal that's shown while
    /// a bounce is actually running. Distinct from `Cancel`, which only
    /// dismisses the pre-bounce input-picker dialog.
    CancelInProgress,
}

impl BounceMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // The input picker's choices and its dismissal only touch
            // `bounce_dialog`, which is session UI — never in the project.
            // Cancelling a running bounce asks the engine to stop; nothing
            // the project holds changes (A-10: these used to record, one
            // empty entry per click that also wiped the redo stack).
            Self::PickDevice(..)
            | Self::PickPort(..)
            | Self::SetMono(..)
            | Self::Cancel
            | Self::CancelInProgress => UndoAction::Skip,
            // Confirm adds the target track and starts the render; like a
            // pool import, the snapshot taken here is the pre-bounce
            // project the async result lands on top of.
            Self::Confirm => UndoAction::Record,
        }
    }
}

/// Where a "bounce in place" request should route. Computed from a
/// track and the project's MIDI clip list — the view uses it to grey
/// out the trigger button, and the update layer uses it to dispatch
/// either the offline render or the realtime input-picker dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BounceMode {
    /// Track has at least one synth plugin: render offline.
    Internal,
    /// Track drives external MIDI hardware: open the input picker
    /// dialog so the user picks which audio input to record from.
    External,
}

/// Classify a bounce request. Returns the routing mode on success or a
/// user-facing reason string when the track isn't bounce-able. The view
/// only inspects `is_ok()`; the update layer surfaces the message.
///
/// When a track has both an internal synth and a configured MIDI Out,
/// the external path wins — the user explicitly wired hardware output
/// for a reason and that's the "interesting" sound source.
pub fn classify_bounce(
    track: &TrackState,
    project_midi_clips: impl Iterator<Item = resonance_audio::types::TrackId>,
) -> Result<BounceMode, &'static str> {
    use resonance_audio::types::TrackType;
    if track.track_type != TrackType::Instrument {
        return Err("Bounce in place is only available on instrument tracks");
    }
    if track.sub_track.is_some() {
        return Err(
            "Bounce a parent track to capture its sub-tracks, not a sub-track itself",
        );
    }
    if !project_midi_clips.into_iter().any(|tid| tid == track.id) {
        return Err("Source track has no MIDI clips to bounce");
    }
    let has_external_midi = track.midi_output_device.is_some();
    let has_synth = !track.plugins.is_empty();
    if has_external_midi {
        Ok(BounceMode::External)
    } else if has_synth {
        Ok(BounceMode::Internal)
    } else {
        Err("Bounce: track has no sound source (no internal synth or MIDI Out)")
    }
}

pub fn handle(r: &mut Resonance, m: TrackMessage) -> Task<Message> {
    match m {
        TrackMessage::AddTrack => {
            // App-allocated since ARCH-04 D-4 — the engine has no track
            // counter of its own left — but still fire-and-forget: the
            // GUI waits for the `TrackAdded` echo to mirror the track,
            // same as before D-4.
            let id = r.allocate_track_id();
            let _ = r.engine.send(AudioCommand::AddTrack { id, name: None });
            r.ui.mixer.add_track_menu_open = false;
        }
        TrackMessage::AddInstrumentTrack => {
            let id = r.allocate_track_id();
            let _ = r.engine.send(AudioCommand::AddInstrumentTrack { id, name: None });
            r.ui.mixer.add_track_menu_open = false;
        }
        TrackMessage::AddExternalInstrumentTrack => {
            // Same track creation as `AddInstrumentTrack`, but the id is
            // allocated up front (rather than fire-and-forget) so we can
            // immediately enable external mode on it. The engine echoes
            // `InstrumentTrackAdded` for this id a beat later, which mirrors the
            // track into the registry. Enabling external mode here — before the
            // echo — is safe: `enable_external_instrument` only touches
            // `r.devices.external_instruments` + the engine, not the registry, and the
            // engine applies `SetExternalInstrument` after the track exists.
            //
            // Both effects (track creation + external state) fall under one undo
            // snapshot: this message classifies as `UndoAction::Record`, whose
            // pre-dispatch snapshot has neither the track nor the external entry,
            // so a single undo removes both and redo restores both.
            let track_id = r.allocate_track_id();
            let _ = r.engine.send(AudioCommand::AddInstrumentTrack {
                id: track_id,
                name: None,
            });
            crate::update::external_instrument::enable_external_instrument(r, track_id);
            r.ui.mixer.add_track_menu_open = false;
        }
        TrackMessage::AddVocalTrack => {
            let id = r.allocate_track_id();
            let _ = r.engine.send(AudioCommand::AddVocalTrack { id, name: None });
            r.ui.mixer.add_track_menu_open = false;
        }
        TrackMessage::AddControlTrack { id, kind, name } => {
            use crate::state::ControlTrackKind;
            // The id is already allocated (by the control handler, so the
            // reply can return it immediately); the engine echoes
            // `*TrackAdded { id }` which mirrors the track into the
            // registry. Drums queue a deferred instrument-type set for
            // that echo.
            let cmd = match kind {
                ControlTrackKind::Vocal => AudioCommand::AddVocalTrack {
                    id,
                    name: name.clone(),
                },
                ControlTrackKind::Audio => AudioCommand::AddTrack {
                    id,
                    name: name.clone(),
                },
                ControlTrackKind::Instrument
                | ControlTrackKind::Drums
                | ControlTrackKind::External => AudioCommand::AddInstrumentTrack {
                    id,
                    name: name.clone(),
                },
            };
            let _ = r.engine.send(cmd);
            // Defer name + drum-type application to the engine echo (the
            // registry mirror ignores the engine's name and always makes
            // a synth track); apply now too, in case the echo already
            // landed (tests drive it synchronously).
            r.control.pending_tracks.insert(
                id,
                crate::state::PendingControlTrack {
                    kind,
                    name: name.clone(),
                },
            );
            r.apply_pending_control_track(id);
            // External mode goes on in the same undo step as the track
            // itself, exactly as `AddExternalInstrumentTrack` does for
            // the Add-Track menu — one undo removes both.
            if matches!(kind, ControlTrackKind::External) {
                crate::update::external_instrument::enable_external_instrument(r, id);
            }
        }
        TrackMessage::RequestRemoveTrack(id) => {
            let has_audio = r.clips.iter().any(|c| c.track_id == id);
            let has_midi = r.midi_clips.iter().any(|c| c.track_id == id);
            // The request itself is `Skip` for undo — opening the confirm
            // dialog is no edit (code review STATE-13). An empty track
            // needs no confirm, so it goes straight to the confirmed
            // delete, recorded exactly as a dispatched `ConfirmRemoveTrack`
            // would be — that delete is the one undo entry.
            r.modals.confirm_delete_track = Some(id);
            if !(has_audio || has_midi) {
                let _ = r.record_undo(&Message::Track(TrackMessage::ConfirmRemoveTrack));
                return handle(r, TrackMessage::ConfirmRemoveTrack);
            }
        }
        TrackMessage::ConfirmRemoveTrack => {
            if let Some(id) = r.modals.confirm_delete_track.take() {
                r.ui.interaction.deselect_track(id);
                if r.compose.expanded_track_id == Some(id) {
                    r.compose.expanded_track_id = None;
                }
                // Tear down any freeze cache the track owned (ba todo #577).
                r.cleanup_freeze_on_delete(id);
                let _ = r.engine.send(AudioCommand::RemoveTrack { track_id: id });
                // Mirror the removal now, not on the `TrackRemoved` echo,
                // so an undo before the echo sees it (code review
                // STATE-10). The engine answers for the track and for each
                // sub-track it drops with it; those echoes are owed, so a
                // late one cannot remove what an undo has put back under
                // the same id (ARCH-01 A-13i).
                let subs: Vec<TrackId> = r
                    .registry
                    .tracks
                    .iter()
                    .filter(|t| t.sub_track.is_some_and(|l| l.parent_track_id == id))
                    .map(|t| t.id)
                    .collect();
                r.io.restore_echoes.expect_track_removed(id);
                for sub in subs {
                    r.io.restore_echoes.expect_track_removed(sub);
                }
                crate::engine_events::tracks::removed(r, id);
            }
        }
        TrackMessage::CancelRemoveTrack => {
            r.modals.confirm_delete_track = None;
        }
        TrackMessage::SetTrackVolume(id, vol_db) => {
            let _ = r.engine.send(AudioCommand::SetTrackVolume {
                track_id: id,
                volume: db_to_gain(vol_db),
            });
            r.with_track_mut(id, |t| t.volume = vol_db);
        }
        TrackMessage::SetTrackPan(id, pan) => {
            let _ = r.engine
                .send(AudioCommand::SetTrackPan { track_id: id, pan });
            r.with_track_mut(id, |t| t.pan = pan);
        }
        TrackMessage::SetMasterVolume(vol_db) => {
            let _ = r.engine.send(AudioCommand::SetMasterVolume {
                volume: db_to_gain(vol_db),
            });
            r.master.volume = vol_db;
        }
        TrackMessage::ToggleMute(id) => {
            let new_muted = r.with_track_mut(id, |t| {
                t.muted = !t.muted;
                t.muted
            });
            if let Some(own) = new_muted {
                // A group's macro mute composes with the track's own mute,
                // so the engine always receives the *effective* mute: the
                // track stays muted while its group mute holds even after
                // its own mute is cleared (todo #687). Shared with the
                // macro-toggle cascade and every restore (FU-A13a).
                let muted = r.track_groups.effective_mute(id, own);
                let _ = r.engine.send(AudioCommand::SetTrackMute {
                    track_id: id,
                    muted,
                });
            }
        }
        TrackMessage::ToggleSolo(id) => {
            let new_soloed = r.with_track_mut(id, |t| {
                t.soloed = !t.soloed;
                t.soloed
            });
            if let Some(own) = new_soloed {
                // A group's macro solo composes with the track's own solo,
                // so the engine always receives the *effective* solo: the
                // track stays soloed while its group solo holds even after
                // its own solo is cleared (todo #688). Shared with the
                // macro-toggle cascade and every restore (FU-A13a).
                let soloed = r.track_groups.effective_solo(id, own);
                let _ = r.engine.send(AudioCommand::SetTrackSolo {
                    track_id: id,
                    soloed,
                });
            }
        }
        TrackMessage::ToggleRecordArm(id) => {
            let default_device = r.devices.input.default_name.clone();
            let auto_device = r.with_track_mut(id, |t| {
                t.record_armed = !t.record_armed;
                if t.record_armed && t.input_device_name.is_none() {
                    t.input_device_name = default_device.clone();
                }
                (t.record_armed, t.input_device_name.clone())
            });
            if let Some((armed, device)) = auto_device {
                if armed && device.is_some() {
                    let _ = r.engine.send(AudioCommand::SetTrackInputDevice {
                        track_id: id,
                        device_name: device,
                    });
                }
                let _ = r.engine.send(AudioCommand::SetTrackRecordArm {
                    track_id: id,
                    armed,
                });
            }
        }
        TrackMessage::ToggleMonitor(id) => {
            let new_enabled = r.with_track_mut(id, |t| {
                t.monitor_enabled = !t.monitor_enabled;
                t.monitor_enabled
            });
            if let Some(enabled) = new_enabled {
                let _ = r.engine.send(AudioCommand::SetTrackMonitor {
                    track_id: id,
                    enabled,
                });
            }
        }
        TrackMessage::SetTrackName(track_id, name) => {
            r.with_track_mut(track_id, |t| t.name = name);
        }
        TrackMessage::ToggleTrackFxBypass(id) => {
            let new_bypass = r.with_track_mut(id, |t| {
                t.fx_bypassed = !t.fx_bypassed;
                t.fx_bypassed
            });
            if let Some(bypassed) = new_bypass {
                let _ = r.engine.send(AudioCommand::SetTrackFxBypass {
                    track_id: id,
                    bypassed,
                });
            }
        }
        TrackMessage::ToggleTrackMono(id) => {
            let new_mono = r.with_track_mut(id, |t| {
                t.mono = !t.mono;
                t.mono
            });
            if let Some(mono) = new_mono {
                let _ = r.engine
                    .send(AudioCommand::SetTrackMono { track_id: id, mono });
            }
        }
        TrackMessage::SetTrackInputDevice(id, device_name) => {
            let updated = r.with_track_mut(id, |t| {
                t.input_device_name = device_name.clone();
                t.input_port_index = 0;
            });
            if updated.is_some() {
                let _ = r.engine.send(AudioCommand::SetTrackInputDevice {
                    track_id: id,
                    device_name,
                });
                let _ = r.engine.send(AudioCommand::SetTrackInputPort {
                    track_id: id,
                    port_index: 0,
                });
            }
        }
        TrackMessage::SetTrackInputPort(id, port_index) => {
            let updated = r.with_track_mut(id, |t| t.input_port_index = port_index);
            if updated.is_some() {
                let _ = r.engine.send(AudioCommand::SetTrackInputPort {
                    track_id: id,
                    port_index,
                });
            }
        }
        TrackMessage::SetTrackMidiInputDevice(id, device) => {
            let updated = r.with_track_mut(id, |t| {
                t.midi_input_device = device.clone();
                t.midi_input_channel
            });
            if let Some(channel) = updated {
                let _ = r.engine.send(AudioCommand::SetTrackMidiInput {
                    track_id: id,
                    device,
                    channel,
                });
            }
        }
        TrackMessage::SetTrackMidiOutputDevice(id, device) => {
            let updated = r.with_track_mut(id, |t| {
                t.midi_output_device = device.clone();
                t.midi_output_channel
            });
            if let Some(channel) = updated {
                let _ = r.engine.send(AudioCommand::SetTrackMidiOutput {
                    track_id: id,
                    device,
                    channel,
                });
            }
        }
        TrackMessage::SetTrackMidiInputChannel(id, channel) => {
            let device = r.with_track_mut(id, |t| {
                t.midi_input_channel = channel;
                t.midi_input_device.clone()
            });
            if let Some(device) = device {
                let _ = r.engine.send(AudioCommand::SetTrackMidiInput {
                    track_id: id,
                    device,
                    channel,
                });
            }
        }
        TrackMessage::SetTrackMidiOutputChannel(id, channel) => {
            let device = r.with_track_mut(id, |t| {
                t.midi_output_channel = channel;
                t.midi_output_device.clone()
            });
            if let Some(device) = device {
                let _ = r.engine.send(AudioCommand::SetTrackMidiOutput {
                    track_id: id,
                    device,
                    channel,
                });
            }
        }
        TrackMessage::ToggleSubTracksVisible(id) => {
            if !r.ui.mixer.expanded_sub_track_parents.insert(id) {
                r.ui.mixer.expanded_sub_track_parents.remove(&id);
            }
        }
        TrackMessage::SetTrackOutput(track_id, output) => {
            let _ = r.engine
                .send(AudioCommand::SetTrackOutput { track_id, output });
            r.with_track_mut(track_id, |t| t.output = output);
        }
        TrackMessage::AddTrackFromPreset {
            preset,
            id_hint,
            name,
        } => {
            // `id_hint` is `Some` from the control endpoint (which had to
            // allocate before building this message, to return the id in
            // its reply) and `None` from the GUI's preset picker, which
            // allocates right here instead — either way every add carries
            // a concrete `id` (ARCH-04 D-4).
            let id = id_hint.unwrap_or_else(|| r.allocate_track_id());
            let track_name = Some(name.unwrap_or_else(|| preset.name.clone()));
            let cmd = match preset.track_type.as_str() {
                "instrument" => AudioCommand::AddInstrumentTrack {
                    id,
                    name: track_name,
                },
                "vocal" => AudioCommand::AddVocalTrack {
                    id,
                    name: track_name,
                },
                _ => AudioCommand::AddTrack {
                    id,
                    name: track_name,
                },
            };
            let _ = r.engine.send(cmd);
            r.presets.pending_track_preset = Some(*preset);
            r.ui.mixer.add_track_menu_open = false;
        }
        TrackMessage::OpenSavePresetPrompt(track_id) => {
            // Seed with the track's own name: it is right often enough
            // to be worth a single Enter, and wrong in a way the user
            // can see before committing.
            let name = r
                .registry
                .tracks
                .iter()
                .find(|t| t.id == track_id)
                .map(|t| t.name.clone())
                .unwrap_or_default();
            let exists = crate::presets::user_preset_exists(&name);
            r.ui.interaction.preset_save = Some(crate::state::PresetSaveState {
                track_id,
                name,
                exists,
            });
            r.ui.interaction.track_menu = None;
        }
        TrackMessage::SetSavePresetName(name) => {
            if let Some(prompt) = r.ui.interaction.preset_save.as_mut() {
                prompt.exists = crate::presets::user_preset_exists(name.trim());
                prompt.name = name;
            }
        }
        TrackMessage::CloseSavePresetPrompt => {
            r.ui.interaction.preset_save = None;
        }
        TrackMessage::SaveTrackAsPreset {
            track_id,
            name,
            overwrite,
        } => {
            handle_save_track_as_preset(r, track_id, name, overwrite);
        }
        TrackMessage::DeleteUserPreset(name) => {
            if let Err(e) = crate::presets::delete_user_preset(&name) {
                r.banners.error_message = Some(format!("Delete preset: {e}"));
            }
            r.presets.user_presets = crate::presets::load_user_presets();
        }
        TrackMessage::BounceInPlace(track_id) => {
            handle_bounce_in_place(r, track_id);
        }
        TrackMessage::BounceInPlaceOffline(track_id) => {
            internal_bounce_dispatch(r, track_id);
        }
        TrackMessage::Bounce(BounceMessage::PickDevice(device)) => {
            if let Some(d) = r.modals.bounce_dialog.as_mut() {
                d.selected_device = device;
                d.selected_port = 0;
            }
        }
        TrackMessage::Bounce(BounceMessage::PickPort(port)) => {
            if let Some(d) = r.modals.bounce_dialog.as_mut() {
                d.selected_port = port;
            }
        }
        TrackMessage::Bounce(BounceMessage::SetMono(mono)) => {
            if let Some(d) = r.modals.bounce_dialog.as_mut() {
                d.mono = mono;
                // Stereo pairs need an even start channel; switching back
                // to stereo from a port that became invalid would dump the
                // user on the right channel of an old pair. Snap to 0.
                if !mono && d.selected_port % 2 != 0 {
                    d.selected_port = 0;
                }
            }
        }
        TrackMessage::Bounce(BounceMessage::Cancel) => {
            r.modals.bounce_dialog = None;
        }
        TrackMessage::Bounce(BounceMessage::CancelInProgress) => {
            // Engine clears `bounce_in_progress` when it emits
            // `TrackBounceCancelled`; don't drop it locally so the
            // modal stays up while the engine teardown runs (offline
            // is fast; realtime needs the audio thread to settle).
            let _ = r.engine.send(AudioCommand::CancelBounce);
        }
        TrackMessage::Bounce(BounceMessage::Confirm) => {
            handle_bounce_dialog_confirm(r);
        }
    }
    Task::none()
}

fn handle_bounce_dialog_confirm(r: &mut Resonance) {
    let Some(dialog) = r.modals.bounce_dialog.take() else {
        return;
    };
    let Some(device) = dialog.selected_device.clone() else {
        r.banners.error_message = Some("Pick an audio input device first".into());
        // Keep the dialog open by re-stashing it.
        r.modals.bounce_dialog = Some(dialog);
        return;
    };
    let Some(source) = r.registry.tracks.iter().find(|t| t.id == dialog.source_track_id) else {
        r.banners.error_message = Some("Bounce: source track not found".into());
        return;
    };
    if r.transport.playing {
        r.banners.error_message = Some("Stop transport before bouncing".into());
        r.modals.bounce_dialog = Some(dialog);
        return;
    }
    // A realtime bounce plays the project live while an offline control
    // measurement is rendering through the same plugin instances — the
    // same conflict the offline renderers have (see
    // `offline_measure_in_progress`), so it refuses too.
    if r.offline_measure_in_progress() {
        r.banners.error_message =
            Some("A measurement is in progress; bounce again when it finishes".into());
        r.modals.bounce_dialog = Some(dialog);
        return;
    }

    let source_name = source.name.clone();
    let target_track_id = r.allocate_track_id();
    let track_name = format!("{source_name} bounce");

    let _ = r.engine.send(AudioCommand::AddTrack {
        id: target_track_id,
        name: Some(track_name),
    });
    let _ = r.engine.send(AudioCommand::BounceTrackRealtimeToAudio {
        source_track_id: dialog.source_track_id,
        target_track_id,
        input_device_name: device,
        input_port_index: dialog.selected_port,
        mono: dialog.mono,
    });
    r.modals.bounce_in_progress = Some(crate::state::BounceProgressState {
        mode: crate::state::BounceMode::Realtime,
        source_name,
        fraction: 0.0,
    });
}

/// Dispatch a "bounce in place" request — runs the source-track
/// classifier and either fires the offline render command (internal
/// synth) or opens the realtime input-picker dialog (external MIDI).
/// Arm a track-preset capture (ba todo #1303, finding P1).
///
/// Everything a preset needs except the plugins' opaque state blobs is
/// already in the app; only the engine can ask a plugin for one. So this
/// stores the intent and sends `SaveAllPluginStates`, and the echo
/// (`engine_events::project_io::all_plugin_states_saved`) writes the
/// file. `pending_preset_save` has existed — declared, initialised to
/// `None`, taken on that echo — with nothing in the app ever setting it,
/// which is why the preset menu could only list presets someone had
/// written by hand.
///
/// Refuses rather than replaces: a name already on disk needs
/// `overwrite`, the same rule `track.delete` and the render targets
/// follow. Both surfaces come through here, so the refusal cannot differ
/// between them.
fn handle_save_track_as_preset(
    r: &mut Resonance,
    track_id: resonance_audio::types::TrackId,
    name: String,
    overwrite: bool,
) {
    let name = name.trim().to_string();
    if name.is_empty() {
        r.banners.error_message = Some("Save preset: name a preset before saving it".to_string());
        return;
    }
    if !r.registry.tracks.iter().any(|t| t.id == track_id) {
        r.banners.error_message = Some(format!("Save preset: no track {track_id}"));
        return;
    }
    if !overwrite && crate::presets::user_preset_exists(&name) {
        // The GUI reaches this only if the prompt's own guard was
        // bypassed; it normally offers "Overwrite" instead.
        r.banners.error_message = Some(format!(
            "Save preset: a preset named {name:?} already exists — save it under another name,              or overwrite it"
        ));
        return;
    }

    r.presets.pending_preset_save = Some(crate::PendingPresetSave { track_id, name });
    let _ = r.engine.send(AudioCommand::SaveAllPluginStates);
    r.ui.interaction.preset_save = None;
}

fn handle_bounce_in_place(r: &mut Resonance, track_id: resonance_audio::types::TrackId) {
    let Some(source) = r.registry.tracks.iter().find(|t| t.id == track_id) else {
        r.banners.error_message = Some("Bounce: source track not found".into());
        return;
    };
    let mode = match classify_bounce(source, r.midi_clips.iter().map(|c| c.track_id)) {
        Ok(mode) => mode,
        Err(msg) => {
            r.banners.error_message = Some(msg.into());
            return;
        }
    };
    if r.transport.playing {
        r.banners.error_message = Some("Stop transport before bouncing".into());
        return;
    }
    // An offline control measurement holds the offline renderer
    // exclusively; a bounce on top of it would drive the same live
    // plugin instances from two renderers at once (mirrors
    // `meter.measure` refusing while a bounce runs).
    if r.offline_measure_in_progress() {
        r.banners.error_message =
            Some("A measurement is in progress; bounce again when it finishes".into());
        return;
    }

    match mode {
        BounceMode::External => {
            r.modals.bounce_dialog = Some(crate::state::BounceDialogState {
                source_track_id: track_id,
                selected_device: r.devices.input.default_name.clone(),
                selected_port: 0,
                mono: false,
            });
            // Make sure the input device list is fresh for the dialog.
            let _ = r.engine.send(AudioCommand::ListInputDevices);
        }
        BounceMode::Internal => {
            // `BounceInPlace` classifies `Skip` — it only asks — so the
            // one committed edit is recorded here, under the dedicated
            // `BounceInPlaceOffline` message, exactly as
            // `RequestRemoveTrack` records `ConfirmRemoveTrack` for its
            // no-confirmation-needed path (FU-A10a).
            let _ = r.record_undo(&Message::Track(TrackMessage::BounceInPlaceOffline(
                track_id,
            )));
            internal_bounce_dispatch(r, track_id);
        }
    }
}

/// Allocate the target track + clip ids and fire the offline bounce
/// command. Caller has already validated the source track.
fn internal_bounce_dispatch(r: &mut Resonance, track_id: resonance_audio::types::TrackId) {
    let source_name = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .map(|t| t.name.clone())
        .unwrap_or_default();
    let target_track_id = r.allocate_track_id();
    let target_clip_id = r.compose.fresh_derived_clip_id();

    let track_name = format!("{source_name} bounce");
    let clip_name = track_name.clone();

    r.modals.bounce_in_progress = Some(crate::state::BounceProgressState {
        mode: crate::state::BounceMode::Offline,
        source_name: source_name.clone(),
        fraction: 0.0,
    });

    let _ = r.engine.send(AudioCommand::AddTrack {
        id: target_track_id,
        name: Some(track_name),
    });
    let _ = r.engine.send(AudioCommand::BounceTrackToAudio {
        source_track_id: track_id,
        target_track_id,
        target_clip_id,
        name: clip_name,
    });
}
