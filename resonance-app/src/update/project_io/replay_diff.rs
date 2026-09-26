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
//! removed or changed type, a clip was inserted), it returns `false` and
//! the caller falls back to the full clear-and-replay pipeline. App-side
//! entities restored whole on both paths (sections, drum patterns, track
//! groups, markers) are not part of the shape (A-13g), and neither are
//! busses and plugin instances, which the diff arms add, remove and
//! reorder one at a time (A-13h).
//!
//! Plugin parameter restores: a snapshot's state blob is re-sent with
//! `LoadPluginState` only when the live cache has moved on since the
//! snapshot; either way the snapshot's param values are then driven
//! explicitly (all of them the blob may have reset, or just those that
//! differ from the live mirror when no blob was pushed), so plugin param
//! undo works without a full re-instantiation (`reconcile::plugin_state`).

use std::collections::HashMap;

use resonance_audio::types::*;

use crate::project::{LoadedProject, ProjectClip, ProjectFile, ProjectMidiClip, ProjectTrack};
use crate::Resonance;

use super::reconcile::{reconcile_all_stages, LiveCarry, Origin, ReconcileCtx};
use super::serialize::build_project_file;

/// Attempt a structure-preserving replay. Returns `true` when the diff
/// path successfully drove engine + GUI to the target state; `false`
/// when the structural shape of the project differs (tracks or clips were
/// added / removed / renumbered) and the caller must fall back to the full
/// clear-and-replay pipeline. App-side entities (sections, drum patterns,
/// track groups, markers) may differ: their domains restore them whole
/// (A-13g); so may busses and plugin chains (A-13h).
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
    // entity; the routing edges, plugin instances and busses the target
    // lacks, removed in that order (A-13h); the track, bus and master
    // scalars that changed and the busses and plugins the live state lacks,
    // the track outputs, each plugin's blob (a live one only when the cache
    // moved on — FU-A2b), bypass and params, the registry resort and chain
    // order; the new or changed routing edges; the clips and what derives
    // from them; the app-side content; external instruments, lanes and
    // freeze last. See
    // `docs/design/A-13-reconcile.md` for why each sits where it does.
    reconcile_all_stages(r, Some(&current), target_file, &ctx);

    true
}

// =====================================================================
// Structural comparison
// =====================================================================

/// True iff the two project files have the same set of structural
/// identifiers — track ids, clip ids — arranged into the same
/// parent-child shape. Pure ordering of the
/// outer collections is normalised via id-sort before comparison so a
/// re-ordering by `.order` alone does NOT force the slow path.
pub fn structurally_compatible(a: &ProjectFile, b: &ProjectFile) -> bool {
    // Track set, sub-track linkage, track type.
    if !track_set_matches(&a.tracks, &b.tracks) {
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
    // Not checked (ARCH-01 A-13g): entity kinds whose domains restore them
    // whole on both paths, so adding or removing one needs no `ClearAll`.
    // Section definitions and placements (`ComposeSections`; what a
    // placement generates is clips, gated above, and the maps keyed by it
    // are rebuilt in `Clips` from the target), drum patterns and the legacy
    // flat drum-group list they promote (`DrumPatterns`, whose diff arm
    // empties the bank for an empty target), track groups (`TrackGroups`),
    // arrangement markers (`Markers`). Nor (A-13h) busses and plugin
    // chains — track, bus and master: `RoutingRemovals` drops the edges
    // `b` lacks, `EntityRemovals` the plugin instances it does not keep
    // (`entities::kept_plugins`: same id, chain and `.clap` identity) and
    // the busses it lacks, the entity domains add what `a` lacks,
    // `PluginState` treats each added instance as a load does, and
    // `EntityOrder` moves every chain into `b`'s order (`MovePlugin*`).
    // What is left is what the diff arms cannot add or remove yet: tracks
    // and clips (A-13i).
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
    }
    true
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
