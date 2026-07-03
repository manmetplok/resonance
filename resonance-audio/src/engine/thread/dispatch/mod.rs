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

mod audition;
mod bounce;
mod busses;
mod clips;
mod midi;
mod midi_map;
mod peaks;
mod plugins;
mod reference;
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

        // Audio clips + automation + project
        AudioCommand::ImportClip { .. }
        | AudioCommand::ImportAudioToPool { .. }
        | AudioCommand::MoveClip { .. }
        | AudioCommand::TrimClip { .. }
        | AudioCommand::DeleteClip { .. }
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
        | AudioCommand::SaveClipsToProjectDir => clips::dispatch_clips(ctx, state, cmd),

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
        | AudioCommand::ScanPlugins
        | AudioCommand::SetPluginParam { .. }
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
        | AudioCommand::SetBusRole { .. }
        | AudioCommand::SetAuxSend { .. }
        | AudioCommand::RemoveAuxSend { .. }
        | AudioCommand::AddPluginToMaster { .. }
        | AudioCommand::RemovePluginFromMaster { .. }
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
        | AudioCommand::SetABSource { .. }
        | AudioCommand::SetRefLoudnessMatch { .. }
        | AudioCommand::SetRefTrim { .. }
        | AudioCommand::AddRefMarker { .. }
        | AudioCommand::RemoveRefMarker { .. }
        | AudioCommand::SetRefPosition { .. }
        | AudioCommand::SetRefLoopToMix { .. }
        | AudioCommand::PollABMeters => reference::dispatch_reference(ctx, state, cmd),

        AudioCommand::PollPeaks => peaks::handle_poll_peaks(ctx),

        AudioCommand::ShutDown => {
            // Handled in the engine_thread loop directly; this arm is
            // unreachable in practice but keeps the match exhaustive.
        }
    }
}
