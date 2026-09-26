//! The app side of the entity-id partition, in one place (ARCH-04 A4-2).
//!
//! Two owners once hand out entity ids here. The engine allocates the ids
//! of what's left that the GUI creates without a hint (clips, take groups —
//! counters in `resonance-audio/src/engine/thread/mod.rs`), and the app
//! allocates the ids it needs *synchronously* — a control reply that must
//! carry the id before the engine echoes, a sub-track the mirror names up
//! front, a derived clip the compose model owns outright, a pool asset
//! (D-7a) about to be imported. Each app-owned space with a base starts far
//! above anything the engine's own counters reach, so the two owners never
//! meet there; this file is where those bases live and where the "is it
//! free?" loop every allocator runs is written once. A space with no base
//! (see the table) has no engine counter to stay clear of at all.
//!
//! Who gets what:
//!
//! | Space | App base | Allocator | Engine rule on a hint |
//! |---|---|---|---|
//! | bus | [`BUS_ID_BASE`] | `TrackRegistry::allocate_bus_id` | none — the engine has no bus counter left (D-3); this base is an app/control-API-only convention (see below) |
//! | clip (derived, control-created, vocal render, …) | [`DERIVED_CLIP_ID_BASE`] | [`ComposeState::fresh_derived_clip_id`](crate::compose::ComposeState::fresh_derived_clip_id) | counter bumps only for ids *below* the base (FU-A6a) |
//! | missing reference | [`MISSING_REFERENCE_ID_BASE`] | local counter in `replay::restore` | never sees one (app-only) |
//! | pool asset (D-7a) | none — the app is the ONLY allocator | [`EntityIds::assets`] (`MediaState::ids`) | none left — the engine invents no asset ids any more; `ImportAudioToPool` carries a mandatory id per file and the engine refuses (`ImportFailed`, `create_new`) rather than overwrite a colliding `asset_<id>.wav` |
//!
//! **Plugin instance ids, aux-send ids and track ids are no longer a
//! partition** (ARCH-04 D-1, D-2, D-4 respectively): the app is the ONLY
//! allocator for each (`Resonance::allocate_plugin_id` in
//! `state/plugin_index.rs`, `AuxSendState::allocate_send_id` in
//! `state/aux_sends.rs`, [`Resonance::allocate_track_id`] right below),
//! the engine has no counter of its own left for any of the three, and
//! every add — GUI, control API, presets, templates, project-load replay —
//! carries a concrete id the engine either honours or refuses
//! (`EngineErrorKind::Internal`) if it collides with a live entity. There
//! is no base to name for plugins or sends because there is no
//! neighbouring range to stay clear of; [`Resonance::allocate_track_id`]'s
//! doc comment below covers why tracks are not quite that simple. Aux
//! sends keep one wrinkle plugins and tracks don't: `AudioCommand::SetAuxSend`
//! legitimately reuses a live id on every edit (level drag, re-route,
//! toggle), so the create path is a separate command, `AddAuxSend`, and
//! only THAT one is refused on a collision.
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
//! comes first, wins any lookup by id. Track ids now start at 1 and grow
//! with usage (same as the engine's own counter used to, before ARCH-04
//! D-4 folded it away) — nowhere near [`BUS_ID_BASE`] in any session that
//! will ever run, but [`Resonance::allocate_track_id`] still
//! `debug_assert`s it, the same "practically safe, not literally
//! unbounded" argument the old `SUB_TRACK_ID_BASE` rested on, discovered
//! the hard way when folding THAT base away made `bus.create` hand out id
//! 1 in a test that had already created track id 1, and `song.summary`
//! reported the bus as the track.
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
//! [`BUS_ID_BASE`] is the last real partition left in this file: the
//! engine has nothing to keep it clear of any more (see above), but the
//! app still never allocates a track id at or above it. Before ARCH-04
//! A4-1 the track path bumped past *any* hint, so one control `track.add`
//! followed by a Cmd-G group and a GUI "Add track" put a track on the
//! group's id; the clip paths did the same until FU-A6a (see
//! [`DERIVED_CLIP_ID_BASE`]). The in-use scan in [`allocate_unused`] is
//! belt and braces on top of that, not the thing that makes it safe: it
//! only sees ids the app already mirrors. (D-4 and D-5 folded the track
//! and reference rows into the same "app is the only owner" shape
//! plugins, sends and busses already had.)

use resonance_audio::types::{AssetId, BusId, TrackId};

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
pub use resonance_audio::types::DERIVED_CLIP_ID_BASE;

/// First bus id the app allocates — every bus now, GUI or control alike
/// (ARCH-04 D-3). Not an engine-agreed range any more (the engine has no
/// bus counter to keep clear of it); see the module doc for why it stays
/// anyway — `song.summary` / `song.tracks` list tracks and busses in one
/// id-addressed sequence, and a bus sharing a live track's raw id would
/// be shadowed by it there. [`Resonance::allocate_track_id`] is the one
/// allocator that still has to know about this base, even though it has
/// no base of its own any more (ARCH-04 D-4): it `debug_assert`s every id
/// it hands out stays below this one.
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

// The convention the one remaining base follows, pinned so a moved base
// fails to compile rather than silently overlapping a neighbour: every
// app range sits above the ids it must stay clear of.
const _: () = {
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

// ---------------------------------------------------------------------------
// D-7a: an id space with no engine counter left to stay clear of at all —
// the app is the ONLY allocator, so there is no base to name, only a
// session-monotonic counter (`docs/design/D-6-engine-created-ids.md` §4.1).
// `IdCounter` is deliberately generic: D-7b promotes the derived-clip
// allocator (`ComposeState::next_derived_clip_id`) into a second field of
// `EntityIds` the same shape, and D-7e a third for take groups.
// ---------------------------------------------------------------------------

/// A session-monotonic id counter: never rewound by undo, a load, or
/// `ClearAll` — only ever raised, by [`Self::allocate`] or
/// [`Self::seed_past`]. This is what makes STATE-08 (an id that ever named
/// a file must never be reissued in the session) trivially true for a space
/// with exactly one allocator: nothing else can ever hand out one of its
/// ids, so "never rewound" is the whole guarantee.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IdCounter {
    next: u64,
}

impl IdCounter {
    /// A counter that will hand out `start` first.
    pub(crate) const fn starting_at(start: u64) -> Self {
        Self { next: start }
    }

    /// Hand out the next id and advance past it.
    pub(crate) fn allocate(&mut self) -> u64 {
        let id = self.next;
        self.next += 1;
        id
    }

    /// Raise the counter past every id in `ids` (each treated as an id
    /// that already exists somewhere — on disk, in a loaded project — so
    /// the next [`Self::allocate`] must not repeat it). A no-op for an
    /// empty iterator or one whose ids are already below the counter.
    pub(crate) fn seed_past(&mut self, ids: impl IntoIterator<Item = u64>) {
        if let Some(max) = ids.into_iter().max() {
            self.next = self.next.max(max.saturating_add(1));
        }
    }
}

/// The app's own allocators for spaces the engine invents no ids for at
/// all (D-7a start: just the pool's asset ids; D-7b/D-7e add clips and take
/// groups here as they promote their allocators out of `ComposeState` /
/// the engine).
#[derive(Debug, Clone)]
pub(crate) struct EntityIds {
    /// Media-pool asset ids (`audio/asset_<id>.wav`). Seeded past every
    /// asset a loaded project holds AND every `asset_<id>.wav` a disk scan
    /// finds (`Resonance::seed_asset_ids_on_disk`) — see
    /// `update::project_io::replay::restore_pool_assets`.
    pub assets: IdCounter,
}

impl Default for EntityIds {
    fn default() -> Self {
        Self {
            assets: IdCounter::starting_at(1),
        }
    }
}

impl crate::Resonance {
    /// Reserve the asset-id counter past every `audio/asset_<id>.wav` file
    /// under the project directory `dir` (D-7a, mirrors
    /// [`ComposeState::reserve_derived_clip_ids_on_disk`](crate::compose::ComposeState::reserve_derived_clip_ids_on_disk)
    /// for the pool). Without this an `asset_<id>.wav` an undone import
    /// left behind (or a stale backup) could be silently overwritten by
    /// the next import after a reopen — the pool itself only knows about
    /// the assets it currently holds, not an orphaned file with no asset
    /// pointing at it any more. A missing or unreadable `audio/` reserves
    /// nothing.
    pub(crate) fn seed_asset_ids_on_disk(&mut self, dir: &std::path::Path) {
        let Ok(entries) = std::fs::read_dir(dir.join("audio")) else {
            return;
        };
        self.media.ids.assets.seed_past(entries.flatten().filter_map(|e| {
            let name = e.file_name();
            let digits = name.to_str()?.strip_prefix("asset_")?.strip_suffix(".wav")?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            digits.parse::<AssetId>().ok()
        }));
    }
}

impl crate::Resonance {
    /// Allocate a fresh app-side track id — for the plain GUI "Add Track"
    /// (and its instrument/vocal/external siblings), a sub-track, a
    /// bounce target, a control-API add, or a track group — skipping
    /// every id a track *or a group* already holds. Tracks and groups
    /// share one id space, so the group registry is part of the in-use
    /// check (code review FU-A1c): without it the counter bump on
    /// project load was the only thing keeping a new track off a saved
    /// group's id.
    ///
    /// Since ARCH-04 D-4 this is the ONLY track-id allocator: the engine
    /// has no counter of its own left (`AudioCommand::AddTrack` /
    /// `AddInstrumentTrack` / `AddVocalTrack` carry a mandatory `id`, not
    /// a hint, and are refused with `EngineErrorKind::Internal` on a
    /// collision), so there is no neighbouring engine range to stay clear
    /// of — the counter starts at 1, the same as the engine's own used
    /// to, so a fresh project's first "Add Track" still lands on id 1
    /// (demo/template projects load unchanged either way, since a loaded
    /// project's ids come from the file, not this counter).
    ///
    /// The one range this DOES still have to respect is
    /// [`BUS_ID_BASE`](super::ids::BUS_ID_BASE): tracks and busses are
    /// listed together in `song.summary` / `song.tracks`' one
    /// id-addressed sequence, so a track id landing at or above it would
    /// be indistinguishable from a bus there. In any session that will
    /// ever actually run this is not a real limit — the same
    /// "practically safe, not literally unbounded" argument the old
    /// `SUB_TRACK_ID_BASE` rested on — so this is a `debug_assert`, not a
    /// clamp: silently wrapping (or refusing) at a boundary this far away
    /// would be new, untested behaviour a real session should never
    /// exercise.
    pub(crate) fn allocate_track_id(&mut self) -> TrackId {
        let tracks = &self.registry.tracks;
        let groups = &self.track_groups;
        let id = allocate_unused(&mut self.registry.next_track_id, |id| {
            tracks.iter().any(|t| t.id == id) || groups.get_group(id).is_some()
        });
        debug_assert!(
            id < BUS_ID_BASE,
            "allocate_track_id: id {id} grew into the bus range ({BUS_ID_BASE}) — \
             song.summary/song.tracks can no longer tell this track from a bus"
        );
        id
    }
}
