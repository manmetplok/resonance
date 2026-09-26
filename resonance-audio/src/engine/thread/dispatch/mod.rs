//! Per-category command dispatch sub-functions.
//!
//! [`dispatch`] is the single entry point called from the engine loop. It
//! routes each [`AudioCommand`] to the appropriate category handler below:
//!
//! | Sub-dispatcher          | Commands handled                              |
//! |-------------------------|-----------------------------------------------|
//! | [`transport`]           | Play/Pause/Stop/Seek/BPM/loop                 |
//! | [`clips`]               | Audio clips, warp, automation, project dir    |
//! | [`tracks`]              | Track add/remove/volume/pan/mute/arm/freeze   |
//! | [`plugins`]             | CLAP plugin add/remove/param/editor/state     |
//! | [`bounce`]              | Bounce, export, stems, freeze                 |
//! | [`midi`]                | MIDI clips, notes, live play, ext instruments |
//! | [`busses`]              | Busses, aux sends, master FX chain            |
//! | [`audition`]            | Audition preview                              |
//! | [`midi_map`]            | MIDI learn & hardware controller mapping      |
//! | [`reference`]           | Reference track A/B comparison               |
//! | [`peaks`]               | Peak meter polling                            |
//! | [`takes`]               | Take-lane comp edits, solo, project restore   |

mod audition;
mod bounce;
mod busses;
mod clips;
mod midi;
mod midi_map;
mod peaks;
mod plugins;
mod reference;
mod takes;
mod tracks;
mod transport;

use super::{HandlerCtx, HandlerState};
use crate::types::*;

// ---------------------------------------------------------------------------
// Top-level router
// ---------------------------------------------------------------------------

/// Route `cmd` to the appropriate category sub-dispatcher.
///
/// Every variant of [`AudioCommand`] is handled exactly once. `ShutDown` is
/// included for exhaustiveness but is unreachable here — the engine loop
/// breaks on it before calling this function.
pub(super) fn dispatch(ctx: &HandlerCtx, state: &mut HandlerState, cmd: AudioCommand) {
    match cmd {
        // External sidechain (key) routing: which source feeds a
        // plugin instance's key port.
        AudioCommand::SetSidechainRoute {
            plugin,
            source,
            enabled,
        } => crate::engine::sidechain::handle_set(ctx, state, plugin, source, enabled),
        AudioCommand::ClearSidechainRoute { plugin } => {
            crate::engine::sidechain::handle_clear(ctx, state, plugin)
        }

        // Transport
        AudioCommand::Play
        | AudioCommand::Record { .. }
        | AudioCommand::Pause
        | AudioCommand::Stop
        | AudioCommand::SeekTo(..)
        | AudioCommand::SetBpm { .. }
        | AudioCommand::SetTempoEvents { .. }
        | AudioCommand::SetTimeSignature { .. }
        | AudioCommand::SetMetronomeEnabled { .. }
        | AudioCommand::SetLoopRange { .. }
        | AudioCommand::SetLoopRecordMode(..) => transport::dispatch_transport(ctx, state, cmd),

        // Take lanes: comp edits, active-take selection, take / lane
        // removal, project-load restore
        AudioCommand::SetTakeComp { .. }
        | AudioCommand::SetActiveTake { .. }
        | AudioCommand::RemoveTake { .. }
        | AudioCommand::RemoveTakeGroup { .. }
        | AudioCommand::RestoreTakeGroups { .. }
        | AudioCommand::LoadTakeClipFromWav { .. } => takes::dispatch_takes(ctx, state, cmd),

        // Audio clips + automation + project
        AudioCommand::ImportClip { .. }
        | AudioCommand::ImportAudioToPool { .. }
        | AudioCommand::ReserveAssetIds { .. }
        | AudioCommand::MoveClip { .. }
        | AudioCommand::TrimClip { .. }
        | AudioCommand::DeleteClip { .. }
        | AudioCommand::SplitClip { .. }
        | AudioCommand::SetClipFade { .. }
        | AudioCommand::SetClipGain { .. }
        | AudioCommand::SetClipWarp { .. }
        | AudioCommand::SetClipWarpMarkers { .. }
        | AudioCommand::DetectClipTempo { .. }
        | AudioCommand::AnalyzeClipPitch { .. }
        | AudioCommand::SetAutomationLane { .. }
        | AudioCommand::ClearAutomationLane { .. }
        | AudioCommand::SetAutomationReadEnabled { .. }
        | AudioCommand::SetProjectDir(..)
        | AudioCommand::LoadClipFromWav { .. }
        | AudioCommand::SaveClipsToProjectDir
        | AudioCommand::PersistClipWavs => clips::dispatch_clips(ctx, state, cmd),

        // Tracks
        AudioCommand::SetTrackVolume { .. }
        | AudioCommand::SetTrackPan { .. }
        | AudioCommand::SetTrackMute { .. }
        | AudioCommand::SetMasterVolume { .. }
        | AudioCommand::SetTrackSolo { .. }
        | AudioCommand::AddTrack { .. }
        | AudioCommand::CreateSubTrack { .. }
        | AudioCommand::RemoveTrack { .. }
        | AudioCommand::SetTrackRecordArm { .. }
        | AudioCommand::SetTrackMono { .. }
        | AudioCommand::SetTrackMonitor { .. }
        | AudioCommand::SetTrackPlaybackSource { .. }
        | AudioCommand::SetTrackInputDevice { .. }
        | AudioCommand::SetTrackInputPort { .. }
        | AudioCommand::ListInputDevices
        | AudioCommand::ClearAll
        | AudioCommand::SetTrackFrozenSource { .. }
        | AudioCommand::UnfreezeTrack { .. }
        | AudioCommand::SetTrackFxBypass { .. } => tracks::dispatch_tracks(ctx, state, cmd),

        // Plugins
        AudioCommand::AddPlugin { .. }
        | AudioCommand::RemovePlugin { .. }
        | AudioCommand::MovePlugin { .. }
        | AudioCommand::ScanPlugins
        | AudioCommand::RescanPlugins
        | AudioCommand::SetPluginParam { .. }
        | AudioCommand::SetPluginBypass { .. }
        | AudioCommand::OpenPluginEditor { .. }
        | AudioCommand::ClosePluginEditor { .. }
        | AudioCommand::SavePluginState { .. }
        | AudioCommand::LoadPluginState { .. }
        | AudioCommand::SaveAllPluginStates => plugins::dispatch_plugins(ctx, state, cmd),

        // Bounce / export / freeze
        AudioCommand::BounceToWav { .. }
        | AudioCommand::ExportAudio { .. }
        | AudioCommand::BounceTrackToAudio { .. }
        | AudioCommand::BounceTrackRealtimeToAudio { .. }
        | AudioCommand::CancelBounce
        | AudioCommand::ExportStems { .. }
        | AudioCommand::CancelStemExport
        | AudioCommand::MeasureMix { .. }
        | AudioCommand::FreezeTrack { .. }
        | AudioCommand::CancelFreeze => bounce::dispatch_bounce(ctx, state, cmd),

        // MIDI clips, notes, live play, external instruments
        AudioCommand::AddInstrumentTrack { .. }
        | AudioCommand::AddVocalTrack { .. }
        | AudioCommand::CreateMidiClip { .. }
        | AudioCommand::LoadMidiClipDirect { .. }
        | AudioCommand::MoveMidiClip { .. }
        | AudioCommand::TrimMidiClip { .. }
        | AudioCommand::DeleteMidiClip { .. }
        | AudioCommand::AddMidiNote { .. }
        | AudioCommand::RemoveMidiNote { .. }
        | AudioCommand::MoveMidiNote { .. }
        | AudioCommand::ResizeMidiNote { .. }
        | AudioCommand::SetMidiNoteVelocity { .. }
        | AudioCommand::SetMidiClipNotes { .. }
        | AudioCommand::QuantizeMidiNotes { .. }
        | AudioCommand::HumanizeMidiNotes { .. }
        | AudioCommand::ApplyGrooveToClip { .. }
        | AudioCommand::ExtractGrooveFromClip { .. }
        | AudioCommand::SendNoteOn { .. }
        | AudioCommand::SendNoteOff { .. }
        | AudioCommand::ListMidiInputDevices
        | AudioCommand::ListMidiOutputDevices
        | AudioCommand::SetTrackMidiInput { .. }
        | AudioCommand::SetTrackMidiOutput { .. }
        | AudioCommand::SetTrackDeviceParams { .. }
        | AudioCommand::SetExternalInstrument { .. }
        | AudioCommand::ClearExternalInstrument { .. }
        | AudioCommand::SetExternalInstrumentPatch { .. }
        | AudioCommand::SetExternalInstrumentLatencyOffset { .. }
        | AudioCommand::CheckExternalInstrumentDevices { .. }
        | AudioCommand::ResendExternalInstrumentPatches
        | AudioCommand::DetectExternalInstrumentLatency { .. }
        | AudioCommand::SetMidiClockOutput { .. }
        | AudioCommand::SetMidiClockInput { .. } => midi::dispatch_midi(ctx, state, cmd),

        // Busses, aux sends, master FX chain
        AudioCommand::AddBus { .. }
        | AudioCommand::RemoveBus { .. }
        | AudioCommand::SetBusVolume { .. }
        | AudioCommand::SetBusPan { .. }
        | AudioCommand::SetBusMute { .. }
        | AudioCommand::SetBusName { .. }
        | AudioCommand::SetTrackOutput { .. }
        | AudioCommand::AddPluginToBus { .. }
        | AudioCommand::RemovePluginFromBus { .. }
        | AudioCommand::MovePluginInBus { .. }
        | AudioCommand::SetBusRole { .. }
        | AudioCommand::AddAuxSend { .. }
        | AudioCommand::SetAuxSend { .. }
        | AudioCommand::RemoveAuxSend { .. }
        | AudioCommand::AddPluginToMaster { .. }
        | AudioCommand::RemovePluginFromMaster { .. }
        | AudioCommand::MovePluginInMaster { .. }
        | AudioCommand::SetBusFxBypass { .. }
        | AudioCommand::SetMasterFxBypass { .. } => busses::dispatch_busses(ctx, state, cmd),

        // Audition preview
        AudioCommand::AuditionFile { .. }
        | AudioCommand::StopAudition
        | AudioCommand::SetAuditionOptions { .. } => audition::dispatch_audition(ctx, cmd),

        // MIDI Learn & hardware controller mapping
        AudioCommand::SetMidiBinding { .. }
        | AudioCommand::ClearMidiBinding { .. }
        | AudioCommand::SetControllerMap { .. }
        | AudioCommand::ClearAllMidiBindings
        | AudioCommand::SetControlSurfaceInput { .. }
        | AudioCommand::EnterMidiLearn { .. }
        | AudioCommand::CancelMidiLearn => midi_map::dispatch_midi_map(ctx, cmd),

        // Reference track A/B
        AudioCommand::LoadReferenceTrack { .. }
        | AudioCommand::ReferenceAnalyzed { .. }
        | AudioCommand::RemoveReferenceTrack { .. }
        | AudioCommand::SetActiveReference { .. }
        | AudioCommand::ClearActiveReference
        | AudioCommand::SetABSource { .. }
        | AudioCommand::SetRefLoudnessMatch { .. }
        | AudioCommand::SetRefTrim { .. }
        | AudioCommand::AddRefMarker { .. }
        | AudioCommand::RemoveRefMarker { .. }
        | AudioCommand::SetRefPosition { .. }
        | AudioCommand::SetRefLoopToMix { .. }
        | AudioCommand::PollABMeters => reference::dispatch_reference(ctx, state, cmd),

        AudioCommand::PollPeaks => peaks::handle_poll_peaks(ctx),

        AudioCommand::QueryIoLatency => {
            use std::sync::atomic::Ordering;
            // Capture latency is only meaningful while an input stream
            // is actually open — the atomic keeps its last value after
            // teardown, so gate on the live handle instead of chasing
            // every teardown site.
            let capture = if state.rec.input_stream.is_some() {
                ctx.shared.capture_latency_samples.load(Ordering::Relaxed)
            } else {
                0
            };
            let playback = ctx.shared.playback_latency_samples.load(Ordering::Relaxed);
            let _ = ctx.event_tx.send(AudioEvent::IoLatencyReport {
                capture_samples: capture,
                playback_samples: playback,
                round_trip_samples: capture + playback,
            });
        }

        AudioCommand::ShutDown => {
            // Handled in the engine_thread loop directly; this arm is
            // unreachable in practice but keeps the match exhaustive.
        }
    }
}
