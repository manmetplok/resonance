//! The app side of the entity-id partition, in one place (ARCH-04 A4-2).
//!
//! Two owners hand out entity ids today. The engine allocates the ids of
//! everything the GUI creates without a hint (tracks, busses, plugins,
//! sends, clips, assets, take groups — counters in
//! `resonance-audio/src/engine/thread/mod.rs`), and the app allocates the
//! ids it needs *synchronously* — a control reply that must carry the id
//! before the engine echoes, a sub-track the mirror names up front, a
//! derived clip the compose model owns outright. Each app-owned space
//! starts at a base far above anything the engine's counters reach, so
//! the two owners never meet; this file is where those bases live and
//! where the "is it free?" loop every allocator runs is written once.
//!
//! Who gets what:
//!
//! | Space | App base | Allocator | Engine rule on a hint |
//! |---|---|---|---|
//! | track + track group (one space) | [`SUB_TRACK_ID_BASE`] | [`Resonance::allocate_track_id`](crate::Resonance::allocate_track_id) | counter bumps only for hints *below* the base |
//! | bus | [`RETURN_BUS_ID_BASE`] | `TrackRegistry::allocate_return_bus_id` | counter bumps only for hints *below* the base |
//! | aux send | [`CONTROL_SEND_ID_BASE`] | `AuxSendState::allocate_control_send_id` | counter bumps only for hints *below* the base |
//! | clip (derived, control-created, vocal render, …) | [`DERIVED_CLIP_ID_BASE`] | [`ComposeState::fresh_derived_clip_id`](crate::compose::ComposeState::fresh_derived_clip_id) | counter bumps only for ids *below* the base (FU-A6a) |
//! | missing reference | [`MISSING_REFERENCE_ID_BASE`] | local counter in `replay::restore` | never sees one (app-only) |
//!
//! **Plugin instance ids are no longer a partition** (ARCH-04 D-1): the
//! app is the ONLY allocator (`Resonance::allocate_plugin_id`, in
//! `state/plugin_index.rs`), the engine has no counter of its own left,
//! and every add — GUI, control API, presets, templates, project-load
//! replay — carries a concrete id the engine either honours or refuses
//! (`EngineErrorKind::Internal`) if it collides with a live instance.
//! There is no base to name because there is no neighbouring range to
//! stay clear of.
//!
//! Markers, automation lanes and grooves are app-only spaces with their
//! own counters; the engine never hears their ids.
//!
//! The last column is what makes each of the REMAINING ranges a real
//! partition rather than a convention: the engine takes an app-range
//! hint but never moves its own counter for it, so an engine allocation
//! (`id_hint: None`) can never land on an id the app holds — including a
//! track group's, which the engine never hears about. Until ARCH-04 A4-1
//! the track, bus and send paths bumped past *any* hint, so one control
//! `track.add` followed by a Cmd-G group and a GUI "Add track" put a
//! track on the group's id; the clip paths did the same until FU-A6a
//! (see [`DERIVED_CLIP_ID_BASE`]). The in-use scan in [`allocate_unused`] is
//! belt and braces on top of the split, not the thing that makes it
//! safe: it only sees ids the app already mirrors. (D-2 through D-5 fold
//! the send, bus, track and reference rows into the same "app is the
//! only owner" shape this row already is.)

use resonance_audio::types::TrackId;

// The four bases the engine also honours are defined beside the
// engine's id types (`resonance-audio/src/types/mod.rs`): the partition
// only works when both sides agree on it, and the engine's add and load
// paths bump their counters only for ids below these.
//
// `DERIVED_CLIP_ID_BASE` is where `ComposeState::fresh_derived_clip_id`
// starts: the clips the app names before the engine echoes (compose
// lanes, drum patterns, vocal MIDI and rendered vocal audio, control
// `notes.create_clip`, imports and bounce targets that go through it).
// Until FU-A6a the engine bumped `next_clip_id` past *any* id handed to
// `LoadMidiClipDirect` / `LoadClipFromWav` (and past every
// `audio/clip_<id>.wav` its STATE-08 scan found), so the first derived
// clip at the base moved the engine to `base + 1` — the id the derived
// counter handed out next — and a drawn clip, a recording or an import
// then collided with the next generated clip (sharing its
// `clip_<id>.wav`, for a vocal render). Nothing but the counter checks
// the range: it is session-monotonic (undo never lowers it) and a load
// reserves past every restored clip id in the range, so with the engine
// kept out of it there is no second allocator to skip over.
pub use resonance_audio::types::{
    CONTROL_SEND_ID_BASE, DERIVED_CLIP_ID_BASE, RETURN_BUS_ID_BASE, SUB_TRACK_ID_BASE,
};

/// First id handed to a reference track whose file is missing on load,
/// so it can be listed without ever being registered with the engine.
/// The engine allocates reference ids sequentially from 1, so it would
/// take ~1e9 loads in one session to reach this.
pub const MISSING_REFERENCE_ID_BASE: u32 = 1_000_000_000;

// The convention the bases follow, pinned so a moved base fails to
// compile rather than silently overlapping a neighbour: every app range
// sits above the engine's counters, and the ranges that share an id
// space with each other are ordered.
const _: () = {
    assert!(SUB_TRACK_ID_BASE < RETURN_BUS_ID_BASE);
    assert!(RETURN_BUS_ID_BASE == CONTROL_SEND_ID_BASE);
    assert!(CONTROL_SEND_ID_BASE < DERIVED_CLIP_ID_BASE);
};

/// Hand out the next id from `next`, skipping any candidate `in_use`
/// reports as taken. The one skip loop behind every app-side allocator:
/// a counter restored from a loaded project, an engine echo that landed
/// in the app's range, or a caller that built its own ids must never make
/// an allocator return an id something already holds.
pub fn allocate_unused(next: &mut u64, in_use: impl Fn(u64) -> bool) -> u64 {
    loop {
        let candidate = *next;
        *next += 1;
        if !in_use(candidate) {
            return candidate;
        }
    }
}

impl crate::Resonance {
    /// Allocate a fresh app-side track id — for a sub-track, a bounce
    /// target, a control-API add or a track group — skipping every id a
    /// track *or a group* already holds. Tracks and groups share one id
    /// space, so the group registry is part of the in-use check (code
    /// review FU-A1c): without it the counter bump on project load was
    /// the only thing keeping a new track off a saved group's id.
    pub(crate) fn allocate_track_id(&mut self) -> TrackId {
        let tracks = &self.registry.tracks;
        let groups = &self.track_groups;
        allocate_unused(&mut self.registry.next_sub_track_id, |id| {
            tracks.iter().any(|t| t.id == id) || groups.get_group(id).is_some()
        })
    }
}
