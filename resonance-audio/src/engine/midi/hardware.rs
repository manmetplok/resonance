//! Hardware MIDI device enumeration and per-track input/output binding.
//! Owns the side-effects on [`MidiInputRegistry`] / [`MidiOutputRegistry`]
//! and mirrors the chosen device + channel onto the engine-side track.
//!
//! The audio thread reads `Track::midi_output_device` lock-free via
//! arc-swap so the swap done here is visible to the next mix block
//! immediately, without waiting for the tracks-map write lock to drop.

use std::sync::Arc;

use crate::midi_hardware::{enumerate_midi_inputs, enumerate_midi_outputs};
use crate::types::*;

use super::super::thread::{HandlerCtx, HandlerState};

pub(crate) fn handle_list_midi_inputs(ctx: &HandlerCtx, state: &mut HandlerState) {
    let devices = enumerate_midi_inputs();
    // Always reconcile: a fresh connect attempt for a pending track
    // is cheap and the only way "unplug, replug" recovers without
    // user intervention. The unchanged-list dedupe below only
    // suppresses the GUI round-trip, not the reconnect attempt.
    state.midi_inputs.reconcile();
    if devices != state.last_midi_input_devices {
        state.last_midi_input_devices = devices.clone();
        let _ = ctx
            .event_tx
            .send(AudioEvent::MidiInputDevicesListed { devices });
    }
}

pub(crate) fn handle_list_midi_outputs(ctx: &HandlerCtx, state: &mut HandlerState) {
    let devices = enumerate_midi_outputs();
    if devices != state.last_midi_output_devices {
        state.last_midi_output_devices = devices.clone();
        let _ = ctx
            .event_tx
            .send(AudioEvent::MidiOutputDevicesListed { devices });
    }
}

pub(crate) fn handle_set_track_midi_input(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    device: Option<String>,
    channel: Option<u8>,
) {
    // Persist the desired config on the engine-side track for
    // subsequent saves and for the registry's reconnect-on-replug
    // path. Plain field write — only the engine thread reads it.
    {
        let mut tracks = ctx.tracks.write();
        if let Some(t) = tracks.get_mut(&track_id) {
            t.midi_input_device = device.clone();
            t.midi_input_channel = channel;
        }
    }
    if let Err(e) = state.midi_inputs.set_track_input(track_id, device, channel) {
        let _ = ctx.event_tx.send(AudioEvent::Error(e));
    }
}

pub(crate) fn handle_set_track_midi_output(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    device: Option<String>,
    channel: Option<u8>,
) {
    // Mirror onto the engine-side track. The audio thread reads
    // `midi_output_device` via arc-swap (no lock), so the swap is
    // visible to the next mix block immediately even though the map
    // itself is held under a write lock for the channel update.
    {
        let mut tracks = ctx.tracks.write();
        if let Some(t) = tracks.get_mut(&track_id) {
            match &device {
                Some(name) => t.midi_output_device.store(Some(Arc::new(name.clone()))),
                None => t.midi_output_device.store(None),
            }
            t.midi_output_channel = channel;
        }
    }
    if let Err(e) = state.midi_outputs.set_track_output(track_id, device) {
        let _ = ctx.event_tx.send(AudioEvent::Error(e));
    }
}
