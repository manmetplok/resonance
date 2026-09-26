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

/// Switch a track's external-instrument playback source between `Live`
/// and `Recorded` (doc #257) and echo the applied value back to the app.
///
/// Pure in-place core of `AudioCommand::SetTrackPlaybackSource`, split
/// out (like the `external_instrument.rs` `*_in_place` handlers) so the
/// command/event boundary is testable without spinning up the engine
/// thread. Follows the "missing lookup ⇒ no event" convention: an
/// unknown track changes nothing and echoes nothing, so the app mirror
/// never records a mode the engine didn't apply.
pub fn set_track_playback_source_in_place(
    tracks: &indexmap::IndexMap<TrackId, Track>,
    event_tx: &crossbeam_channel::Sender<AudioEvent>,
    track_id: TrackId,
    source: resonance_common::PlaybackSource,
) {
    let Some(track) = tracks.get(&track_id) else {
        return;
    };
    track.set_playback_source(source);
    let _ = event_tx.send(AudioEvent::TrackPlaybackSourceChanged { track_id, source });
}

pub(crate) fn handle_set_track_playback_source(
    ctx: &HandlerCtx,
    track_id: TrackId,
    source: resonance_common::PlaybackSource,
) {
    set_track_playback_source_in_place(&ctx.tracks.read(), ctx.event_tx, track_id, source);
}

pub(crate) fn handle_set_track_fx_bypass(ctx: &HandlerCtx, track_id: TrackId, bypassed: bool) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        super::plugins::apply_bypass_request(ctx.shared, track.fx_bypass(), bypassed);
    }
    let _ = ctx
        .event_tx
        .send(AudioEvent::TrackFxBypassChanged { track_id, bypassed });
}

/// Attach or detach a track's decoded freeze-cache buffer.
///
/// `Some(source)` makes the track frozen — the mixer can replay the cached
/// audio instead of the live instrument + FX chain. `None` detaches it,
/// restoring live playback. Used on project load to rehydrate frozen tracks
/// without re-rendering, and by `UnfreezeTrack` to clear the cache. The
/// field is an `ArcSwapOption`, so a read lock on `tracks` is enough — the
/// audio thread reads it wait-free.
pub(crate) fn handle_set_track_frozen_source(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    source: Option<FrozenSource>,
) {
    // A newer command for this track supersedes a conversion in flight.
    state.frozen_conversions.retain(|p| p.track_id != track_id);
    // A cache rendered at another rate (a project frozen at 44.1 kHz,
    // opened at 48 kHz) is converted to the engine rate so the audio
    // thread reads it frame for frame (code review FU-G3a) — on a worker,
    // not here: it costs ~2.7 ms per audio second, and a project load
    // sends one per frozen track (FU-A4c). Until it lands the track has
    // no cache (never the previous one, which may be other content), and
    // `settle_frozen_conversions` attaches it. Accepted cost (FU-A5c): for
    // those ~100s of ms a live-playing frozen track renders through its
    // live chain instead — audible only if the transport is already
    // rolling, and offline renders wait for the conversion first.
    let source = match source {
        Some(s) if s.sample_rate != ctx.sample_rate && s.sample_rate != 0 => {
            let rate = ctx.sample_rate;
            let generation = state.clear_generation.load(Ordering::SeqCst);
            match std::thread::Builder::new()
                .name("freeze-cache-resample".into())
                .spawn(move || s.at_rate(rate))
            {
                Ok(handle) => {
                    state.frozen_conversions.push(PendingFrozenConversion {
                        track_id,
                        generation,
                        handle,
                    });
                    None
                }
                // The source went with the closure; nothing to attach.
                Err(e) => {
                    let _ = ctx.event_tx.send(AudioEvent::Error(format!(
                        "Could not convert track {track_id}'s freeze cache to the engine \
                         sample rate ({e}); it plays unfrozen."
                    )));
                    None
                }
            }
        }
        other => other,
    };
    publish_frozen_source(ctx, track_id, source.map(Arc::new));
}

/// Publish `source` as `track_id`'s frozen cache. A replaced cache can be
/// tens of MB; it is retired, not dropped here, so the callback's
/// block-long load can never be its last owner (code review MIX-04).
fn publish_frozen_source(ctx: &HandlerCtx, track_id: TrackId, source: Option<Arc<FrozenSource>>) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        super::retire::publish_opt(&track.frozen_source, source, &ctx.shared.retired);
    }
}

/// A freeze cache being converted to the engine rate on a worker
/// (FU-A4c).
pub(crate) struct PendingFrozenConversion {
    track_id: TrackId,
    /// `HandlerState::clear_generation` when it started: a project cleared
    /// since then (track ids are reused) discards the result.
    generation: u64,
    handle: std::thread::JoinHandle<FrozenSource>,
}

/// Attach finished freeze-cache conversions to their tracks. With `wait`,
/// join the ones still running first — every offline render does, so a
/// bounce right after a project load renders the frozen tracks from
/// their caches; the engine loop polls without waiting.
pub(crate) fn settle_frozen_conversions(ctx: &HandlerCtx, state: &mut HandlerState, wait: bool) {
    if state.frozen_conversions.is_empty() {
        return;
    }
    let generation = state.clear_generation.load(Ordering::SeqCst);
    let pending = std::mem::take(&mut state.frozen_conversions);
    for p in pending {
        if !wait && !p.handle.is_finished() {
            state.frozen_conversions.push(p);
            continue;
        }
        match p.handle.join() {
            Ok(source) if p.generation == generation => {
                publish_frozen_source(ctx, p.track_id, Some(Arc::new(source)));
            }
            Ok(_) => {}
            Err(_) => {
                let _ = ctx.event_tx.send(AudioEvent::Error(format!(
                    "Could not convert track {}'s freeze cache to the engine sample rate; \
                     it plays unfrozen.",
                    p.track_id
                )));
            }
        }
    }
}

/// Take a track out of the live set for good: its published snapshots
/// (frozen cache, plugin chain) go to the retire queue first, because a
/// callback block may still hold a load of either, and dropping the
/// `Track` — and with it the `ArcSwap`'s own reference — would otherwise
/// hand that block the last owner (code review MIX-04).
pub(crate) fn retire_removed_track(track: Track, retired: &super::Retired) {
    retired.retire_opt(track.frozen_source.swap(None));
    retired.retire(track.clear_plugins());
    drop(track);
}

/// Detach a track's frozen source so playback resumes through the live
/// instrument + FX chain. Equivalent to `SetTrackFrozenSource { source: None }`,
/// kept as a distinct command so the intent reads clearly at the call site.
pub(crate) fn handle_unfreeze_track(ctx: &HandlerCtx, state: &mut HandlerState, track_id: TrackId) {
    state.frozen_conversions.retain(|p| p.track_id != track_id);
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        super::retire::publish_opt(&track.frozen_source, None, &ctx.shared.retired);
    }
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
    // A hint below the app's base is an engine-allocated id coming back
    // on the project-load replay, and the counter has to move past it.
    // One at or above the base is app-owned (`SUB_TRACK_ID_BASE`): the
    // app skips ids it holds, so the engine must never start handing
    // out ids in that range itself.
    if id_hint.is_some() && id < SUB_TRACK_ID_BASE {
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
///
/// Bumping `next_track_id` past `sub_id` is critical: the caller picks
/// `sub_id` from an app-side counter that doesn't know about the
/// engine's `next_track_id`. If we don't bump, a later
/// `AddTrack { id_hint: None }` can allocate an id that already exists
/// — the `insert` call here silently overwrites the sub-track (or vice
/// versa) and the GUI's `TrackAdded` handler discards the event as a
/// duplicate, leaving the user with nothing happening on their `+`
/// click. This is the same shape as the `id_hint` branch in
/// `handle_add_track`.
pub(crate) fn handle_create_sub_track(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    sub_id: TrackId,
    parent_track_id: TrackId,
    output_port_index: u32,
    name: String,
) {
    // Bump `next_track_id` past `sub_id` even on the no-op path so an
    // idempotent replay still leaves the counter in the right place —
    // but only for an id below the app's base (see `handle_add_track`);
    // sub-tracks normally live in the app-owned range.
    if sub_id < SUB_TRACK_ID_BASE {
        state.next_track_id = state.next_track_id.max(sub_id + 1);
    }
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
            .map(|t| t.plugin_chain_snapshot());
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
    let (removed_sub_ids, removed_tracks): (Vec<TrackId>, Vec<Track>) = {
        let mut tracks_guard = ctx.tracks.write();
        let mut removed = Vec::new();
        removed.extend(tracks_guard.shift_remove(&track_id));
        let sub_ids: Vec<TrackId> = tracks_guard
            .values()
            .filter(|t| matches!(t.sub_track_of, Some((p, _)) if p == track_id))
            .map(|t| t.id)
            .collect();
        for sid in &sub_ids {
            removed.extend(tracks_guard.shift_remove(sid));
        }
        (sub_ids, removed)
    };
    for track in removed_tracks {
        retire_removed_track(track, &ctx.shared.retired);
    }
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
    state.midi_hw.midi_inputs.remove_track(track_id);
    state.midi_hw.midi_outputs.remove_track(track_id);
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

pub(crate) fn handle_set_track_mono(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    mono: bool,
) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        track.set_mono(mono);
    }
    // Mono ↔ stereo flips the channel count needed (`port + 1` vs
    // `port + 2`); resync in case the monitor stream is now too
    // narrow.
    sync_input_stream(ctx, state);
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
    sync_input_stream(ctx, state);
}

pub(crate) fn handle_set_track_input_device(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    device_name: Option<String>,
) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        track
            .input_device_name
            .store(device_name.map(std::sync::Arc::new));
    }
    sync_input_stream(ctx, state);
}

pub(crate) fn handle_set_track_input_port(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    port_index: u16,
) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        track.set_input_port(port_index);
    }
    sync_input_stream(ctx, state);
}

/// Reconcile the live input stream with whatever the tracks now want.
/// Called from every handler that can change a track field the input
/// stream depends on (monitor toggle, input device, input port, mono
/// toggle). Opens / rebuilds / closes as needed:
///
/// - If any track is monitor-enabled or record-armed and we don't have
///   a stream, open one.
/// - If the existing stream's channel count is below what the
///   currently-active tracks need (e.g. user switched from In 1/2 to
///   In 3/4 after monitoring was on), drop and reopen.
/// - If neither monitoring nor recording is active, drop the stream.
///
/// Recording-driven rebuilds happen in `transport::begin_recording_stream`
/// instead — that path knows the start_sample and allocates per-track
/// WAV writers, neither of which the monitor path does.
fn sync_input_stream(ctx: &HandlerCtx, state: &mut HandlerState) {
    let any_monitoring = ctx.tracks.read().values().any(|t| t.monitor_enabled());
    ctx.shared
        .monitoring
        .store(any_monitoring, Ordering::SeqCst);
    let recording = ctx.shared.recording.load(Ordering::SeqCst);

    if !any_monitoring && !recording {
        if state.rec.input_stream.is_some() {
            state.rec.input_stream = None;
            ctx.shared.input_channels.store(0, Ordering::Release);
        }
        return;
    }

    if recording {
        // Don't disturb an in-flight recording — its stream was sized
        // for the armed tracks at record-start and has WAV writers
        // pinned to its sample rate. begin_recording_stream is the
        // only path that may rebuild while recording.
        return;
    }

    // Pure-monitor path: figure out what's needed and rebuild only if
    // the existing stream can't deliver enough channels. The source
    // device follows whichever monitor-enabled track was found first
    // (the mixer UX scopes monitoring to one source at a time).
    let (source_name, desired_channels) = {
        let tg = ctx.tracks.read();
        let source = tg
            .values()
            .find(|t| t.monitor_enabled())
            .and_then(|t| t.input_device_name.load_full().map(|a| (*a).clone()));
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

    let needs_rebuild = state.rec.input_stream.is_none()
        || state.rec.input_channels < desired_channels;
    if !needs_rebuild {
        return;
    }
    // Drop the existing stream first so PipeWire releases the source
    // before we open a new connection — otherwise the second open
    // might race the teardown.
    state.rec.input_stream = None;
    match platform::build_input_stream(
        source_name.as_deref(),
        Arc::clone(ctx.shared),
        None,
        Arc::clone(ctx.monitor_prod),
        ctx.buf_frames,
        ctx.quantum,
        ctx.sample_rate,
        desired_channels,
        None,
    ) {
        Ok((stream, in_sr, in_ch)) => {
            state.rec.input_stream = Some(stream);
            state.rec.input_sample_rate = in_sr;
            state.rec.input_channels = in_ch;
            ctx.shared.input_channels.store(in_ch, Ordering::Release);
        }
        Err(e) => {
            let _ = ctx.event_tx.send(AudioEvent::Error(format!(
                "Failed to open input stream: {}",
                e
            )));
        }
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

    // Clear tracks -- drain under the lock, retire + drop outside it.
    let removed_tracks: Vec<Track> = ctx.tracks.write().drain(..).map(|(_, t)| t).collect();
    for track in removed_tracks {
        retire_removed_track(track, &ctx.shared.retired);
    }

    // Clear busses
    ctx.busses.write().clear();

    // Clear aux sends (and publish the now-empty table to the render path)
    state.aux_sends.clear();
    super::busses::publish_aux_sends(ctx, state);

    // Clear master FX chain
    ctx.master.write().plugin_ids.clear();
    // A cleared project has no audio to click: land the bypass on
    // "engaged" outright rather than fading there.
    ctx.shared.master_fx_bypass.set_bypassed_settled(false);

    // Fence queued imports first: a worker that has not yet taken the clip
    // lock sees the new generation and drops its result (UPD-09).
    state
        .clear_generation
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    // Clear clips -- collect to drop outside lock
    let removed_clips: Vec<_> = ctx.clips.write().drain(..).collect();
    drop(removed_clips);

    // Clear MIDI clips
    ctx.midi_clips.write().clear();

    // Clear bundles
    state.bundles.clear();

    // Drop loaded A/B references and reset their controls, then publish so
    // the audio-thread monitor stops reading the dropped reference's PCM.
    // (References are monitor-only and never in any render, but a stale one
    // would otherwise linger across a project load.)
    state.reference.clear();
    state.reference.publish(ctx.shared, true);

    // Drop cycle-record take lanes and publish the now-empty comp table
    // (epic #15, ba todo #1394). `wipe_registry` on the app side has always
    // claimed `ClearAll` does this; it did not, so a project loaded on top
    // of one with take lanes kept the old comp governing clip ids the new
    // project reuses — those clips vanished from the ordinary clip path and
    // the stale comp played in their place. A project that *does* carry
    // take lanes then replaces this via `RestoreTakeGroups`; one that
    // doesn't (and File > New) is left correctly empty.
    // The parked recordings of removed takes go with them (ba todo #1397).
    // The clip list has just been drained, so nothing un-parks into a
    // project that never had them; keeping the park across a load would
    // only hold the previous project's WAV mappings open.
    state.take_groups.clear();
    state.take_clip_park.clear();
    super::takes::publish_take_comp(ctx, state);

    // Reset ID counters — except clips: a clip id names its
    // `audio/clip_{id}.wav`, and a slow-path undo's redo stack (or a
    // backup) can still reference an id cleared here. Reissuing it let a
    // new take overwrite that WAV (code review STATE-08); clip ids stay
    // monotonic for the session.
    state.next_track_id = 1;
    state.next_bus_id = 1;
    state.next_plugin_id = 1;
    state.next_send_id = 1;
    state.next_take_group_id = 1;

    let _ = ctx.event_tx.send(AudioEvent::AllCleared);
}
