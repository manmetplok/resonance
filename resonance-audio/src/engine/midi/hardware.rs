//! Hardware MIDI device enumeration and per-track input/output binding.
//! Owns the side-effects on [`MidiInputRegistry`] / [`MidiOutputRegistry`]
//! and mirrors the chosen device + channel onto the engine-side track.
//!
//! The audio thread reads `Track::midi_output_device` lock-free via
//! arc-swap (shared by every copy of the track) so the swap done here is
//! visible to the next mix block immediately; the engine-thread-only
//! bindings (input device / channels) are a copy-on-write track edit
//! published in a new render graph.

use std::sync::Arc;

use crossbeam_channel::Sender;
use resonance_common::DeviceParam;

use crate::midi_hardware::{enumerate_midi_inputs, enumerate_midi_outputs};
use crate::types::*;

use super::super::thread::{HandlerCtx, HandlerState};

pub(crate) fn handle_list_midi_inputs(ctx: &HandlerCtx, state: &mut HandlerState) {
    let devices = enumerate_midi_inputs();
    // Always reconcile: a fresh connect attempt for a pending track
    // is cheap and the only way "unplug, replug" recovers without
    // user intervention. The unchanged-list dedupe below only
    // suppresses the GUI round-trip, not the reconnect attempt.
    state.midi_hw.midi_inputs.reconcile();
    // Same reconcile pass for the dedicated control-surface port so a
    // re-plugged surface reconnects without the user re-picking it.
    state.midi_hw.control_surface.reconcile();
    if devices != state.midi_hw.last_midi_input_devices {
        state.midi_hw.last_midi_input_devices = devices.clone();
        // The control-surface picker offers the same ports.
        crate::engine::midi_map::report_control_surface_devices(
            ctx,
            devices.iter().map(|d| d.name.clone()).collect(),
        );
        let _ = ctx
            .event_tx
            .send(AudioEvent::MidiInputDevicesListed { devices });
    }
}

pub(crate) fn handle_list_midi_outputs(ctx: &HandlerCtx, state: &mut HandlerState) {
    let devices = enumerate_midi_outputs();
    if devices != state.midi_hw.last_midi_output_devices {
        state.midi_hw.last_midi_output_devices = devices.clone();
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
    // path. Structural (only the engine thread reads it): a
    // copy-on-write edit of the track, published in a new render graph.
    ctx.shared.edit_track(track_id, |t| {
        t.midi_input_device = device.clone();
        t.midi_input_channel = channel;
    });
    if let Err(e) = state
        .midi_hw
        .midi_inputs
        .set_track_input(track_id, device, channel)
    {
        let _ = ctx.event_tx.send(AudioEvent::Error(e.into()));
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
    // `midi_output_device` via arc-swap (shared by every copy of the
    // track), so the swap is visible to the next mix block immediately;
    // the channel (engine-thread only) is a copy-on-write edit published
    // in a new render graph.
    ctx.shared.edit_track(track_id, |t| {
        match &device {
            Some(name) => t.midi_output_device.store(Some(Arc::new(name.clone()))),
            None => t.midi_output_device.store(None),
        }
        t.midi_output_channel = channel;
    });
    if let Err(e) = state.midi_hw.midi_outputs.set_track_output(track_id, device) {
        let _ = ctx.event_tx.send(AudioEvent::Error(e.into()));
    }
}

pub(crate) fn handle_set_track_device_params(
    ctx: &HandlerCtx,
    track_id: TrackId,
    params: Vec<DeviceParam>,
) {
    set_track_device_params_in_place(&ctx.tracks(), ctx.event_tx, track_id, params);
}

/// Store the device preset's automatable parameters on the engine-side
/// track keyed by [`DeviceParam::id`] (architecture doc #201 §4, epic #40)
/// and confirm with `AudioEvent::TrackDeviceParamsApplied`. Replaces the
/// whole map; an empty `params` clears it. The mutation and the event both
/// live inside the `if let Some(track)` branch, so an unknown track id is a
/// silent no-op that never emits a ghost event (mirroring
/// [`super::super::clips::set_clip_fade_in_place`] and the other per-track
/// setters).
///
/// Reads the published track map only: [`Track::set_device_params`]
/// publishes the new map through an `ArcSwap` store on `&self` (shared by
/// every copy of the track), so the audio thread sees it on the next block
/// without a render-graph publish.
pub fn set_track_device_params_in_place(
    tracks: &TrackMap,
    event_tx: &Sender<AudioEvent>,
    track_id: TrackId,
    params: Vec<DeviceParam>,
) {
    if let Some(track) = tracks.get(&track_id) {
        let param_ids = track.set_device_params(params);
        let _ = event_tx.send(AudioEvent::TrackDeviceParamsApplied { track_id, param_ids });
    }
}
