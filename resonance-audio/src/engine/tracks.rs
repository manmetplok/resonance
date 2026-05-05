//! Track handlers: add/remove (audio) tracks, sub-tracks, per-track
//! volume/pan/mute/solo/arm/mono/monitor/input routing, master volume,
//! input-device enumeration, and project clear. Instrument-track
//! creation lives in `midi.rs`.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::platform;
use crate::types::*;

use super::thread::{HandlerCtx, HandlerState};

pub(crate) fn handle_set_track_volume(ctx: &HandlerCtx, track_id: TrackId, volume: f32) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        track.set_volume(volume.max(0.0));
    }
}

pub(crate) fn handle_set_track_pan(ctx: &HandlerCtx, track_id: TrackId, pan: f32) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        track.set_pan(pan.clamp(-1.0, 1.0));
    }
}

pub(crate) fn handle_set_track_mute(ctx: &HandlerCtx, track_id: TrackId, muted: bool) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        track.set_muted(muted);
    }
}

pub(crate) fn handle_set_track_fx_bypass(ctx: &HandlerCtx, track_id: TrackId, bypassed: bool) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        track.set_fx_bypassed(bypassed);
    }
    let _ = ctx
        .event_tx
        .send(AudioEvent::TrackFxBypassChanged { track_id, bypassed });
}

pub(crate) fn handle_set_master_volume(ctx: &HandlerCtx, volume: f32) {
    ctx.shared
        .master_volume_bits
        .store(volume.max(0.0).to_bits(), Ordering::Relaxed);
}

pub(crate) fn handle_set_track_solo(ctx: &HandlerCtx, track_id: TrackId, soloed: bool) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        track.set_soloed(soloed);
    }
}

pub(crate) fn handle_add_track(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    id_hint: Option<TrackId>,
    name: Option<String>,
) {
    let id = id_hint.unwrap_or_else(|| {
        let i = state.next_track_id;
        state.next_track_id += 1;
        i
    });
    if id_hint.is_some() {
        state.next_track_id = state.next_track_id.max(id + 1);
    }
    let name = name.unwrap_or_else(|| format!("Track {}", id));
    let track = Track::new(id, name);
    ctx.tracks.write().insert(id, track);
    let _ = ctx.event_tx.send(AudioEvent::TrackAdded { track_id: id });
}

/// Create a sub-track in the engine for one output port of a multi-output
/// plugin. Sub-tracks are a UI-initiated concept: the UI creates them in
/// response to `PluginAdded` events (see `engine_events.rs`). The engine
/// stores them as regular `Track`s with `sub_track_of` set; the mixer
/// reads this field during mixdown to route output ports to the
/// sub-track's own fader/pan/bus chain.
pub(crate) fn handle_create_sub_track(
    ctx: &HandlerCtx,
    sub_id: TrackId,
    parent_track_id: TrackId,
    output_port_index: u32,
    name: String,
) {
    // Idempotent: skip if this sub-track already exists. Project load
    // replays saved sub-tracks, then PluginAdded re-fires the
    // auto-create path; the second hit should be a no-op.
    if ctx.tracks.read().contains_key(&sub_id) {
        return;
    }
    if !ctx.tracks.read().contains_key(&parent_track_id) {
        debug_assert!(
            false,
            "CreateSubTrack: parent track {parent_track_id:?} not found"
        );
        return;
    }
    let track = Track::new_sub_track(sub_id, name, parent_track_id, output_port_index);
    ctx.tracks.write().insert(sub_id, track);
}

pub(crate) fn handle_remove_track(ctx: &HandlerCtx, state: &mut HandlerState, track_id: TrackId) {
    // Remove plugins for this track -- extract under write lock, then
    // drop instances outside the lock so audio callback isn't blocked.
    let removed_plugins: Vec<_> = {
        let plugin_ids = ctx
            .tracks
            .read()
            .get(&track_id)
            .map(|t| t.plugin_ids.clone());
        if let Some(ids) = plugin_ids {
            let mut plugins_guard = ctx.plugins.write();
            ids.iter()
                .filter_map(|pid| plugins_guard.shift_remove(pid))
                .collect()
        } else {
            Vec::new()
        }
    };
    drop(removed_plugins);
    // Drop the parent track and any sub-tracks fed by it in one pass
    // under the same write lock.
    let removed_sub_ids: Vec<TrackId> = {
        let mut tracks_guard = ctx.tracks.write();
        tracks_guard.shift_remove(&track_id);
        let sub_ids: Vec<TrackId> = tracks_guard
            .values()
            .filter(|t| matches!(t.sub_track_of, Some((p, _)) if p == track_id))
            .map(|t| t.id)
            .collect();
        for sid in &sub_ids {
            tracks_guard.shift_remove(sid);
        }
        sub_ids
    };
    // Remove clips -- collect removed clips so dealloc happens outside
    // lock.
    let removed_clips: Vec<_> = {
        let mut clips_guard = ctx.clips.write();
        let mut removed = Vec::new();
        let mut i = 0;
        while i < clips_guard.len() {
            if clips_guard[i].track_id == track_id {
                removed.push(clips_guard.swap_remove(i));
            } else {
                i += 1;
            }
        }
        removed
    };
    drop(removed_clips);
    state.rec.buffers.remove(&track_id);
    state.midi_inputs.remove_track(track_id);
    state.midi_outputs.remove_track(track_id);
    state.midi_recording.remove(&track_id);
    let _ = ctx.event_tx.send(AudioEvent::TrackRemoved { track_id });
    for sid in removed_sub_ids {
        let _ = ctx
            .event_tx
            .send(AudioEvent::TrackRemoved { track_id: sid });
    }
}

pub(crate) fn handle_set_track_record_arm(ctx: &HandlerCtx, track_id: TrackId, armed: bool) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        track.set_record_armed(armed);
    }
}

pub(crate) fn handle_set_track_mono(ctx: &HandlerCtx, track_id: TrackId, mono: bool) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        track.set_mono(mono);
    }
}

pub(crate) fn handle_set_track_monitor(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    enabled: bool,
) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        track.set_monitor_enabled(enabled);
    }
    // Update monitoring flag: true if any track has monitoring enabled.
    let any_monitoring = ctx.tracks.read().values().any(|t| t.monitor_enabled());
    ctx.shared
        .monitoring
        .store(any_monitoring, Ordering::SeqCst);

    if any_monitoring && state.rec.input_stream.is_none() {
        // Start input stream: monitoring on but no stream active.
        // Figure out the source name and the highest input channel any
        // monitoring track needs. Without the channel hint, cpal opens
        // a stereo stream and tracks listening to channels 3+ get
        // clamped to channel 1.
        let (source_name, desired_channels) = {
            let tg = ctx.tracks.read();
            let source = tg
                .values()
                .find(|t| t.monitor_enabled())
                .and_then(|t| t.input_device_name.clone());
            let max_needed: u16 = tg
                .values()
                .filter(|t| t.monitor_enabled())
                .map(|t| {
                    let port = t.input_port();
                    if t.mono() { port + 1 } else { port + 2 }
                })
                .max()
                .unwrap_or(2)
                .max(2);
            (source, max_needed)
        };
        match platform::build_input_stream(
            source_name.as_deref(),
            Arc::clone(ctx.shared),
            None,
            Arc::clone(ctx.monitor_prod),
            ctx.buf_frames,
            ctx.quantum,
            ctx.sample_rate,
            desired_channels,
        ) {
            Ok((stream, in_sr, in_ch)) => {
                state.rec.input_stream = Some(stream);
                state.rec.input_sample_rate = in_sr;
                state.rec.input_channels = in_ch;
                ctx.shared.input_channels.store(in_ch, Ordering::Release);
            }
            Err(e) => {
                let _ = ctx.event_tx.send(AudioEvent::Error(format!(
                    "Failed to start monitoring: {}",
                    e
                )));
            }
        }
    } else if !any_monitoring && !ctx.shared.recording.load(Ordering::SeqCst) {
        // Stop input stream if no monitoring and not recording.
        state.rec.input_stream = None;
        ctx.shared.input_channels.store(0, Ordering::Release);
    }
}

pub(crate) fn handle_set_track_input_device(
    ctx: &HandlerCtx,
    track_id: TrackId,
    device_name: Option<String>,
) {
    if let Some(track) = ctx.tracks.write().get_mut(&track_id) {
        track.input_device_name = device_name;
    }
}

pub(crate) fn handle_set_track_input_port(ctx: &HandlerCtx, track_id: TrackId, port_index: u16) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        track.set_input_port(port_index);
    }
}

pub(crate) fn handle_list_input_devices(ctx: &HandlerCtx) {
    let (devices, default_name) = platform::enumerate_input_devices();
    let _ = ctx.event_tx.send(AudioEvent::InputDevicesListed {
        devices,
        default_name,
    });
}

pub(crate) fn handle_clear_all(ctx: &HandlerCtx, state: &mut HandlerState) {
    // Stop playback/recording
    ctx.shared.playing.store(false, Ordering::SeqCst);
    ctx.shared.recording.store(false, Ordering::SeqCst);
    ctx.shared.playhead.store(0, Ordering::SeqCst);
    state.rec.input_stream = None;
    state.rec.buffers.clear();

    // Drop all plugin instances outside the write lock
    {
        let mut plugins_guard = ctx.plugins.write();
        let removed: Vec<_> = plugins_guard.drain(..).collect();
        drop(plugins_guard);
        drop(removed);
    }

    // Clear tracks
    ctx.tracks.write().clear();

    // Clear busses
    ctx.busses.write().clear();

    // Clear master FX chain
    ctx.master.write().plugin_ids.clear();
    ctx.shared
        .master_fx_bypassed
        .store(false, Ordering::Relaxed);

    // Clear clips -- collect to drop outside lock
    let removed_clips: Vec<_> = ctx.clips.write().drain(..).collect();
    drop(removed_clips);

    // Clear MIDI clips
    ctx.midi_clips.write().clear();

    // Clear bundles
    state.bundles.clear();

    // Reset ID counters
    state.next_track_id = 1;
    state.next_bus_id = 1;
    state.next_clip_id = 1;
    state.next_plugin_id = 1;

    let _ = ctx.event_tx.send(AudioEvent::AllCleared);
}
