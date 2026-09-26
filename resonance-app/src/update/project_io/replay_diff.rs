//! Diff-based undo/redo replay — the cheap alternative to the
//! `ClearAll → AllCleared → replay_loaded_project` round-trip.
//!
//! For undo/redo within a single editing session the engine's *shape*
//! almost never changes: the same tracks, busses, plugins, and clips are
//! still there; only their scalar properties have moved. The full replay
//! tears every plugin instance down and re-instantiates it — expensive,
//! audible (the plugin chain is briefly silent), and entirely wasted
//! when the user just dragged a fader.
//!
//! [`try_diff_replay`] computes the structural shape of the current
//! state vs. the target snapshot. When they match, it drives the engine
//! surgically — one engine command per changed scalar — and rebuilds
//! GUI state in place. When the shapes diverge (a track was added or
//! removed, a clip was inserted, a plugin instance changed identity),
//! it returns `false` and the caller falls back to the full clear-and-
//! replay pipeline.
//!
//! Plugin parameter restores: a snapshot's state blob is re-sent with
//! `LoadPluginState` only when the live cache has moved on since the
//! snapshot; either way the snapshot's param values are then driven
//! explicitly (all of them the blob may have reset, or just those that
//! differ from the live mirror when no blob was pushed), so plugin param
//! undo works without a full re-instantiation.

use std::collections::HashMap;

use resonance_audio::types::*;

use crate::project::{
    fade_curve_from_tag, send_source_from_tag, LoadedProject, ProjectBus, ProjectClip, ProjectFile,
    ProjectMidiClip, ProjectPlugin, ProjectSend, ProjectTrack,
};
use crate::undo::UndoExtras;
use crate::util::db_to_gain;
use crate::Resonance;

use super::replay::{restore_drum_patterns, restore_tempo_events};
use super::serialize::build_project_file;

/// Attempt a structure-preserving replay. Returns `true` when the diff
/// path successfully drove engine + GUI to the target state; `false`
/// when the structural shape of the project differs (tracks, busses,
/// plugins, clips, drum groups, master plugins, or sections were
/// added / removed / renumbered) and the caller must fall back to the
/// full clear-and-replay pipeline.
///
/// On success the caller must skip the `ClearAll` command — there is no
/// `AllCleared` event to wait for, and `pending_load` / `pending_undo_extras`
/// should be cleared immediately rather than left for the `AllCleared`
/// handler that will never fire.
pub fn try_diff_replay(
    r: &mut Resonance,
    target: &LoadedProject,
    extras: &UndoExtras,
) -> bool {
    let current = build_project_file(r);
    let target_file = &target.file;

    if !structurally_compatible(&current, target_file) {
        return false;
    }

    // -- Global transport / master -------------------------------------
    apply_global(r, &current, target_file);

    // -- Tracks --------------------------------------------------------
    apply_tracks(r, &current, target_file);

    // -- Busses --------------------------------------------------------
    apply_busses(r, &current, target_file);

    // -- Aux sends -----------------------------------------------------
    // A send is a plain routing edge: `SetAuxSend` upserts one and
    // `RemoveAuxSend` drops one, both surgical, so adding or removing a
    // send never has to force the slow path (unlike a plugin instance,
    // which can only be re-created wholesale). Reconciled after the
    // busses so a send restored alongside its return bus lands second.
    apply_sends(r, &current, target_file);

    // -- Sidechain key routes ------------------------------------------
    // A key route is another plain routing edge — `SetSidechainRoute`
    // upserts one and `ClearSidechainRoute` drops one, both surgical — so
    // keying and unkeying a plugin never forces the slow path either
    // (ba todo #1311).
    apply_sidechain_routes(r, &current, target_file);

    // -- Master FX -----------------------------------------------------
    apply_master(r, &current, target_file);

    // -- Plugin state blobs --------------------------------------------
    // Re-push a snapshot's blob only when the cache moved on since it was
    // taken (editor close, preset capture, save): then the plugin's
    // internal state may differ from the snapshot's. When the snapshot
    // holds the very blob still cached (`Arc::ptr_eq`), nothing but the
    // params can have drifted — the blob is refreshed at every point that
    // changes anything else — so pushing it would only reset the params
    // the next step has to repair (FU-A2b).
    let pushed = push_all_plugin_states(r, target);

    // -- Plugin parameter values ---------------------------------------
    // The blob just pushed is only as fresh as its last refresh (plugin
    // add, editor close, save) — never a param edit — so the snapshot's
    // own values are re-applied after it, to the mirror and the engine,
    // exactly as a load does. Without this an undone knob kept its value
    // on screen and in the next save while the engine sat on the stale
    // blob (STATE-03). A plugin whose blob was not pushed only needs the
    // params that differ from the live mirror (FU-A2b).
    apply_all_plugin_params(r, target_file, &pushed);

    // -- Audio clips: scalar reposition / retrim only ------------------
    apply_audio_clips(r, &current, target_file);

    // -- MIDI clips: reposition + replace notes via delete+reload ------
    apply_midi_clips(r, &current, target_file, &target.midi_notes);

    // -- Compose state (definitions, placements, drum groups, lyrics) --
    apply_compose(r, target_file, extras);
    apply_track_groups(r, target_file);
    apply_markers(r, target_file);

    // -- Media pool (doc #175) -----------------------------------------
    // The pool is pure app-side data (no engine instances), so an
    // undo/redo that added / removed / relinked an asset is restored
    // verbatim here — the structural check ignores the pool entirely.
    // Asset-ref changes on clips were already mirrored in
    // `apply_audio_clips`; this rebuilds the asset list and usage tally.
    apply_pool(r, target_file);

    // -- Cycle-record take lanes (epic #15, todo #412/#1394) -----------
    // An undo/redo that promoted a segment, soloed a take or deleted one
    // is reconciled verbatim here: the mirror is rebuilt from the target
    // snapshot and `replay_take_groups` pushes the result into the engine
    // with `RestoreTakeGroups`. That command replaces the engine's store
    // wholesale, which is what this path needs — unlike a project load it
    // sends no `ClearAll`, so a merge would resurrect the very takes an
    // undo just deleted. Take groups never alter the project shape, so,
    // like the pool and the quantize state, they are absent from
    // `structurally_compatible` and always take this fast path.
    apply_take_groups(r, target_file);

    // -- Quantize state (ba todo #395) ---------------------------------
    // The groove library + last-used quantize settings are pure app-side
    // data (no engine instances), so an undo/redo that edited them is
    // restored verbatim. They never alter the project shape, so this
    // always takes the fast path.
    super::replay::restore_quantize(r, target_file);

    // -- Performance footer (tuning + capo) ----------------------------
    // Pure app-side, persisted in the `ProjectFile`: restored verbatim as
    // the slow path's `replay_loaded_project` does, so both paths land on
    // the same state (FU-H2c).
    super::replay::restore_performance(r, target_file);

    // -- Reference (A/B) content ---------------------------------------
    // References aren't in the `ProjectFile`, so they ride along in the
    // snapshot's `extras` and are restored verbatim here. Engine re-sync
    // of references across undo lands with the A/B playback work; the
    // engine reference handlers are stubs until then.
    r.reference.restore_undo(extras.reference.clone());

    // -- Global chord track --------------------------------------------
    // App-side metadata restored wholesale from the snapshot's file, as
    // `replay_globals` does on the slow path. The structural check
    // ignores it — chord edits never alter the project shape, so they
    // always take this fast path.
    r.chord_track = target_file.chord_track.to_chord_track();

    // -- Track freeze status (detach/delete caches no longer frozen) ----
    r.apply_freeze_restore(extras.track_freeze.clone());

    // -- External-instrument config -----------------------------------
    // From `ProjectTrack::external_instrument`, as `replay_track` does on
    // the slow path. Entering or leaving external mode never alters the
    // project shape, so it always takes this fast path.
    r.restore_external_instruments(target_file);

    // -- Tempo / signature events --------------------------------------
    apply_tempo(r, target_file);

    // -- Automation lanes ----------------------------------------------
    // Lanes aren't part of `ProjectFile` (and so don't affect the
    // structural check), so reconcile them straight from the snapshot's
    // extras: clear lanes that went away, re-send those that changed.
    r.restore_automation_lanes(&extras.automation_lanes);

    // -- Sort track / bus registry so view-layer invariant holds -------
    r.registry.resort_tracks();
    r.registry.resort_busses();
    r.view_caches.rebuild_output(&r.registry.busses);
    r.compose.refresh_track_count(&r.registry.tracks);

    // Rebuild runtime-only caches that aren't captured in the snapshot.
    // Mirrors the tail end of `replay_loaded_project` so the Compose tab
    // shows the right vocal audio clips after the restore. The derived
    // MIDI clip map is *not* rebuilt here: `apply_compose` restored the
    // snapshot's, exactly as `finalize_undo_restore` does (FU-H2a).
    use std::collections::HashSet;
    let vocal_track_ids: HashSet<resonance_audio::types::TrackId> = r
        .registry
        .tracks
        .iter()
        .filter(|t| t.track_type == resonance_audio::types::TrackType::Vocal)
        .map(|t| t.id)
        .collect();
    let project_dir = r.io.project_path.clone().unwrap_or_default();
    let audio_clip_paths: HashMap<resonance_audio::types::ClipId, std::path::PathBuf> = target
        .file
        .clips
        .iter()
        .map(|pc| (pc.id, project_dir.join(&pc.audio_file)))
        .collect();
    r.compose
        .rebuild_vocal_audio_clips(&r.clips, &audio_clip_paths, &vocal_track_ids, &r.tempo_map);

    true
}

// =====================================================================
// Structural comparison
// =====================================================================

/// True iff the two project files have the same set of structural
/// identifiers — track ids, plugin instance ids, clip ids, etc. —
/// arranged into the same parent-child shape. Pure ordering of the
/// outer collections is normalised via id-sort before comparison so a
/// re-ordering by `.order` alone does NOT force the slow path.
pub fn structurally_compatible(a: &ProjectFile, b: &ProjectFile) -> bool {
    // Track set + per-track plugin set, sub-track linkage, track type,
    // and clap plugin identity.
    if !track_set_matches(&a.tracks, &b.tracks) {
        return false;
    }
    // Bus set + per-bus plugin set.
    if !bus_set_matches(&a.busses, &b.busses) {
        return false;
    }
    // Master plugin set.
    if !plugin_set_matches(&a.master_plugins, &b.master_plugins) {
        return false;
    }
    // Audio + MIDI clip ids + clip→track binding (a clip that moved to a
    // different track is structural — we can `MoveClip` but the GUI
    // state needs more care; force fallback for safety).
    if !audio_clip_set_matches(&a.clips, &b.clips) {
        return false;
    }
    if !midi_clip_set_matches(&a.midi_clips, &b.midi_clips) {
        return false;
    }
    // Compose: section definitions/placements + drum groups by id only.
    if !id_set_eq(
        a.section_definitions.iter().map(|d| d.id),
        b.section_definitions.iter().map(|d| d.id),
    ) {
        return false;
    }
    if !id_set_eq(
        a.section_placements.iter().map(|p| p.id),
        b.section_placements.iter().map(|p| p.id),
    ) {
        return false;
    }
    if !id_set_eq(
        a.drum_groups.iter().map(|g| g.id),
        b.drum_groups.iter().map(|g| g.id),
    ) {
        return false;
    }
    if !id_set_eq(
        a.drum_patterns.iter().map(|p| p.id),
        b.drum_patterns.iter().map(|p| p.id),
    ) {
        return false;
    }
    if !id_set_eq(
        a.track_groups.iter().map(|g| g.id),
        b.track_groups.iter().map(|g| g.id),
    ) {
        return false;
    }
    if !id_set_eq(
        a.arrangement_markers.iter().map(|m| m.id),
        b.arrangement_markers.iter().map(|m| m.id),
    ) {
        return false;
    }
    true
}

pub fn id_set_eq<I, J>(a: I, b: J) -> bool
where
    I: IntoIterator<Item = u64>,
    J: IntoIterator<Item = u64>,
{
    let mut av: Vec<u64> = a.into_iter().collect();
    let mut bv: Vec<u64> = b.into_iter().collect();
    av.sort_unstable();
    bv.sort_unstable();
    av == bv
}

fn track_set_matches(a: &[ProjectTrack], b: &[ProjectTrack]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let by_id_b: HashMap<u64, &ProjectTrack> = b.iter().map(|t| (t.id, t)).collect();
    for ta in a {
        let Some(tb) = by_id_b.get(&ta.id) else {
            return false;
        };
        // Track-shape changes that the fast path cannot fix:
        if ta.track_type != tb.track_type {
            return false;
        }
        if ta.sub_track != tb.sub_track {
            return false;
        }
        if !plugin_set_matches(&ta.plugins, &tb.plugins) {
            return false;
        }
    }
    true
}

fn bus_set_matches(a: &[ProjectBus], b: &[ProjectBus]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let by_id_b: HashMap<u64, &ProjectBus> = b.iter().map(|x| (x.id, x)).collect();
    for ba in a {
        let Some(bb) = by_id_b.get(&ba.id) else {
            return false;
        };
        if !plugin_set_matches(&ba.plugins, &bb.plugins) {
            return false;
        }
    }
    true
}

fn plugin_set_matches(a: &[ProjectPlugin], b: &[ProjectPlugin]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    // Chain order matters: reordering plugins in the FX chain is a
    // structural change the engine doesn't expose a surgical command
    // for today. Zip+compare by position covers both "same ids in the
    // same order" and "identity bytes match for each slot".
    a.iter().zip(b.iter()).all(|(pa, pb)| {
        pa.instance_id == pb.instance_id
            && pa.clap_plugin_id == pb.clap_plugin_id
            && pa.clap_file_path == pb.clap_file_path
    })
}

fn audio_clip_set_matches(a: &[ProjectClip], b: &[ProjectClip]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let by_id_b: HashMap<u64, &ProjectClip> = b.iter().map(|c| (c.id, c)).collect();
    for ca in a {
        let Some(cb) = by_id_b.get(&ca.id) else {
            return false;
        };
        // Track reassignment can ride through MoveClip surgically (track
        // and start sample are both arguments). But the underlying WAV
        // file must be identical — a re-import would have produced a
        // new id, so this is mostly defensive.
        if ca.audio_file != cb.audio_file {
            return false;
        }
        if ca.total_frames != cb.total_frames {
            return false;
        }
    }
    true
}

fn midi_clip_set_matches(a: &[ProjectMidiClip], b: &[ProjectMidiClip]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let ids_a: std::collections::HashSet<u64> = a.iter().map(|c| c.id).collect();
    let ids_b: std::collections::HashSet<u64> = b.iter().map(|c| c.id).collect();
    ids_a == ids_b
}

// =====================================================================
// Apply layer
// =====================================================================

fn apply_global(r: &mut Resonance, a: &ProjectFile, b: &ProjectFile) {
    if a.bpm != b.bpm {
        r.transport.bpm = b.bpm;
        let _ = r.engine.send(AudioCommand::SetBpm { bpm: b.bpm });
    }
    if a.time_sig_num != b.time_sig_num || a.time_sig_den != b.time_sig_den {
        r.transport.time_sig_num = b.time_sig_num;
        r.transport.time_sig_den = b.time_sig_den;
        let _ = r.engine.send(AudioCommand::SetTimeSignature {
            numerator: b.time_sig_num,
            denominator: b.time_sig_den,
        });
    }
    if a.metronome_enabled != b.metronome_enabled {
        r.transport.metronome_enabled = b.metronome_enabled;
        let _ = r.engine.send(AudioCommand::SetMetronomeEnabled {
            enabled: b.metronome_enabled,
        });
    }
    if a.master_volume != b.master_volume {
        r.master_volume = b.master_volume;
        let _ = r.engine.send(AudioCommand::SetMasterVolume {
            volume: db_to_gain(b.master_volume),
        });
    }
    if a.loop_enabled != b.loop_enabled
        || a.loop_in != b.loop_in
        || a.loop_out != b.loop_out
    {
        r.transport.loop_enabled = b.loop_enabled;
        r.transport.loop_in = b.loop_in;
        r.transport.loop_out = b.loop_out;
        r.transport.loop_range_set = b.loop_enabled;
        let _ = r.engine.send(AudioCommand::SetLoopRange {
            enabled: b.loop_enabled,
            loop_in: b.loop_in,
            loop_out: b.loop_out,
        });
    }
    if a.midi_clock_send_enabled != b.midi_clock_send_enabled
        || a.midi_clock_send_device != b.midi_clock_send_device
    {
        r.midi_clock_send_enabled = b.midi_clock_send_enabled;
        r.midi_clock_send_device = b.midi_clock_send_device.clone();
        let _ = r.engine.send(AudioCommand::SetMidiClockOutput {
            device: b.midi_clock_send_device.clone(),
            enabled: b.midi_clock_send_enabled,
        });
    }
    if a.midi_clock_recv_enabled != b.midi_clock_recv_enabled
        || a.midi_clock_recv_device != b.midi_clock_recv_device
    {
        r.midi_clock_recv_enabled = b.midi_clock_recv_enabled;
        r.midi_clock_recv_device = b.midi_clock_recv_device.clone();
        let _ = r.engine.send(AudioCommand::SetMidiClockInput {
            device: b.midi_clock_recv_device.clone(),
            enabled: b.midi_clock_recv_enabled,
        });
    }
}

fn apply_tracks(r: &mut Resonance, a: &ProjectFile, b: &ProjectFile) {
    let a_by_id: HashMap<u64, &ProjectTrack> = a.tracks.iter().map(|t| (t.id, t)).collect();
    for tb in &b.tracks {
        let ta = a_by_id
            .get(&tb.id)
            .copied();
        let Some(ta) = ta else {
            // Defence in depth: `structurally_compatible` should have
            // gated us here, but if it ever drifts we'd rather skip an
            // unmatched id than crash on undo. The caller may detect
            // a stale slot and fall back to a full replay.
            continue;
        };
        apply_track(r, ta, tb);
    }
}

fn apply_track(r: &mut Resonance, a: &ProjectTrack, b: &ProjectTrack) {
    let track_id = b.id;
    if a.volume != b.volume {
        let _ = r.engine.send(AudioCommand::SetTrackVolume {
            track_id,
            volume: db_to_gain(b.volume),
        });
    }
    if a.pan != b.pan {
        let _ = r.engine.send(AudioCommand::SetTrackPan {
            track_id,
            pan: b.pan,
        });
    }
    if a.muted != b.muted {
        let _ = r.engine.send(AudioCommand::SetTrackMute {
            track_id,
            muted: b.muted,
        });
    }
    if a.soloed != b.soloed {
        let _ = r.engine.send(AudioCommand::SetTrackSolo {
            track_id,
            soloed: b.soloed,
        });
    }
    if a.record_armed != b.record_armed {
        let _ = r.engine.send(AudioCommand::SetTrackRecordArm {
            track_id,
            armed: b.record_armed,
        });
    }
    if a.monitor_enabled != b.monitor_enabled {
        let _ = r.engine.send(AudioCommand::SetTrackMonitor {
            track_id,
            enabled: b.monitor_enabled,
        });
    }
    if a.playback_source != b.playback_source {
        let _ = r.engine.send(AudioCommand::SetTrackPlaybackSource {
            track_id,
            source: b.playback_source,
        });
    }
    if a.mono != b.mono {
        let _ = r.engine.send(AudioCommand::SetTrackMono {
            track_id,
            mono: b.mono,
        });
    }
    if a.fx_bypassed != b.fx_bypassed {
        let _ = r.engine.send(AudioCommand::SetTrackFxBypass {
            track_id,
            bypassed: b.fx_bypassed,
        });
    }
    if a.input_device_name != b.input_device_name {
        let _ = r.engine.send(AudioCommand::SetTrackInputDevice {
            track_id,
            device_name: b.input_device_name.clone(),
        });
    }
    if a.input_port_index != b.input_port_index {
        if let Some(port_index) = b.input_port_index {
            let _ = r.engine.send(AudioCommand::SetTrackInputPort {
                track_id,
                port_index,
            });
        }
    }
    if a.midi_input_device != b.midi_input_device || a.midi_input_channel != b.midi_input_channel {
        let _ = r.engine.send(AudioCommand::SetTrackMidiInput {
            track_id,
            device: b.midi_input_device.clone(),
            channel: b.midi_input_channel,
        });
    }
    if a.midi_output_device != b.midi_output_device || a.midi_output_channel != b.midi_output_channel
    {
        let _ = r.engine.send(AudioCommand::SetTrackMidiOutput {
            track_id,
            device: b.midi_output_device.clone(),
            channel: b.midi_output_channel,
        });
    }
    if a.output_bus != b.output_bus {
        let output = b
            .output_bus
            .map(TrackOutput::Bus)
            .unwrap_or(TrackOutput::Master);
        let _ = r.engine.send(AudioCommand::SetTrackOutput {
            track_id,
            output,
        });
    }

    // Mirror onto GUI track state. The structural check guarantees the
    // track exists in `r.registry.tracks`.
    if let Some(t) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) {
        t.name = b.name.clone();
        t.order = b.order;
        t.volume = b.volume;
        t.pan = b.pan;
        t.muted = b.muted;
        t.soloed = b.soloed;
        t.fx_bypassed = b.fx_bypassed;
        t.record_armed = b.record_armed;
        t.monitor_enabled = b.monitor_enabled;
        t.playback_source = b.playback_source;
        t.mono = b.mono;
        t.input_device_name = b.input_device_name.clone();
        t.input_port_index = b.input_port_index.unwrap_or(0);
        t.output = b
            .output_bus
            .map(TrackOutput::Bus)
            .unwrap_or(TrackOutput::Master);
        t.instrument_type = b.instrument_type;
        t.instrument_icon = b.instrument_icon;
        t.role = b.role;
        t.midi_input_device = b.midi_input_device.clone();
        t.midi_input_channel = b.midi_input_channel;
        t.midi_output_device = b.midi_output_device.clone();
        t.midi_output_channel = b.midi_output_channel;
        // Plugin slot metadata: instance_id/clap identity are
        // guaranteed stable by the structural check, but the
        // human-visible name may change.
        for (slot, pp) in t.plugins.iter_mut().zip(b.plugins.iter()) {
            slot.plugin_name = pp.plugin_name.clone();
        }
        apply_plugin_bypass(&r.engine, &mut t.plugins, &b.plugins);
    }
}

fn apply_busses(r: &mut Resonance, a: &ProjectFile, b: &ProjectFile) {
    let a_by_id: HashMap<u64, &ProjectBus> = a.busses.iter().map(|x| (x.id, x)).collect();
    for bb in &b.busses {
        let ba = a_by_id
            .get(&bb.id)
            .copied();
        let Some(ba) = ba else {
            // Defence in depth — see the note in `apply_tracks`.
            continue;
        };
        apply_bus(r, ba, bb);
    }
}

fn apply_bus(r: &mut Resonance, a: &ProjectBus, b: &ProjectBus) {
    let bus_id = b.id;
    if a.volume != b.volume {
        let _ = r.engine.send(AudioCommand::SetBusVolume {
            bus_id,
            volume: db_to_gain(b.volume),
        });
    }
    if a.pan != b.pan {
        let _ = r.engine.send(AudioCommand::SetBusPan { bus_id, pan: b.pan });
    }
    if a.muted != b.muted {
        let _ = r.engine.send(AudioCommand::SetBusMute {
            bus_id,
            muted: b.muted,
        });
    }
    if a.fx_bypassed != b.fx_bypassed {
        let _ = r.engine.send(AudioCommand::SetBusFxBypass {
            bus_id,
            bypassed: b.fx_bypassed,
        });
    }
    if a.name != b.name {
        let _ = r.engine.send(AudioCommand::SetBusName {
            bus_id,
            name: b.name.clone(),
        });
    }
    if a.is_return != b.is_return {
        let _ = r.engine.send(AudioCommand::SetBusRole {
            bus_id,
            is_return: b.is_return,
        });
    }
    if let Some(bus) = r.registry.busses.iter_mut().find(|x| x.id == bus_id) {
        bus.name = b.name.clone();
        bus.order = b.order;
        bus.volume = b.volume;
        bus.pan = b.pan;
        bus.muted = b.muted;
        bus.fx_bypassed = b.fx_bypassed;
        bus.is_return = b.is_return;
        for (slot, pp) in bus.plugins.iter_mut().zip(b.plugins.iter()) {
            slot.plugin_name = pp.plugin_name.clone();
        }
        apply_plugin_bypass(&r.engine, &mut bus.plugins, &b.plugins);
    }
}

/// Reconcile the aux-send graph (ba doc #273) to the target snapshot:
/// upsert every send that is new or changed, drop every send the target
/// no longer has, and mirror both onto [`crate::state::AuxSendState`].
///
/// Sends whose fields already match are left untouched, so the common
/// undo (a fader move somewhere else entirely) emits no send traffic at
/// all — the same "only push what changed" rule the track and bus
/// appliers follow.
fn apply_sends(r: &mut Resonance, a: &ProjectFile, b: &ProjectFile) {
    let a_by_id: HashMap<u64, &ProjectSend> = a.sends.iter().map(|s| (s.id, s)).collect();

    // Removals drain FIRST so the reconciliation is order-independent.
    // Upserting first means a send that REPLACES another edge is checked
    // for feedback loops against a graph that still holds the edge it
    // replaces: undo across "delete bus A->B, create bus B->A" would
    // have the new edge rejected as a loop, then the old one removed,
    // leaving the engine with neither while the mirror shows the new one.
    let target_ids: std::collections::HashSet<u64> = b.sends.iter().map(|s| s.id).collect();
    for sa in &a.sends {
        if !target_ids.contains(&sa.id) {
            let _ = r.engine.send(AudioCommand::RemoveAuxSend { send_id: sa.id });
            r.aux.remove(sa.id);
        }
    }

    for sb in &b.sends {
        if a_by_id.get(&sb.id).copied() == Some(sb) {
            continue;
        }
        // Unknown source kind: drop rather than guess (see
        // `send_source_from_tag`).
        let Some(source) = send_source_from_tag(&sb.source_kind, sb.source_id) else {
            continue;
        };
        let _ = r.engine.send(AudioCommand::SetAuxSend {
            id_hint: Some(sb.id),
            source,
            dest: sb.dest_bus,
            level_db: sb.level_db,
            pre_fader: sb.pre_fader,
            enabled: sb.enabled,
        });
        r.aux.upsert(AuxSend {
            id: sb.id,
            source,
            dest: sb.dest_bus,
            level_db: sb.level_db,
            pre_fader: sb.pre_fader,
            enabled: sb.enabled,
        });
    }

}

/// Reconcile the sidechain key routes (ba doc #157/#159, todo #1311) to
/// the target snapshot: clear every route the target no longer has, then
/// upsert every route that is new or changed, mirroring both onto
/// [`crate::state::SidechainState`].
///
/// Removals drain first for the same order-independence reason
/// [`apply_sends`] gives, though the stakes are lower here: a route is
/// keyed by target plugin and simply replaces whatever that plugin had,
/// so there is no cycle check to fool. Routes whose fields already match
/// are left alone, so the common undo emits no key traffic at all.
fn apply_sidechain_routes(r: &mut Resonance, a: &ProjectFile, b: &ProjectFile) {
    let target_plugins: std::collections::HashSet<u64> = b
        .sidechain_routes
        .iter()
        .map(|route| route.plugin_instance_id)
        .collect();
    for ra in &a.sidechain_routes {
        if !target_plugins.contains(&ra.plugin_instance_id) {
            let _ = r.engine.send(AudioCommand::ClearSidechainRoute {
                plugin: ra.plugin_instance_id,
            });
            r.sidechain.clear_plugin(ra.plugin_instance_id);
        }
    }

    let a_by_plugin: HashMap<u64, &crate::project::ProjectSidechainRoute> = a
        .sidechain_routes
        .iter()
        .map(|route| (route.plugin_instance_id, route))
        .collect();
    for rb in &b.sidechain_routes {
        if a_by_plugin.get(&rb.plugin_instance_id).copied() == Some(rb) {
            continue;
        }
        // Unknown source kind: drop rather than guess (see
        // `send_source_from_tag`).
        let Some(source) = send_source_from_tag(&rb.source_kind, rb.source_id) else {
            continue;
        };
        let _ = r.engine.send(AudioCommand::SetSidechainRoute {
            plugin: rb.plugin_instance_id,
            source,
            enabled: rb.enabled,
        });
        r.sidechain.upsert(resonance_audio::types::SidechainRoute {
            plugin: rb.plugin_instance_id,
            source,
            enabled: rb.enabled,
        });
    }
}

fn apply_master(r: &mut Resonance, a: &ProjectFile, b: &ProjectFile) {
    if a.master_fx_bypassed != b.master_fx_bypassed {
        r.master_fx_bypassed = b.master_fx_bypassed;
        let _ = r.engine.send(AudioCommand::SetMasterFxBypass {
            bypassed: b.master_fx_bypassed,
        });
    }
    for (slot, pp) in r.master_plugins.iter_mut().zip(b.master_plugins.iter()) {
        slot.plugin_name = pp.plugin_name.clone();
    }
    let master_saved = b.master_plugins.clone();
    apply_plugin_bypass(&r.engine, &mut r.master_plugins, &master_saved);
}

fn push_all_plugin_states(
    r: &mut Resonance,
    target: &LoadedProject,
) -> std::collections::HashSet<PluginInstanceId> {
    // Walk every (track, bus, master) plugin in the target snapshot and
    // re-push the cached blob to the engine if it differs from the live
    // cache. Returns the instances whose blob was pushed.
    let mut pushed = std::collections::HashSet::new();
    for pt in &target.file.tracks {
        push_plugin_states(r, target, &pt.plugins, &mut pushed);
    }
    for pb in &target.file.busses {
        push_plugin_states(r, target, &pb.plugins, &mut pushed);
    }
    push_plugin_states(r, target, &target.file.master_plugins, &mut pushed);
    pushed
}

fn push_plugin_states(
    r: &mut Resonance,
    target: &LoadedProject,
    plugins: &[ProjectPlugin],
    pushed: &mut std::collections::HashSet<PluginInstanceId>,
) {
    for pp in plugins {
        if let Some(blob) = target.plugin_states.get(&pp.instance_id) {
            if r
                .plugin_state_cache
                .get(&pp.instance_id)
                .is_some_and(|live| std::sync::Arc::ptr_eq(live, blob))
            {
                continue;
            }
            let _ = r.engine.send(AudioCommand::LoadPluginState {
                instance_id: pp.instance_id,
                data: blob.to_vec(),
            });
            pushed.insert(pp.instance_id);
            // Track what was just pushed, so the cache matches the state
            // the engine now holds. For an instance that isn't live (a
            // missing `.clap`) this is the only copy that exists, and it
            // has to survive undo/redo as well as save (ba doc #275, P5).
            r.plugin_state_cache
                .insert(pp.instance_id, std::sync::Arc::clone(blob));
        }
    }
}

fn apply_all_plugin_params(
    r: &mut Resonance,
    b: &ProjectFile,
    pushed: &std::collections::HashSet<PluginInstanceId>,
) {
    for track in r.registry.tracks.iter_mut() {
        if let Some(pt) = b.tracks.iter().find(|t| t.id == track.id) {
            apply_plugin_params(&r.engine, &mut track.plugins, &pt.plugins, pushed);
        }
    }
    for bus in r.registry.busses.iter_mut() {
        if let Some(pb) = b.busses.iter().find(|x| x.id == bus.id) {
            apply_plugin_params(&r.engine, &mut bus.plugins, &pb.plugins, pushed);
        }
    }
    apply_plugin_params(&r.engine, &mut r.master_plugins, &b.master_plugins, pushed);
}

/// Drive every live slot's params to the snapshot's values: the saved
/// override where there is one, the plugin's default otherwise (only
/// non-defaults are saved). For a slot whose blob was just `pushed`, a
/// param is re-sent when it is non-default on either side — that covers
/// every changed value, and every value the stale blob may have reset.
/// For any other slot the engine still holds the live values the mirror
/// shows, so only the params that differ from the mirror are sent
/// (FU-A2b). A slot with no live params (a missing `.clap`) has nothing
/// to drive.
fn apply_plugin_params(
    engine: &resonance_audio::AudioEngine,
    slots: &mut [crate::state::PluginSlotState],
    saved: &[ProjectPlugin],
    pushed: &std::collections::HashSet<PluginInstanceId>,
) {
    for slot in slots.iter_mut() {
        let Some(pp) = saved.iter().find(|p| p.instance_id == slot.instance_id) else {
            continue;
        };
        let blob_pushed = pushed.contains(&slot.instance_id);
        for param in slot.params.iter_mut() {
            let target = pp
                .params
                .iter()
                .find(|p| p.id == param.id)
                .map_or(param.default_value, |p| p.value);
            let unchanged = if blob_pushed {
                param.current_value == param.default_value && target == param.default_value
            } else {
                param.current_value == target
            };
            if unchanged {
                continue;
            }
            param.current_value = target;
            let _ = engine.send(AudioCommand::SetPluginParam {
                instance_id: slot.instance_id,
                param_id: param.id,
                value: target,
            });
        }
    }
}

fn apply_audio_clips(r: &mut Resonance, a: &ProjectFile, b: &ProjectFile) {
    let a_by_id: HashMap<u64, &ProjectClip> = a.clips.iter().map(|c| (c.id, c)).collect();
    for cb in &b.clips {
        let ca = a_by_id
            .get(&cb.id)
            .copied();
        let Some(ca) = ca else {
            // Defence in depth — see the note in `apply_tracks`.
            continue;
        };
        let trim_changed = ca.trim_start_frames != cb.trim_start_frames
            || ca.trim_end_frames != cb.trim_end_frames;
        let moved = ca.start_sample != cb.start_sample || ca.track_id != cb.track_id;
        if trim_changed {
            let _ = r.engine.send(AudioCommand::TrimClip {
                clip_id: cb.id,
                new_start_sample: cb.start_sample,
                trim_start_frames: cb.trim_start_frames,
                trim_end_frames: cb.trim_end_frames,
            });
        } else if moved {
            let _ = r.engine.send(AudioCommand::MoveClip {
                clip_id: cb.id,
                new_start_sample: cb.start_sample,
                new_track_id: cb.track_id,
            });
        }

        // Fades & per-clip gain (epic #18, doc #156). Independent of the
        // trim/move fast path above — a snapshot diff (save/load or
        // undo/redo) that only changes a fade length, curve, or gain must
        // still reach the engine and the GUI mirror. Curves round-trip as
        // tags, so compare the parsed `FadeCurve` (normalizing unknown /
        // legacy tags) rather than the raw strings.
        let fa_in = fade_curve_from_tag(&ca.fade_in_curve);
        let fb_in = fade_curve_from_tag(&cb.fade_in_curve);
        let fa_out = fade_curve_from_tag(&ca.fade_out_curve);
        let fb_out = fade_curve_from_tag(&cb.fade_out_curve);
        let fade_changed = ca.fade_in_frames != cb.fade_in_frames
            || ca.fade_out_frames != cb.fade_out_frames
            || fa_in != fb_in
            || fa_out != fb_out;
        let gain_changed = ca.gain_db != cb.gain_db;
        if fade_changed {
            let _ = r.engine.send(AudioCommand::SetClipFade {
                clip_id: cb.id,
                fade_in_frames: cb.fade_in_frames,
                fade_in_curve: fb_in,
                fade_out_frames: cb.fade_out_frames,
                fade_out_curve: fb_out,
            });
        }
        if gain_changed {
            let _ = r.engine.send(AudioCommand::SetClipGain {
                clip_id: cb.id,
                gain_db: cb.gain_db,
            });
        }

        // Mirror onto GUI state.
        if let Some(cs) = r.clips.iter_mut().find(|c| c.id == cb.id) {
            cs.start_sample = cb.start_sample;
            cs.track_id = cb.track_id;
            cs.trim_start_frames = cb.trim_start_frames;
            cs.trim_end_frames = cb.trim_end_frames;
            cs.name = cb.name.clone();
            cs.duration_samples = cb
                .total_frames
                .saturating_sub(cb.trim_start_frames)
                .saturating_sub(cb.trim_end_frames);
            // Fade/gain mirror (epic #18, doc #156).
            cs.fade_in_frames = cb.fade_in_frames;
            cs.fade_in_curve = fb_in;
            cs.fade_out_frames = cb.fade_out_frames;
            cs.fade_out_curve = fb_out;
            cs.gain_db = cb.gain_db;
            // Pool link (doc #175): restore the clip's asset ref so an
            // undo/redo that relinked or cleared the link is reflected.
            cs.asset_ref = cb.asset_ref.map(crate::state::pool::AssetRef::new);
        }
    }
}

fn apply_midi_clips(
    r: &mut Resonance,
    a: &ProjectFile,
    b: &ProjectFile,
    target_notes: &HashMap<ClipId, Vec<MidiNote>>,
) {
    let a_by_id: HashMap<u64, &ProjectMidiClip> =
        a.midi_clips.iter().map(|c| (c.id, c)).collect();
    let current_notes: HashMap<ClipId, Vec<MidiNote>> = r
        .midi_clips
        .iter()
        .map(|mc| (mc.id, mc.notes.clone()))
        .collect();

    for cb in &b.midi_clips {
        let ca = a_by_id
            .get(&cb.id)
            .copied();
        let Some(ca) = ca else {
            // Defence in depth — see the note in `apply_tracks`.
            continue;
        };
        let target_for_clip = target_notes.get(&cb.id).cloned().unwrap_or_default();
        let current_for_clip = current_notes.get(&cb.id).cloned().unwrap_or_default();
        let notes_changed = !midi_notes_equal(&target_for_clip, &current_for_clip);
        let trim_changed = ca.trim_start_ticks != cb.trim_start_ticks
            || ca.trim_end_ticks != cb.trim_end_ticks;
        let moved = ca.start_sample != cb.start_sample || ca.track_id != cb.track_id;
        let duration_changed = ca.duration_ticks != cb.duration_ticks;

        if notes_changed || duration_changed {
            // Delete + reload preserves the clip id, so the rest of the
            // engine state (track binding, derived-clip map keys) stays
            // consistent. Cheaper than a full ClearAll.
            let _ = r.engine.send(AudioCommand::DeleteMidiClip { clip_id: cb.id });
            let _ = r.engine.send(AudioCommand::LoadMidiClipDirect {
                clip_id: cb.id,
                track_id: cb.track_id,
                start_sample: cb.start_sample,
                duration_ticks: cb.duration_ticks,
                notes: target_for_clip.clone(),
                name: cb.name.clone(),
                trim_start_ticks: cb.trim_start_ticks,
                trim_end_ticks: cb.trim_end_ticks,
            });
        } else if trim_changed {
            let _ = r.engine.send(AudioCommand::TrimMidiClip {
                clip_id: cb.id,
                new_start_sample: cb.start_sample,
                trim_start_ticks: cb.trim_start_ticks,
                trim_end_ticks: cb.trim_end_ticks,
            });
        } else if moved {
            let _ = r.engine.send(AudioCommand::MoveMidiClip {
                clip_id: cb.id,
                new_start_sample: cb.start_sample,
                new_track_id: cb.track_id,
            });
        }

        if let Some(mc) = r.midi_clips.iter_mut().find(|c| c.id == cb.id) {
            mc.start_sample = cb.start_sample;
            mc.track_id = cb.track_id;
            mc.duration_ticks = cb.duration_ticks;
            mc.trim_start_ticks = cb.trim_start_ticks;
            mc.trim_end_ticks = cb.trim_end_ticks;
            mc.name = cb.name.clone();
            mc.notes = target_for_clip;
        }
    }
}

fn apply_compose(r: &mut Resonance, b: &ProjectFile, extras: &UndoExtras) {
    // Section definitions / placements — drum arrangements included, from
    // `ProjectSectionDefinition::arrangement` — come back through
    // `load_from_project`, which clears runtime-only sub-state. After
    // that, restore the extras captured at snapshot time.
    r.compose
        .load_from_project(&b.section_definitions, &b.section_placements);
    // Restore the drum pattern bank. Modern snapshots persist
    // `drum_patterns`; older snapshots still in the undo stack only have
    // the legacy `drum_groups` field, so promote it the same way the
    // project loader does. Unlike the full load, an all-empty snapshot
    // clears the bank rather than keeping the seeded default.
    restore_drum_patterns(&mut r.compose, b, true);
    // After `apply_midi_clips`, so the counter is reserved past the
    // restored clips. No rebuild from the mirror: see
    // `restore_derived_clips` (FU-H2a).
    r.restore_derived_clips(
        extras.compose_derived_clips.clone(),
        extras.compose_next_derived_clip_id,
        true,
    );
    r.compose.vocal_audio.clip_lyrics = extras.vocal_clip_lyrics.clone();
}

/// `MidiNote` is a plain bag of `u8/f32/u64` fields but does not derive
/// `PartialEq` (the engine has no need for it). Comparing field-wise
/// here keeps the diff replay self-contained without touching the
/// engine crate's public API.
pub fn midi_notes_equal(a: &[MidiNote], b: &[MidiNote]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(x, y)| {
        x.note == y.note
            && x.velocity.to_bits() == y.velocity.to_bits()
            && x.start_tick == y.start_tick
            && x.duration_ticks == y.duration_ticks
    })
}

fn apply_tempo(r: &mut Resonance, b: &ProjectFile) {
    restore_tempo_events(r, b);
    r.rebuild_and_send_tempo();
}

/// Restore the track group registry from a saved project file. The
/// group id set is guaranteed equal by `structurally_compatible`, but
/// the per-group contents (membership, collapse state, nesting, macros)
/// may differ, so the registry is rebuilt wholesale from the snapshot.
/// `add_group` drops membership edges that would close a nested-group
/// cycle, so a corrupted `track_groups` array loads with the cycle
/// broken rather than aborting the flattening walks downstream.
fn apply_track_groups(r: &mut Resonance, b: &ProjectFile) {
    r.track_groups = crate::state::TrackGroupRegistry::new();
    for tg in &b.track_groups {
        r.track_groups.add_group(tg.clone());
        // Same counter bump as the slow path (code review STATE-04).
        if tg.id >= r.registry.next_sub_track_id {
            r.registry.next_sub_track_id = tg.id + 1;
        }
    }
}

fn apply_markers(r: &mut Resonance, b: &ProjectFile) {
    r.markers = crate::state::ArrangementMarkers::from(b.arrangement_markers.clone());
}

/// Restore the media pool from a snapshot on the fast (diff) path (doc
/// #175). Mirrors the slow-path [`super::replay::restore_pool`] but uses
/// `r.io.project_path` as the directory to resolve relative asset paths
/// against — it's still set during an in-session undo/redo (the slow path
/// has to thread the dir explicitly because replay clears it). Rebuilds
/// the asset list (flagging missing files) and recomputes usage from the
/// clips' already-mirrored asset refs.
fn apply_pool(r: &mut Resonance, b: &ProjectFile) {
    use crate::state::pool::PoolAsset;

    r.pool.clear_assets();
    let project_dir = r.io.project_path.clone();
    for pa in &b.pool_assets {
        let missing = match &project_dir {
            Some(dir) => !dir.join(&pa.project_relative_path).exists(),
            // No anchored path (shouldn't happen on the undo path, which
            // only records with a saved project) — assume present rather
            // than spuriously flag everything missing.
            None => false,
        };
        r.pool.add(PoolAsset {
            id: pa.id,
            project_relative_path: pa.project_relative_path.clone(),
            original_path: pa.original_path.clone(),
            format: crate::project::audio_format_from_tag(&pa.format),
            channels: pa.channels,
            source_sample_rate: pa.source_sample_rate,
            duration_frames: pa.duration_frames,
            thumbnail_peaks: Vec::new(),
            missing,
        });
    }
    r.recompute_pool_usage();
}

/// Restore the cycle-record take lanes from a snapshot on the fast (diff)
/// path (epic #15, todo #412).
///
/// Same split as the slow path, just with the clear inlined: the wipe that
/// `wipe_registry` performs there has no counterpart on this path (nothing
/// is cleared at all), so it happens here immediately before the shared
/// [`super::replay::replay_take_groups`] re-seeds.
///
/// The project directory comes from `r.io.project_path`, still set during
/// an in-session undo/redo — exactly as [`apply_pool`] resolves its assets
/// — so a take whose WAV was deleted mid-session is re-flagged on every
/// history step rather than going quiet.
///
/// **The clear is [`clear_for_snapshot`], not [`clear`]** (ba todo #1400):
/// it keeps the waveform peaks. This function runs on *every* history
/// step, a fader undo included, and since #1400 `replay_take_groups`
/// reads each take's WAV rather than stat-ing it — so throwing the tables
/// away here would put an mmap and a full scan of every take in the
/// project on a hold-to-repeat gesture (~3.2 ms per recorded minute).
/// With them kept, `replay_take_groups` skips every already-read take and
/// an undo costs no filesystem access at all. Safe because a recording is
/// immutable and a cached table records the `clip_ref` it came from, so a
/// snapshot naming a different recording under the same key misses and
/// re-reads.
///
/// [`clear_for_snapshot`]: crate::state::TakeGroupState::clear_for_snapshot
/// [`clear`]: crate::state::TakeGroupState::clear
fn apply_take_groups(r: &mut Resonance, b: &ProjectFile) {
    // No anchored path (can't happen on the undo path, which only records
    // with a saved project): an empty dir joins to a bare relative path
    // that won't exist, which flags rather than hides — the safe way round
    // for takes, where "present" is the claim that could mislead.
    let project_dir = r.io.project_path.clone().unwrap_or_default();
    r.take_groups.clear_for_snapshot();
    super::replay::replay_take_groups(r, b, &project_dir);
}

/// Apply the saved per-slot bypass to one chain, telling the engine about
/// every slot that actually moved (ba todo #1305).
///
/// This is what makes bypass UNDOABLE rather than merely persisted. Undo
/// restores through the diff replay, not through a reload, and the diff
/// used to copy only `plugin_name` per slot — so an undo of a bypass
/// recorded its entry, replayed, and changed nothing. `plugin_set_matches`
/// compares slot IDENTITY only, deliberately: a bypass-only change is not
/// a structural change and must not force the whole project to reload.
/// That means the difference has to be applied here, or nowhere.
///
/// Sends only on a real change. The engine crossfades a bypass, and
/// re-asserting the state a slot is already in would start a fade for a
/// value that is not moving.
fn apply_plugin_bypass(
    engine: &resonance_audio::AudioEngine,
    slots: &mut [crate::state::PluginSlotState],
    saved: &[ProjectPlugin],
) {
    for (slot, pp) in slots.iter_mut().zip(saved.iter()) {
        if slot.bypassed == pp.bypassed {
            continue;
        }
        slot.bypassed = pp.bypassed;
        let _ = engine.send(AudioCommand::SetPluginBypass {
            instance_id: slot.instance_id,
            bypassed: pp.bypassed,
        });
    }
}
