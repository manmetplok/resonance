//! Diff-based undo/redo replay — the cheap alternative to the
//! `ClearAll → AllCleared → replay_loaded_project` round-trip.
//!
//! The full replay tears every plugin instance down and re-instantiates
//! it — expensive, audible (the plugin chain is briefly silent), and
//! wasted on an undo, which almost always changes a few things.
//!
//! [`try_diff_replay`] drives the engine surgically — one engine command
//! per changed scalar, one add or remove per entity that differs — and
//! rebuilds GUI state in place. Since A-13i it accepts every pair of
//! snapshots: app-side entities are restored whole (A-13g), busses and
//! plugin instances (A-13h), tracks and clips (A-13i) are added, removed
//! and reordered one at a time. `structurally_compatible` is left, always
//! true, for A-13j to delete with the undo's `ClearAll` fallback.
//!
//! Plugin parameter restores: a snapshot's state blob is re-sent with
//! `LoadPluginState` only when the live cache has moved on since the
//! snapshot; either way the snapshot's param values are then driven
//! explicitly (all of them the blob may have reset, or just those that
//! differ from the live mirror when no blob was pushed), so plugin param
//! undo works without a full re-instantiation (`reconcile::plugin_state`).

use resonance_audio::types::*;

use crate::project::{LoadedProject, ProjectFile};
use crate::Resonance;

use super::reconcile::{reconcile_all_stages, LiveCarry, Origin, ReconcileCtx};
use super::serialize::build_project_file;

/// Attempt a diff replay. Returns `true` when the diff path drove the
/// engine and the GUI to the target state — since A-13i, always (`false`
/// would send the caller down the full clear-and-replay pipeline).
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

/// Whether the diff path can restore `b` over `a`. Always, since A-13i.
pub fn structurally_compatible(_a: &ProjectFile, _b: &ProjectFile) -> bool {
    // Nothing is checked any more (ARCH-01 A-13g..i): every domain restores
    // any difference itself. App-side entities (sections, drum patterns,
    // track groups, markers) are restored whole on both paths (A-13g).
    // Busses and plugin chains (A-13h): `RoutingRemovals` drops the edges
    // `b` lacks, `EntityRemovals` the plugin instances it does not keep
    // (`entities::kept_plugins`: same id, chain and `.clap` identity) and
    // the busses it lacks, the entity domains add what `a` lacks,
    // `PluginState` treats each added instance as a load does, and
    // `EntityOrder` moves every chain into `b`'s order (`MovePlugin*`).
    // Tracks (A-13i): `entities::kept_tracks` keeps a track whose id, type
    // and sub-track link match (and whose parent is kept); `EntityRemovals`
    // removes every other track of `a` (sub-tracks first), `Tracks` adds
    // every other track of `b` (parents first) as a load does, and the
    // domains that treat a track differently after a `ClearAll` (outputs,
    // plugin state, external instruments, freeze, group macros) do so per
    // fresh track. Clips (A-13i): `clips::kept_audio_clips` /
    // `kept_midi_clips` keep a clip on a kept track (an audio clip with the
    // same WAV and length); `ClipRemovals` deletes every other clip of `a`
    // before any track goes, and the clip domains load every other clip of
    // `b` as a load does. Left in place, always true, until A-13j deletes
    // it with the undo's `ClearAll` fallback.
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
