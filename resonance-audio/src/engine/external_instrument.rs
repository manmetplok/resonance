//! Engine-thread handlers for external-instrument track config (doc #169,
//! epic #39).
//!
//! An external-instrument track pairs a hardware MIDI output (the track's
//! existing `midi_output_device` / `midi_output_channel`) with an audio-return
//! input (the track's existing `input_device_name` / input port), plus the
//! extra config a plain track has nowhere to hold: the selected bank/program
//! and a manual latency offset. That extra config lives engine-thread-local in
//! [`ExternalInstruments`], keyed by [`TrackId`] — one entry per track. The
//! *presence* of an entry is what marks the track as an external instrument;
//! clearing it removes the mode. Device, channel, monitor and record-arm are
//! never duplicated here — they stay on the engine-side `Track`, edited through
//! the existing `SetTrackMidiOutput` / `SetTrackInputDevice` /
//! `SetTrackMonitor` / `SetTrackRecordArm` commands.
//!
//! Every mutation echoes the resulting config back to the app via an
//! [`AudioEvent`] so the app mirror stays in lock-step. Setting bank/program
//! additionally triggers a "patch send" (Bank Select + Program Change) to the
//! MIDI output device. If either endpoint is offline the engine reports a
//! recoverable device-offline event and leaves the route intact so a replug
//! reconnects — it never tears the config down.
//!
//! Everything here runs on the engine control thread; the audio callback is not
//! touched, so there are no realtime allocations. Applying the latency offset
//! to the audio return is a later step — this is the config boundary only.

use std::collections::{HashMap, HashSet};

use crossbeam_channel::Sender;
use resonance_common::ExternalInstrument;

use crate::midi_hardware::{enumerate_midi_outputs, MidiOutputRegistry};
use crate::platform;
use crate::types::{AudioEvent, TrackId};

use super::thread::{HandlerCtx, HandlerState};

/// Engine-thread-local map of external-instrument configs, one per track.
pub type ExternalInstruments = HashMap<TrackId, ExternalInstrument>;

/// Store or replace the external-instrument config for its track, marking the
/// track as an external instrument, then echo the stored config back via
/// `ExternalInstrumentChanged`.
pub fn set_external_instrument_in_place(
    instruments: &mut ExternalInstruments,
    event_tx: &Sender<AudioEvent>,
    config: ExternalInstrument,
) {
    instruments.insert(config.track_id, config);
    let _ = event_tx.send(AudioEvent::ExternalInstrumentChanged { config });
}

/// Remove the external-instrument config for `track_id`, taking the track out
/// of external-instrument mode. Emits `ExternalInstrumentCleared` only when a
/// config was actually present, mirroring the other handlers' "missing lookup
/// ⇒ no event" convention.
pub fn clear_external_instrument_in_place(
    instruments: &mut ExternalInstruments,
    event_tx: &Sender<AudioEvent>,
    track_id: TrackId,
) {
    if instruments.remove(&track_id).is_some() {
        let _ = event_tx.send(AudioEvent::ExternalInstrumentCleared { track_id });
    }
}

/// Update the manual latency offset (samples) for `track_id` and echo the
/// updated config via `ExternalInstrumentChanged`. No-op (no event) when the
/// track is not an external instrument.
pub fn set_external_instrument_latency_in_place(
    instruments: &mut ExternalInstruments,
    event_tx: &Sender<AudioEvent>,
    track_id: TrackId,
    latency_offset_samples: i64,
) {
    if let Some(config) = instruments.get_mut(&track_id) {
        config.latency_offset_samples = latency_offset_samples;
        let config = *config;
        let _ = event_tx.send(AudioEvent::ExternalInstrumentChanged { config });
    }
}

/// Update the selected bank/program for `track_id`, echo the updated config via
/// `ExternalInstrumentChanged`, and fire the patch send (Bank Select + Program
/// Change) on `channel` to the track's MIDI output device.
///
/// When the patch cannot reach a live connection — the device is unassigned or
/// offline — an `ExternalInstrumentMidiOutOffline` event is emitted for
/// `midi_out_device` while the config stays intact (recoverable; a replug
/// reconnects). No-op (no event at all) when the track is not an external
/// instrument.
#[allow(clippy::too_many_arguments)]
pub fn set_external_instrument_patch_in_place(
    instruments: &mut ExternalInstruments,
    event_tx: &Sender<AudioEvent>,
    midi_outputs: &mut MidiOutputRegistry,
    track_id: TrackId,
    channel: u8,
    midi_out_device: Option<String>,
    bank: Option<u16>,
    program: Option<u8>,
) {
    let Some(config) = instruments.get_mut(&track_id) else {
        return;
    };
    config.bank = bank;
    config.program = program;
    let config = *config;
    let _ = event_tx.send(AudioEvent::ExternalInstrumentChanged { config });

    let reached = midi_outputs.send_program_change(track_id, channel, bank, program);
    if !reached {
        let _ = event_tx.send(AudioEvent::ExternalInstrumentMidiOutOffline {
            track_id,
            device: midi_out_device,
        });
    }
}

/// Re-check both endpoints of an external-instrument track against the
/// currently-available devices and report any that are offline. Emits
/// `ExternalInstrumentMidiOutOffline` / `ExternalInstrumentReturnInputOffline`
/// for a configured endpoint whose device is no longer present; an endpoint
/// left unset (`None`) or still present is silent. The config is never
/// modified — offline is recoverable and the route is preserved. No-op when the
/// track is not an external instrument.
#[allow(clippy::too_many_arguments)]
pub fn check_external_instrument_devices_in_place(
    instruments: &ExternalInstruments,
    event_tx: &Sender<AudioEvent>,
    track_id: TrackId,
    midi_out_device: Option<&str>,
    return_input_device: Option<&str>,
    available_midi_outputs: &HashSet<String>,
    available_inputs: &HashSet<String>,
) {
    if !instruments.contains_key(&track_id) {
        return;
    }
    if let Some(dev) = midi_out_device {
        if !available_midi_outputs.contains(dev) {
            let _ = event_tx.send(AudioEvent::ExternalInstrumentMidiOutOffline {
                track_id,
                device: Some(dev.to_string()),
            });
        }
    }
    if let Some(dev) = return_input_device {
        if !available_inputs.contains(dev) {
            let _ = event_tx.send(AudioEvent::ExternalInstrumentReturnInputOffline {
                track_id,
                device: Some(dev.to_string()),
            });
        }
    }
}

/// Dispatch glue for `AudioCommand::SetExternalInstrumentPatch`: read the
/// track's MIDI output channel + device, then update the config and fire the
/// patch send via [`set_external_instrument_patch_in_place`].
pub(crate) fn handle_set_patch(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    bank: Option<u16>,
    program: Option<u8>,
) {
    let (channel, device) = {
        let tracks = ctx.tracks.read();
        match tracks.get(&track_id) {
            Some(t) => (
                t.midi_output_channel.unwrap_or(0),
                t.midi_output_device.load_full().map(|n| (*n).clone()),
            ),
            None => (0, None),
        }
    };
    set_external_instrument_patch_in_place(
        &mut state.external_instruments,
        ctx.event_tx,
        &mut state.midi_hw.midi_outputs,
        track_id,
        channel,
        device,
        bank,
        program,
    );
}

/// Dispatch glue for `AudioCommand::CheckExternalInstrumentDevices`: read the
/// track's configured MIDI output + audio-return device names, enumerate the
/// currently-available hardware, and report any endpoint that has gone offline
/// via [`check_external_instrument_devices_in_place`].
pub(crate) fn handle_check_devices(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
) {
    if !state.external_instruments.contains_key(&track_id) {
        return;
    }
    let (midi_out_device, return_input_device) = {
        let tracks = ctx.tracks.read();
        match tracks.get(&track_id) {
            Some(t) => (
                t.midi_output_device.load_full().map(|n| (*n).clone()),
                t.input_device_name.load_full().map(|n| (*n).clone()),
            ),
            None => (None, None),
        }
    };
    let available_midi_outputs: HashSet<String> =
        enumerate_midi_outputs().into_iter().map(|d| d.name).collect();
    let (inputs, _default) = platform::enumerate_input_devices();
    let available_inputs: HashSet<String> = inputs.into_iter().map(|d| d.name).collect();

    check_external_instrument_devices_in_place(
        &state.external_instruments,
        ctx.event_tx,
        track_id,
        midi_out_device.as_deref(),
        return_input_device.as_deref(),
        &available_midi_outputs,
        &available_inputs,
    );
}
