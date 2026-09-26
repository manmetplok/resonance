//! App-side handlers for track / bus lifecycle and bounce events.

use resonance_audio::types::*;

use crate::state::*;
use crate::Resonance;

/// Handle the silent-drop case (`TrackAdded` for an id already present in
/// the registry). Project load replays saved tracks app-side first, then
/// hands them to the engine which echoes `TrackAdded` back — that path
/// legitimately hits the guard. But if the engine's id allocator hands
/// out a colliding id (historical bug: `handle_create_sub_track` not
/// bumping `next_track_id`), we'd silently drop a user `+` click *and*
/// leave `pending_track_preset` armed for the next add to inherit. Clear
/// the slot here so a recovery click doesn't apply the wrong preset.
fn drop_duplicate_track_added(r: &mut Resonance, track_id: TrackId) {
    if r.pending_track_preset.is_some() {
        eprintln!(
            "engine_events::tracks: dropped TrackAdded for id {} — id already in registry; \
             clearing pending preset to avoid leaking to next add",
            track_id
        );
        r.pending_track_preset = None;
        r.pending_preset_plugin_states = None;
    }
}

pub(super) fn added(r: &mut Resonance, track_id: TrackId) {
    // Idempotent: skip if the track already exists (created by project load).
    if r.registry.tracks.iter().any(|t| t.id == track_id) {
        drop_duplicate_track_added(r, track_id);
        return;
    }
    let order = r.registry.next_track_order;
    r.registry.next_track_order += 1;
    let mut track = TrackState::new_audio(track_id, order);
    if let Some(preset) = r.pending_track_preset.take() {
        super::presets::apply_preset_to_track(r, &mut track, &preset);
    }
    r.registry.tracks.push(track);
    r.compose.refresh_track_count(&r.registry.tracks);
    r.apply_pending_control_track(track_id);
}

pub(super) fn instrument_added(r: &mut Resonance, track_id: TrackId) {
    if r.registry.tracks.iter().any(|t| t.id == track_id) {
        drop_duplicate_track_added(r, track_id);
        return;
    }
    let order = r.registry.next_track_order;
    r.registry.next_track_order += 1;
    let mut track = TrackState::new_instrument(track_id, order);
    if let Some(preset) = r.pending_track_preset.take() {
        super::presets::apply_preset_to_track(r, &mut track, &preset);
    }
    r.registry.tracks.push(track);
    r.compose.refresh_track_count(&r.registry.tracks);
    // A control-endpoint `track.add` (todo #1152) may have deferred
    // this track's name / drum type to the mirror — apply it now.
    r.apply_pending_control_track(track_id);
}

pub(super) fn vocal_added(r: &mut Resonance, track_id: TrackId) {
    if r.registry.tracks.iter().any(|t| t.id == track_id) {
        drop_duplicate_track_added(r, track_id);
        return;
    }
    let order = r.registry.next_track_order;
    r.registry.next_track_order += 1;
    let mut track = TrackState::new_vocal(track_id, order);
    if let Some(preset) = r.pending_track_preset.take() {
        super::presets::apply_preset_to_track(r, &mut track, &preset);
    }
    r.registry.tracks.push(track);
    r.compose.refresh_track_count(&r.registry.tracks);
    r.apply_pending_control_track(track_id);
}

pub(super) fn removed(r: &mut Resonance, track_id: TrackId) {
    // Aux sends leaving this track die with it (ba todo #1269 review).
    // The engine does NOT prune its own table on RemoveTrack, so tell it
    // explicitly -- otherwise it keeps rendering-and-cycle-checking an
    // edge that no view can show and no command can remove.
    for send_id in r.aux.drop_sends_touching_track(track_id) {
        let _ = r.engine.send(AudioCommand::RemoveAuxSend { send_id });
    }
    // Key routes sourced from this track die with it too (ba todo #1311).
    // No engine command here, unlike the sends above: the engine's
    // `RemoveTrack` arm calls `drop_source_routes` itself, so this only
    // has to keep the mirror — and therefore the next save — honest.
    r.sidechain
        .drop_routes_from_source(resonance_audio::types::SendSource::Track(track_id));
    if let Some(sel_clip_id) = r.interaction.selected_clip {
        if r.clips
            .iter()
            .any(|c| c.id == sel_clip_id && c.track_id == track_id)
        {
            r.interaction.selected_clip = None;
        }
    }
    if let Some(sel_plugin_id) = r.mixer.selected_plugin {
        if r.registry
            .tracks
            .iter()
            .filter(|t| t.id == track_id)
            .any(|t| t.plugins.iter().any(|p| p.instance_id == sel_plugin_id))
        {
            r.mixer.selected_plugin = None;
        }
    }
    // Drop side-index entries for every plugin on the removed track
    // and on any sub-track that's about to be removed too. Collect ids
    // first to avoid a borrow-collision between `registry.tracks` and
    // `plugin_index` (both fields of `r`).
    let removed_plugin_ids: Vec<resonance_audio::types::PluginInstanceId> = r
        .registry
        .tracks
        .iter()
        .filter(|t| {
            t.id == track_id
                || t.sub_track.map(|l| l.parent_track_id == track_id).unwrap_or(false)
        })
        .flat_map(|t| t.plugins.iter().map(|p| p.instance_id))
        .collect();
    drop_track_references(r, track_id, &removed_plugin_ids);
    for id in removed_plugin_ids {
        r.plugin_index.remove(&id);
        // …and any key route pointing AT one of them: a track deletion
        // takes its whole chain without a per-plugin `PluginRemoved`
        // echo (ba todo #1311).
        r.sidechain.clear_plugin(id);
    }
    r.registry.tracks.retain(|t| t.id != track_id);
    r.clips.retain(|c| c.track_id != track_id);
    r.recompute_pool_usage(); // review VIEW-30
    // Also drop any sub-tracks whose parent just went away.
    r.registry.tracks.retain(|t| {
        t.sub_track
            .map(|l| l.parent_track_id != track_id)
            .unwrap_or(true)
    });
    r.compose.refresh_track_count(&r.registry.tracks);
    crate::update::compose::forget_track(r, track_id);
}

/// Drop everything besides the track row that names a removed track (code
/// review STATE-05): its MIDI clips, the automation lanes aimed at it or at
/// its plugins, its group memberships, its external-instrument config and
/// any freeze status. All of these are saved, and after a reload the
/// engine hands the deleted id to the next new track, which would inherit
/// them. Sub-tracks get their own `TrackRemoved`, so only `track_id` is
/// handled here; `plugin_ids` covers the whole removed chain.
fn drop_track_references(
    r: &mut Resonance,
    track_id: TrackId,
    plugin_ids: &[resonance_audio::types::PluginInstanceId],
) {
    use resonance_common::AutomationTarget as T;

    // The engine's `RemoveTrack` keeps MIDI clips, so delete them there
    // too; the `MidiClipDeleted` echo then finds nothing left to drop.
    let midi_ids: Vec<ClipId> = r
        .midi_clips
        .iter()
        .filter(|c| c.track_id == track_id)
        .map(|c| c.id)
        .collect();
    for clip_id in &midi_ids {
        let _ = r.engine.send(AudioCommand::DeleteMidiClip { clip_id: *clip_id });
        r.compose.vocal_audio.clip_lyrics.remove(clip_id);
    }
    r.midi_clips.retain(|c| c.track_id != track_id);
    r.compose
        .derived_clips
        .retain(|&(_, _, t), clip_id| t != track_id && !midi_ids.contains(clip_id));

    let stale_lanes: Vec<T> = r
        .automation
        .lanes
        .keys()
        .filter(|t| match t {
            T::TrackGain(id) | T::TrackPan(id) | T::TrackMute(id) => *id == track_id,
            T::DeviceParam { track, .. } => *track == track_id,
            T::PluginParam { instance, .. } => plugin_ids.contains(instance),
            _ => false,
        })
        .cloned()
        .collect();
    for target in stale_lanes {
        r.automation.lanes.remove(&target);
        r.automation.live_values.remove(&target);
        let _ = r.engine.send(AudioCommand::ClearAutomationLane { target });
    }

    for group in r.track_groups.get_all_groups_mut() {
        group.ordered_members.retain(|&m| m != track_id);
    }

    if r.external_instruments.remove(&track_id).is_some() {
        let _ = r
            .engine
            .send(AudioCommand::ClearExternalInstrument { track_id });
    }
    r.cleanup_freeze_on_delete(track_id);
    r.freeze.clear(track_id);
}

pub(super) fn bounce_completed(
    r: &mut Resonance,
    source_track_id: TrackId,
    target_track_id: TrackId,
    clip: Option<BouncedClipData>,
) {
    // Drop the progress modal — the run finished one way or another.
    r.bounce_in_progress = None;
    // Offline bounce delivers the clip inline; realtime bounce delivers
    // it via the regular `RecordingFinished` event handled above and
    // leaves `clip` as `None`.
    if let Some(c) = clip {
        if !r.clips.iter().any(|existing| existing.id == c.clip_id) {
            r.clips.push(ClipState {
                id: c.clip_id,
                track_id: target_track_id,
                start_sample: c.start_sample,
                duration_samples: c.duration_samples,
                name: c.name,
                total_frames: c.duration_samples,
                trim_start_frames: 0,
                trim_end_frames: 0,
                fade_in_frames: 0,
                fade_in_curve: FadeCurve::default(),
                fade_out_frames: 0,
                fade_out_curve: FadeCurve::default(),
                gain_db: 0.0,
                waveform_peaks: c.waveform_peaks,
                vocal_tuning: None,
                // A bounced-in-place clip is engine-rendered, not a pool import.
                asset_ref: None,
            });
        }
    } else {
        // Realtime bounce: the clip arrived as `RecordingFinished` with
        // a generic "Recording N" label. Inherit the target track's
        // name so it's obvious in the timeline which bounce belongs to
        // which track. Rename the most recently-added clip on the
        // target — there's only one bounce in flight at a time.
        let track_name = r
            .registry
            .tracks
            .iter()
            .find(|t| t.id == target_track_id)
            .map(|t| t.name.clone());
        if let Some(name) = track_name {
            if let Some(clip) = r
                .clips
                .iter_mut()
                .filter(|c| c.track_id == target_track_id)
                .max_by_key(|c| c.id)
            {
                clip.name = name;
            }
        }
    }
    finalize_bounce(r, source_track_id, target_track_id);
}

/// Shared post-bounce wrap-up: mute the source, send the engine the
/// matching `SetTrackMute`, and reorder the bounce target so it sits
/// right under the source. Called from both the offline
/// (`TrackBouncedToAudio`) and realtime (`TrackBounceCompleted`)
/// completion handlers.
pub(super) fn finalize_bounce(
    r: &mut Resonance,
    source_track_id: TrackId,
    target_track_id: TrackId,
) {
    if let Some(track) = r
        .registry
        .tracks
        .iter_mut()
        .find(|t| t.id == source_track_id)
    {
        track.muted = true;
    }
    let _ = r.engine.send(AudioCommand::SetTrackMute {
        track_id: source_track_id,
        muted: true,
    });
    let source_order = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == source_track_id)
        .map(|t| t.order);
    if let Some(src_order) = source_order {
        for t in r.registry.tracks.iter_mut() {
            if t.id != target_track_id && t.order > src_order {
                t.order += 1;
            }
        }
        if let Some(t) = r
            .registry
            .tracks
            .iter_mut()
            .find(|t| t.id == target_track_id)
        {
            t.order = src_order + 1;
        }
        r.registry.next_track_order += 1;
        r.registry.resort_tracks();
    }
}

pub(super) fn fx_bypass_changed(r: &mut Resonance, track_id: TrackId, bypassed: bool) {
    if let Some(track) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) {
        track.fx_bypassed = bypassed;
    }
}

/// Mirror the engine's echo of `SetTrackPlaybackSource` (doc #257, todo
/// #1100): the track's external-instrument playback source changed —
/// either from the inspector toggle or from the auto-switch after a
/// recorded take lands. Engine-owned state like monitor/arm; the mirror
/// simply follows.
pub(super) fn playback_source_changed(
    r: &mut Resonance,
    track_id: TrackId,
    source: resonance_common::PlaybackSource,
) {
    if let Some(track) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) {
        track.playback_source = source;
    }
}

pub(super) fn bus_added(r: &mut Resonance, bus_id: BusId, name: String) {
    if r.registry.busses.iter().any(|b| b.id == bus_id) {
        return;
    }
    let order = r.registry.next_bus_order;
    r.registry.next_bus_order += 1;
    r.registry.busses.push(BusState::new(bus_id, order, name));
    r.view_caches.rebuild_output(&r.registry.busses);
}

pub(super) fn bus_removed(r: &mut Resonance, bus_id: BusId) {
    // A bus can be either end of a send edge, so drop both directions.
    for send_id in r.aux.drop_sends_touching_bus(bus_id) {
        let _ = r.engine.send(AudioCommand::RemoveAuxSend { send_id });
    }
    // Same for key routes keyed off this bus — see `removed` above.
    r.sidechain
        .drop_routes_from_source(resonance_audio::types::SendSource::Bus(bus_id));
    if let Some(sel) = r.mixer.selected_plugin {
        if r.registry
            .busses
            .iter()
            .filter(|b| b.id == bus_id)
            .any(|b| b.plugins.iter().any(|p| p.instance_id == sel))
        {
            r.mixer.selected_plugin = None;
        }
    }
    let removed_plugin_ids: Vec<resonance_audio::types::PluginInstanceId> = r
        .registry
        .busses
        .iter()
        .find(|b| b.id == bus_id)
        .map(|b| b.plugins.iter().map(|p| p.instance_id).collect())
        .unwrap_or_default();
    for id in removed_plugin_ids {
        r.plugin_index.remove(&id);
        // A bus deletion takes the bus's whole insert chain with it
        // without a per-plugin `BusPluginRemoved` echo, so a key route
        // *onto* one of those plugins has to be pruned here as well as
        // one keyed *off* the bus (ba todo #1311).
        r.sidechain.clear_plugin(id);
    }
    r.registry.busses.retain(|b| b.id != bus_id);
    // Don't leave the inspector pointed at a bus that no longer exists.
    if r.mixer.selected_bus == Some(bus_id) {
        r.mixer.selected_bus = None;
    }
    // Any track that was routed to the removed bus falls back to Master
    // locally (the engine did the same server-side).
    for track in &mut r.registry.tracks {
        if track.output == TrackOutput::Bus(bus_id) {
            track.output = TrackOutput::Master;
        }
    }
    r.view_caches.rebuild_output(&r.registry.busses);
}

pub(super) fn bus_fx_bypass_changed(r: &mut Resonance, bus_id: BusId, bypassed: bool) {
    if let Some(bus) = r.registry.busses.iter_mut().find(|b| b.id == bus_id) {
        bus.fx_bypassed = bypassed;
    }
}
