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
//! undo works without a full re-instantiation (`reconcile::plugin_state`).

use std::collections::HashMap;

use resonance_audio::types::*;

use crate::project::{
    LoadedProject, ProjectBus, ProjectClip, ProjectFile, ProjectMidiClip, ProjectPlugin,
    ProjectTrack,
};
use crate::Resonance;

use super::reconcile::{reconcile_all_stages, LiveCarry, Origin, ReconcileCtx};
use super::serialize::build_project_file;

/// Attempt a structure-preserving replay. Returns `true` when the diff
/// path successfully drove engine + GUI to the target state; `false`
/// when the structural shape of the project differs (tracks, busses,
/// plugins, clips, drum groups, master plugins, or sections were
/// added / removed / renumbered) and the caller must fall back to the
/// full clear-and-replay pipeline.
///
/// On success the caller must skip the `ClearAll` command — there is no
/// `AllCleared` event to wait for, so neither `pending_load` nor
/// `io.restoring_undo` may be set for the `AllCleared` handler that will
/// never fire.
pub fn try_diff_replay(r: &mut Resonance, target: &LoadedProject) -> bool {
    let current = build_project_file(r);
    let target_file = &target.file;

    if !structurally_compatible(&current, target_file) {
        return false;
    }
    r.io.reconcile_trace.clear();
    let project_path = r.io.project_path.clone();
    let ctx = ReconcileCtx {
        origin: Origin::UndoDiff,
        project_dir: project_path.as_deref(),
        midi_notes: &target.midi_notes,
        plugin_states: &target.plugin_states,
        live: LiveCarry {
            project_path: project_path.as_deref(),
            derived_counter_floor: LiveCarry::derived_counter_floor(r, Origin::UndoDiff),
        },
    };

    // Every domain, in table order, by diff against `current` (ARCH-01
    // A-13): the transport / compose globals and the tempo map before any
    // entity; the track, bus and master scalars that changed, the track
    // outputs, each plugin's blob (only when the cache moved on — FU-A2b),
    // bypass and params, the registry resort; the routing edges (removals
    // first); the clips and what derives from them; the app-side content;
    // external instruments, lanes and freeze last. See
    // `docs/design/A-13-reconcile.md` for why each sits where it does.
    reconcile_all_stages(r, Some(&current), target_file, &ctx);

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
    // Not checked (ARCH-01 A-13g): entity kinds whose domains restore them
    // whole on both paths, so an added or removed one needs no `ClearAll`:
    // arrangement markers (`Markers`).
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
