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
}

pub(super) fn removed(r: &mut Resonance, track_id: TrackId) {
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
    for id in removed_plugin_ids {
        r.plugin_index.remove(&id);
    }
    r.registry.tracks.retain(|t| t.id != track_id);
    r.clips.retain(|c| c.track_id != track_id);
    // Also drop any sub-tracks whose parent just went away.
    r.registry.tracks.retain(|t| {
        t.sub_track
            .map(|l| l.parent_track_id != track_id)
            .unwrap_or(true)
    });
    r.compose.refresh_track_count(&r.registry.tracks);
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
                waveform_peaks: c.waveform_peaks,
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
    }
    r.registry.busses.retain(|b| b.id != bus_id);
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
