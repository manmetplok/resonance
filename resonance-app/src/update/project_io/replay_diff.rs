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
//! Plugin parameter restores: even when the cached state blob bytes
//! match between snapshots, the engine's live plugin instance may have
//! drifted (knob coalescing recorded the *pre-burst* blob; the engine
//! holds the *post-burst* state). The fast path always re-sends
//! `LoadPluginState` for every plugin that has a cached blob, so plugin
//! param undo works without a full re-instantiation.

use std::collections::HashMap;

use resonance_audio::types::*;

use crate::compose::DrumGroup;
use crate::project::{
    LoadedProject, ProjectBus, ProjectClip, ProjectFile, ProjectMidiClip, ProjectPlugin,
    ProjectTrack,
};
use crate::undo::UndoExtras;
use crate::util::db_to_gain;
use crate::Resonance;

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

    // -- Master FX -----------------------------------------------------
    apply_master(r, &current, target_file);

    // -- Plugin state blobs --------------------------------------------
    // Always re-push every cached blob: the engine's live plugin state
    // may have drifted from the snapshot point even when the cache key
    // bytes match (knob-burst coalescing captures the pre-burst blob).
    push_all_plugin_states(r, target);

    // -- Audio clips: scalar reposition / retrim only ------------------
    apply_audio_clips(r, &current, target_file);

    // -- MIDI clips: reposition + replace notes via delete+reload ------
    apply_midi_clips(r, &current, target_file, &target.midi_notes);

    // -- Compose state (definitions, placements, drum groups, lyrics) --
    apply_compose(r, target_file, extras);

    // -- Tempo / signature events --------------------------------------
    apply_tempo(r, target_file);

    // -- Sort track / bus registry so view-layer invariant holds -------
    r.registry.resort_tracks();
    r.registry.resort_busses();
    r.view_caches.rebuild_output(&r.registry.busses);

    // Rebuild runtime-only caches that aren't captured in the snapshot.
    // Mirrors the tail end of `replay_loaded_project` so the Compose tab
    // shows the right derived MIDI / vocal audio clips after the restore.
    let samples_per_beat = r.sample_rate as f64 * 60.0 / r.transport.bpm as f64;
    let samples_per_bar = (samples_per_beat * r.transport.time_sig_num as f64) as u64;
    r.compose
        .rebuild_derived_clips(&r.midi_clips, samples_per_bar);

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
        .rebuild_vocal_audio_clips(&r.clips, &audio_clip_paths, &vocal_track_ids, samples_per_bar);

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
fn structurally_compatible(a: &ProjectFile, b: &ProjectFile) -> bool {
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
    true
}

fn id_set_eq<I, J>(a: I, b: J) -> bool
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
        r.engine.send(AudioCommand::SetBpm { bpm: b.bpm });
    }
    if a.time_sig_num != b.time_sig_num || a.time_sig_den != b.time_sig_den {
        r.transport.time_sig_num = b.time_sig_num;
        r.transport.time_sig_den = b.time_sig_den;
        r.engine.send(AudioCommand::SetTimeSignature {
            numerator: b.time_sig_num,
            denominator: b.time_sig_den,
        });
    }
    if a.metronome_enabled != b.metronome_enabled {
        r.transport.metronome_enabled = b.metronome_enabled;
        r.engine.send(AudioCommand::SetMetronomeEnabled {
            enabled: b.metronome_enabled,
        });
    }
    if a.master_volume != b.master_volume {
        r.master_volume = b.master_volume;
        r.engine.send(AudioCommand::SetMasterVolume {
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
        r.engine.send(AudioCommand::SetLoopRange {
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
        r.engine.send(AudioCommand::SetMidiClockOutput {
            device: b.midi_clock_send_device.clone(),
            enabled: b.midi_clock_send_enabled,
        });
    }
    if a.midi_clock_recv_enabled != b.midi_clock_recv_enabled
        || a.midi_clock_recv_device != b.midi_clock_recv_device
    {
        r.midi_clock_recv_enabled = b.midi_clock_recv_enabled;
        r.midi_clock_recv_device = b.midi_clock_recv_device.clone();
        r.engine.send(AudioCommand::SetMidiClockInput {
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
        r.engine.send(AudioCommand::SetTrackVolume {
            track_id,
            volume: db_to_gain(b.volume),
        });
    }
    if a.pan != b.pan {
        r.engine.send(AudioCommand::SetTrackPan {
            track_id,
            pan: b.pan,
        });
    }
    if a.muted != b.muted {
        r.engine.send(AudioCommand::SetTrackMute {
            track_id,
            muted: b.muted,
        });
    }
    if a.soloed != b.soloed {
        r.engine.send(AudioCommand::SetTrackSolo {
            track_id,
            soloed: b.soloed,
        });
    }
    if a.record_armed != b.record_armed {
        r.engine.send(AudioCommand::SetTrackRecordArm {
            track_id,
            armed: b.record_armed,
        });
    }
    if a.monitor_enabled != b.monitor_enabled {
        r.engine.send(AudioCommand::SetTrackMonitor {
            track_id,
            enabled: b.monitor_enabled,
        });
    }
    if a.mono != b.mono {
        r.engine.send(AudioCommand::SetTrackMono {
            track_id,
            mono: b.mono,
        });
    }
    if a.fx_bypassed != b.fx_bypassed {
        r.engine.send(AudioCommand::SetTrackFxBypass {
            track_id,
            bypassed: b.fx_bypassed,
        });
    }
    if a.input_device_name != b.input_device_name {
        r.engine.send(AudioCommand::SetTrackInputDevice {
            track_id,
            device_name: b.input_device_name.clone(),
        });
    }
    if a.input_port_index != b.input_port_index {
        if let Some(port_index) = b.input_port_index {
            r.engine.send(AudioCommand::SetTrackInputPort {
                track_id,
                port_index,
            });
        }
    }
    if a.midi_input_device != b.midi_input_device || a.midi_input_channel != b.midi_input_channel {
        r.engine.send(AudioCommand::SetTrackMidiInput {
            track_id,
            device: b.midi_input_device.clone(),
            channel: b.midi_input_channel,
        });
    }
    if a.midi_output_device != b.midi_output_device || a.midi_output_channel != b.midi_output_channel
    {
        r.engine.send(AudioCommand::SetTrackMidiOutput {
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
        r.engine.send(AudioCommand::SetTrackOutput {
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
    }
}

fn apply_busses(r: &mut Resonance, a: &ProjectFile, b: &ProjectFile) {
    let a_by_id: HashMap<u64, &ProjectBus> = a.busses.iter().map(|x| (x.id, x)).collect();
    for bb in &b.busses {
        let ba = a_by_id
            .get(&bb.id)
            .copied();
        let Some(ba) = ba else {
            // Defence in depth: `structurally_compatible` should have
            // gated us here, but if it ever drifts we'd rather skip an
            // unmatched id than crash on undo. The caller may detect
            // a stale slot and fall back to a full replay.
            continue;
        };
        apply_bus(r, ba, bb);
    }
}

fn apply_bus(r: &mut Resonance, a: &ProjectBus, b: &ProjectBus) {
    let bus_id = b.id;
    if a.volume != b.volume {
        r.engine.send(AudioCommand::SetBusVolume {
            bus_id,
            volume: db_to_gain(b.volume),
        });
    }
    if a.pan != b.pan {
        r.engine.send(AudioCommand::SetBusPan { bus_id, pan: b.pan });
    }
    if a.muted != b.muted {
        r.engine.send(AudioCommand::SetBusMute {
            bus_id,
            muted: b.muted,
        });
    }
    if a.fx_bypassed != b.fx_bypassed {
        r.engine.send(AudioCommand::SetBusFxBypass {
            bus_id,
            bypassed: b.fx_bypassed,
        });
    }
    if a.name != b.name {
        r.engine.send(AudioCommand::SetBusName {
            bus_id,
            name: b.name.clone(),
        });
    }
    if let Some(bus) = r.registry.busses.iter_mut().find(|x| x.id == bus_id) {
        bus.name = b.name.clone();
        bus.order = b.order;
        bus.volume = b.volume;
        bus.pan = b.pan;
        bus.muted = b.muted;
        bus.fx_bypassed = b.fx_bypassed;
        for (slot, pp) in bus.plugins.iter_mut().zip(b.plugins.iter()) {
            slot.plugin_name = pp.plugin_name.clone();
        }
    }
}

fn apply_master(r: &mut Resonance, a: &ProjectFile, b: &ProjectFile) {
    if a.master_fx_bypassed != b.master_fx_bypassed {
        r.master_fx_bypassed = b.master_fx_bypassed;
        r.engine.send(AudioCommand::SetMasterFxBypass {
            bypassed: b.master_fx_bypassed,
        });
    }
    for (slot, pp) in r.master_plugins.iter_mut().zip(b.master_plugins.iter()) {
        slot.plugin_name = pp.plugin_name.clone();
    }
}

fn push_all_plugin_states(r: &mut Resonance, target: &LoadedProject) {
    // Walk every (track, bus, master) plugin in the target snapshot and
    // re-push the cached blob to the engine if one exists.
    for pt in &target.file.tracks {
        for pp in &pt.plugins {
            if let Some(blob) = target.plugin_states.get(&pp.instance_id) {
                r.engine.send(AudioCommand::LoadPluginState {
                    instance_id: pp.instance_id,
                    data: blob.clone(),
                });
            }
        }
    }
    for pb in &target.file.busses {
        for pp in &pb.plugins {
            if let Some(blob) = target.plugin_states.get(&pp.instance_id) {
                r.engine.send(AudioCommand::LoadPluginState {
                    instance_id: pp.instance_id,
                    data: blob.clone(),
                });
            }
        }
    }
    for pp in &target.file.master_plugins {
        if let Some(blob) = target.plugin_states.get(&pp.instance_id) {
            r.engine.send(AudioCommand::LoadPluginState {
                instance_id: pp.instance_id,
                data: blob.clone(),
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
            // Defence in depth: `structurally_compatible` should have
            // gated us here, but if it ever drifts we'd rather skip an
            // unmatched id than crash on undo. The caller may detect
            // a stale slot and fall back to a full replay.
            continue;
        };
        let trim_changed = ca.trim_start_frames != cb.trim_start_frames
            || ca.trim_end_frames != cb.trim_end_frames;
        let moved = ca.start_sample != cb.start_sample || ca.track_id != cb.track_id;
        if trim_changed {
            r.engine.send(AudioCommand::TrimClip {
                clip_id: cb.id,
                new_start_sample: cb.start_sample,
                trim_start_frames: cb.trim_start_frames,
                trim_end_frames: cb.trim_end_frames,
            });
        } else if moved {
            r.engine.send(AudioCommand::MoveClip {
                clip_id: cb.id,
                new_start_sample: cb.start_sample,
                new_track_id: cb.track_id,
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
            // Defence in depth: `structurally_compatible` should have
            // gated us here, but if it ever drifts we'd rather skip an
            // unmatched id than crash on undo. The caller may detect
            // a stale slot and fall back to a full replay.
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
            r.engine.send(AudioCommand::DeleteMidiClip { clip_id: cb.id });
            r.engine.send(AudioCommand::LoadMidiClipDirect {
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
            r.engine.send(AudioCommand::TrimMidiClip {
                clip_id: cb.id,
                new_start_sample: cb.start_sample,
                trim_start_ticks: cb.trim_start_ticks,
                trim_end_ticks: cb.trim_end_ticks,
            });
        } else if moved {
            r.engine.send(AudioCommand::MoveMidiClip {
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
    // Section definitions / placements come back through `load_from_project`,
    // which clears runtime-only sub-state. After that, restore the extras
    // captured at snapshot time.
    r.compose
        .load_from_project(&b.section_definitions, &b.section_placements);
    // Restore the drum pattern bank. Modern snapshots persist
    // `drum_patterns`; older snapshots still in the undo stack only have
    // the legacy `drum_groups` field, so promote it the same way the
    // project loader does.
    if !b.drum_patterns.is_empty() {
        r.compose.drum_patterns = b.drum_patterns.clone();
    } else if !b.drum_groups.is_empty() {
        let (patterns, _id) = crate::compose::drumroll::legacy_groups_to_pattern(
            b.drum_groups.clone(),
            &mut r.compose.next_id,
        );
        r.compose.drum_patterns = patterns;
    } else {
        r.compose.drum_patterns.clear();
    }
    r.compose.default_drum_pattern_id = r.compose.drum_patterns.first().map(|p| p.id);
    let max_id = r
        .compose
        .drum_patterns
        .iter()
        .flat_map(|p| std::iter::once(p.id).chain(p.groups.iter().map(|g| g.id)))
        .max();
    if let Some(m) = max_id {
        r.compose.next_id = r.compose.next_id.max(m);
    }
    r.compose.derived_clips = extras.compose_derived_clips.clone();
    r.compose.next_derived_clip_id = extras.compose_next_derived_clip_id;
    r.compose.vocal_audio.clip_lyrics = extras.vocal_clip_lyrics.clone();
}

/// `MidiNote` is a plain bag of `u8/f32/u64` fields but does not derive
/// `PartialEq` (the engine has no need for it). Comparing field-wise
/// here keeps the diff replay self-contained without touching the
/// engine crate's public API.
fn midi_notes_equal(a: &[MidiNote], b: &[MidiNote]) -> bool {
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
    if b.tempo_events.is_empty() {
        r.tempo_events = vec![crate::state::TempoEvent {
            bar: 0,
            bpm: b.bpm,
        }];
    } else {
        r.tempo_events = b.tempo_events.clone();
    }
    if b.signature_events.is_empty() {
        r.signature_events = vec![crate::state::SignatureEvent {
            bar: 0,
            numerator: b.time_sig_num,
            denominator: b.time_sig_den,
        }];
    } else {
        r.signature_events = b.signature_events.clone();
    }
    r.rebuild_and_send_tempo();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::PROJECT_FORMAT_VERSION;
    use crate::state::{InstrumentIcon, InstrumentType};

    fn empty_file() -> ProjectFile {
        ProjectFile {
            version: PROJECT_FORMAT_VERSION,
            sample_rate: 44100,
            bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
            metronome_enabled: false,
            master_volume: 0.0,
            master_plugins: Vec::new(),
            master_fx_bypassed: false,
            loop_enabled: false,
            loop_in: 0,
            loop_out: 0,
            tracks: Vec::new(),
            clips: Vec::new(),
            midi_clips: Vec::new(),
            busses: Vec::new(),
            section_definitions: Vec::new(),
            section_placements: Vec::new(),
            tempo_events: Vec::new(),
            signature_events: Vec::new(),
            midi_clock_send_enabled: false,
            midi_clock_send_device: None,
            midi_clock_recv_enabled: false,
            midi_clock_recv_device: None,
            drum_groups: Vec::new(),
            drum_patterns: Vec::new(),
        }
    }

    fn track(id: u64, vol: f32) -> ProjectTrack {
        ProjectTrack {
            id,
            name: format!("T{id}"),
            order: id as usize,
            volume: vol,
            pan: 0.0,
            muted: false,
            soloed: false,
            fx_bypassed: false,
            record_armed: false,
            monitor_enabled: false,
            mono: true,
            input_device_name: None,
            input_port_index: Some(0),
            plugins: Vec::new(),
            track_type: "audio".to_string(),
            output_bus: None,
            instrument_type: InstrumentType::default(),
            instrument_icon: InstrumentIcon::default(),
            role: None,
            sub_track: None,
            midi_input_device: None,
            midi_input_channel: None,
            midi_output_device: None,
            midi_output_channel: None,
        }
    }

    fn plugin(id: u64) -> ProjectPlugin {
        ProjectPlugin {
            instance_id: id,
            plugin_name: format!("P{id}"),
            clap_plugin_id: "com.example.foo".to_string(),
            clap_file_path: "/x/foo.clap".to_string(),
            state_file: format!("plugins/plugin_{id}.bin"),
        }
    }

    #[test]
    fn empty_projects_are_structurally_compatible() {
        let a = empty_file();
        let b = empty_file();
        assert!(structurally_compatible(&a, &b));
    }

    #[test]
    fn scalar_only_track_diff_is_compatible() {
        let mut a = empty_file();
        let mut b = empty_file();
        a.tracks = vec![track(1, 0.0)];
        b.tracks = vec![track(1, -6.0)];
        assert!(structurally_compatible(&a, &b));
    }

    #[test]
    fn added_track_forces_fallback() {
        let mut a = empty_file();
        let mut b = empty_file();
        a.tracks = vec![track(1, 0.0)];
        b.tracks = vec![track(1, 0.0), track(2, 0.0)];
        assert!(!structurally_compatible(&a, &b));
    }

    #[test]
    fn removed_track_forces_fallback() {
        let mut a = empty_file();
        let mut b = empty_file();
        a.tracks = vec![track(1, 0.0), track(2, 0.0)];
        b.tracks = vec![track(1, 0.0)];
        assert!(!structurally_compatible(&a, &b));
    }

    #[test]
    fn renumbered_track_forces_fallback() {
        let mut a = empty_file();
        let mut b = empty_file();
        a.tracks = vec![track(1, 0.0)];
        b.tracks = vec![track(2, 0.0)];
        assert!(!structurally_compatible(&a, &b));
    }

    #[test]
    fn track_type_change_forces_fallback() {
        let mut a = empty_file();
        let mut b = empty_file();
        a.tracks = vec![track(1, 0.0)];
        let mut t = track(1, 0.0);
        t.track_type = "instrument".to_string();
        b.tracks = vec![t];
        assert!(!structurally_compatible(&a, &b));
    }

    #[test]
    fn added_plugin_forces_fallback() {
        let mut a = empty_file();
        let mut b = empty_file();
        a.tracks = vec![track(1, 0.0)];
        let mut t = track(1, 0.0);
        t.plugins = vec![plugin(10)];
        b.tracks = vec![t];
        assert!(!structurally_compatible(&a, &b));
    }

    #[test]
    fn plugin_reorder_forces_fallback() {
        let mut a = empty_file();
        let mut b = empty_file();
        let mut t_a = track(1, 0.0);
        t_a.plugins = vec![plugin(10), plugin(11)];
        let mut t_b = track(1, 0.0);
        t_b.plugins = vec![plugin(11), plugin(10)];
        a.tracks = vec![t_a];
        b.tracks = vec![t_b];
        assert!(!structurally_compatible(&a, &b));
    }

    #[test]
    fn plugin_clap_identity_change_forces_fallback() {
        let mut a = empty_file();
        let mut b = empty_file();
        let mut t_a = track(1, 0.0);
        t_a.plugins = vec![plugin(10)];
        let mut p = plugin(10);
        p.clap_plugin_id = "com.example.bar".to_string();
        let mut t_b = track(1, 0.0);
        t_b.plugins = vec![p];
        a.tracks = vec![t_a];
        b.tracks = vec![t_b];
        assert!(!structurally_compatible(&a, &b));
    }

    #[test]
    fn id_set_eq_ignores_order() {
        assert!(id_set_eq([1u64, 2, 3], [3u64, 2, 1]));
        assert!(!id_set_eq([1u64, 2], [1u64, 2, 3]));
    }

    #[test]
    fn track_reorder_alone_is_compatible() {
        // Reorder via `.order` field — the actual track set is unchanged.
        let mut a = empty_file();
        let mut b = empty_file();
        a.tracks = vec![track(1, 0.0), track(2, 0.0)];
        let mut t1 = track(1, 0.0);
        t1.order = 1;
        let mut t2 = track(2, 0.0);
        t2.order = 0;
        b.tracks = vec![t2, t1];
        assert!(structurally_compatible(&a, &b));
    }

    #[test]
    fn audio_file_path_change_forces_fallback() {
        let mut a = empty_file();
        let mut b = empty_file();
        let mk = |id: u64, name: &str| ProjectClip {
            id,
            track_id: 1,
            start_sample: 0,
            name: name.into(),
            total_frames: 1000,
            trim_start_frames: 0,
            trim_end_frames: 0,
            audio_file: name.into(),
        };
        a.clips = vec![mk(1, "audio/a.wav")];
        b.clips = vec![mk(1, "audio/b.wav")];
        assert!(!structurally_compatible(&a, &b));
    }

    #[test]
    fn midi_notes_equal_field_wise() {
        let n = |note, vel, start, dur| MidiNote {
            note,
            velocity: vel,
            start_tick: start,
            duration_ticks: dur,
        };
        assert!(midi_notes_equal(&[], &[]));
        assert!(midi_notes_equal(
            &[n(60, 0.8, 0, 480)],
            &[n(60, 0.8, 0, 480)]
        ));
        assert!(!midi_notes_equal(
            &[n(60, 0.8, 0, 480)],
            &[n(62, 0.8, 0, 480)]
        ));
        assert!(!midi_notes_equal(
            &[n(60, 0.8, 0, 480)],
            &[n(60, 0.8, 0, 481)]
        ));
        // Different lengths are unequal.
        assert!(!midi_notes_equal(&[n(60, 0.8, 0, 480)], &[]));
    }

    #[test]
    fn drum_group_id_set_change_forces_fallback() {
        let mut a = empty_file();
        let mut b = empty_file();
        let g = |id: u64| DrumGroup {
            id,
            name: format!("g{id}"),
            color: [0, 0, 0],
            grid: 4,
            cycle: 16,
            phase: 0,
            pads: Vec::new(),
            density: 0.0,
            swing: 0.0,
            accent: 0.0,
            humanize: 0.0,
            fills: 0.0,
            style: String::new(),
            seed: 0,
        };
        a.drum_groups = vec![g(1)];
        b.drum_groups = vec![g(1), g(2)];
        assert!(!structurally_compatible(&a, &b));
    }
}
