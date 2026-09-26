//! The app side of the entity-id partition, in one place (ARCH-04 A4-2).
//!
//! Two owners hand out entity ids today. The engine allocates the ids of
//! everything the GUI creates without a hint (tracks, clips, assets, take
//! groups — counters in `resonance-audio/src/engine/thread/mod.rs`), and
//! the app allocates the ids it needs *synchronously* — a control reply
//! that must carry the id before the engine echoes, a sub-track the
//! mirror names up front, a derived clip the compose model owns outright.
//! Each app-owned space starts at a base far above anything the engine's
//! counters reach, so the two owners never meet; this file is where those
//! bases live and where the "is it free?" loop every allocator runs is
//! written once.
//!
//! Who gets what:
//!
//! | Space | App base | Allocator | Engine rule on a hint |
//! |---|---|---|---|
//! | track + track group (one space) | [`SUB_TRACK_ID_BASE`] | [`Resonance::allocate_track_id`](crate::Resonance::allocate_track_id) | counter bumps only for hints *below* the base |
//! | bus | [`BUS_ID_BASE`] | `TrackRegistry::allocate_bus_id` | none — the engine has no bus counter left (D-3); this base is an app/control-API-only convention (see below) |
//! | clip (derived, control-created, vocal render, …) | [`DERIVED_CLIP_ID_BASE`] | [`ComposeState::fresh_derived_clip_id`](crate::compose::ComposeState::fresh_derived_clip_id) | counter bumps only for ids *below* the base (FU-A6a) |
//! | missing reference | [`MISSING_REFERENCE_ID_BASE`] | local counter in `replay::restore` | never sees one (app-only) |
//!
//! **Plugin instance ids and aux-send ids are no longer a partition**
//! (ARCH-04 D-1, D-2 respectively): the app is the ONLY allocator for
//! each (`Resonance::allocate_plugin_id` in `state/plugin_index.rs`,
//! `AuxSendState::allocate_send_id` in `state/aux_sends.rs`), the engine
//! has no counter of its own left for either, and every add — GUI,
//! control API, presets, templates, project-load replay — carries a
//! concrete id the engine either honours or refuses
//! (`EngineErrorKind::Internal`) if it collides with a live entity. There
//! is no base to name for either because there is no neighbouring range
//! to stay clear of. Aux sends keep one wrinkle plugins don't:
//! `AudioCommand::SetAuxSend` legitimately reuses a live id on every edit
//! (level drag, re-route, toggle), so the create path is a separate
//! command, `AddAuxSend`, and only THAT one is refused on a collision.
//!
//! **Busses also lost their engine-side counter** (ARCH-04 D-3,
//! `TrackRegistry::allocate_bus_id` is the only allocator, and
//! `AudioCommand::AddBus` is refused rather than honoured on a collision)
//! but — unlike plugins and sends — [`BUS_ID_BASE`] stays. The reason has
//! nothing to do with the engine: `resonance-audio`'s `ctx.tracks` and
//! `ctx.busses` are separate maps that never confuse a track id for a bus
//! id. It is `song.summary` / `song.tracks` (ba doc #265) that cannot:
//! both list tracks and busses in ONE `TrackKind`-tagged sequence,
//! addressed by this same raw id (`view_model::track::track_summaries`
//! appends bus rows after track rows). A fresh bus landing on a live
//! track's id would not error — it would silently make that bus
//! unreachable through the control API, because the track's entry, which
//! comes first, wins any lookup by id. Tracks (engine-allocated ones
//! count from 1, `SUB_TRACK_ID_BASE`-range ones from 1e9) can in
//! principle grow into any range over a long enough session, so this is
//! the same "practically safe, not literally unbounded" argument
//! [`SUB_TRACK_ID_BASE`] itself already rests on — discovered the hard
//! way when folding this base away made `bus.create` hand out id 1 in a
//! test that had already created track id 1, and `song.summary` reported
//! the bus as the track.
//!
//! **Reference ids lost their engine-side counter too** (ARCH-04 D-5):
//! [`crate::reference::ReferenceState::alloc_engine_id`] is the only
//! allocator, and `AudioCommand::LoadReferenceTrack`'s `id` is mandatory
//! — the engine refuses a collision (`EngineErrorKind::Internal`) rather
//! than replacing the live entry, same shape as plugins/sends/busses.
//! [`MISSING_REFERENCE_ID_BASE`] stays, but for a different reason than
//! [`BUS_ID_BASE`]: it isn't there to stay clear of an engine counter —
//! there is none — it separates the app's own two reference sub-spaces,
//! a live engine-registered id from `next_engine_id` and a `Missing`
//! entry's id (which the engine never hears about), so a missing entry
//! can never collide with a later real load.
//!
//! Markers, automation lanes and grooves are app-only spaces with their
//! own counters; the engine never hears their ids. This includes
//! reference *comparison* markers (`AddRefMarker`, FU-A5a) as well as
//! arrangement/timeline markers ([`ArrangementMarkers`](crate::state::markers::ArrangementMarkers))
//! — the engine has never had a counter for either.
//!
//! The last column is what makes each of the REMAINING engine-agreed
//! ranges (track, clip) a real partition rather than a convention: the
//! engine takes an app-range hint but never moves its own counter for it,
//! so an engine allocation (`id_hint: None`) can never land on an id the
//! app holds — including a track group's, which the engine never hears
//! about. Until ARCH-04 A4-1 the track path bumped past *any* hint, so
//! one control `track.add` followed by a Cmd-G group and a GUI "Add
//! track" put a track on the group's id; the clip paths did the same
//! until FU-A6a (see [`DERIVED_CLIP_ID_BASE`]). The in-use scan in
//! [`allocate_unused`] is belt and braces on top of the split, not the
//! thing that makes it safe: it only sees ids the app already mirrors.
//! (D-4 will fold the track row into the same "app is the only owner"
//! shape plugins, sends, busses and references already have.)

use resonance_audio::types::{BusId, TrackId};

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
// the range: it is session-monotonic (undo never lowers it), and a load
// reserves past every restored clip id in the range and every
// derived-range `audio/clip_<id>.wav` in the bundle (as does a Save As
// into an existing one, FU-A6c), so with the engine kept out of it there
// is no second allocator to skip over.
pub use resonance_audio::types::{DERIVED_CLIP_ID_BASE, SUB_TRACK_ID_BASE};

/// First bus id the app allocates — every bus now, GUI or control alike
/// (ARCH-04 D-3). Not an engine-agreed range any more (the engine has no
/// bus counter to keep clear of it); see the module doc for why it stays
/// anyway — `song.summary` / `song.tracks` list tracks and busses in one
/// id-addressed sequence, and a bus sharing a live track's raw id would
/// be shadowed by it there.
pub const BUS_ID_BASE: BusId = 2_000_000_000;

/// First id handed to a reference track whose file is missing on load,
/// so it can be listed without ever being registered with the engine.
/// Kept disjoint from [`ReferenceState::next_engine_id`](crate::reference::ReferenceState::next_engine_id),
/// the app's own allocator for every *live* reference id (ARCH-04 D-5:
/// the engine has no reference-id counter of its own left to stay clear
/// of) — a `LoadReferenceTrack` id comes from that counter, which starts
/// at 1, so it would take ~1e9 loads in one session for it to reach this
/// base and risk landing on a missing entry's id.
pub const MISSING_REFERENCE_ID_BASE: u32 = 1_000_000_000;

// The convention the bases follow, pinned so a moved base fails to
// compile rather than silently overlapping a neighbour: every app range
// sits above the ids it must stay clear of.
const _: () = {
    assert!(SUB_TRACK_ID_BASE < BUS_ID_BASE);
    assert!(BUS_ID_BASE < DERIVED_CLIP_ID_BASE);
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
