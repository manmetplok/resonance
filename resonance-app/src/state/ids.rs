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
//! | clip (drawn, derived, control-created, import, split, bounce target, vocal render, …; and since D-7d recordings, cycle-record passes, live-MIDI captures and realtime bounces, through the engine's grant) | [`CLIP_ID_BASE`] | [`EntityIds::clips`] (`MediaState::ids`, D-7b); the engine's ids are blocks of it ([`IdCounter::allocate_block`], `AudioCommand::GrantIds`, D-7d) | the engine allocates nothing from its own counter any more; FU-A6a's "bump only below the base" reservation stays until D-7f deletes that counter |
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
//! [`CLIP_ID_BASE`]). The in-use scan in [`allocate_unused`] is
//! belt and braces on top of that, not the thing that makes it safe: it
//! only sees ids the app already mirrors. (D-4 and D-5 folded the track
//! and reference rows into the same "app is the only owner" shape
//! plugins, sends and busses already had.)

use resonance_audio::types::{AssetId, BusId, ClipId, TrackId};

/// Where the app's one clip-id allocator ([`EntityIds::clips`]) starts
/// (D-7b; was `DERIVED_CLIP_ID_BASE`, the start of the "derived" range
/// `ComposeState::fresh_derived_clip_id` owned). Every clip the app names
/// — drawn MIDI clips, compose lanes and drum patterns, vocal MIDI and
/// rendered vocal audio, control `notes.create_clip` / `clip.place` /
/// `clip.split`, MIDI-file imports, pool placements, bounce targets —
/// takes its id from that one counter.
///
/// Since D-7d the engine's own clips (recordings, cycle-record passes,
/// live-MIDI captures, realtime bounces) take their ids from blocks of the
/// same counter, granted ahead of time ([`Resonance::send_clip_id_grant`](crate::Resonance::send_clip_id_grant)).
/// The engine's old counter allocates nothing any more, but keeps FU-A6a's
/// rule until D-7f deletes it: an id handed to it (`LoadMidiClipDirect`,
/// `LoadClipFromWav`, …) raises it only when it is *below* the base, and
/// its STATE-08 WAV scan skips the range.
/// Before FU-A6a the engine bumped past *any* id it was handed, so the
/// first app clip at the base moved the engine to `base + 1` — the id the
/// app handed out next — and a recording then collided with the next
/// generated clip (sharing its `clip_<id>.wav`, for a vocal render).
///
/// Once the engine stops counting, the base is no longer a partition, only
/// where the allocator starts (design doc D-6 §4.1): legacy projects hold
/// engine-allocated ids below it and WAVs named after them, and starting
/// above means none of those needs a scan; `rebuild_derived_clips`' drum
/// rule (`clip.id >= base`) stays right for the pre-A-6 files it runs on.
pub const CLIP_ID_BASE: ClipId = resonance_audio::types::DERIVED_CLIP_ID_BASE;

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
    assert!(BUS_ID_BASE < CLIP_ID_BASE);
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
// D-7a / D-7b: session-monotonic counters for the spaces the app allocates
// from one counter (`docs/design/D-6-engine-created-ids.md` §4.1): pool
// assets (D-7a) and clips (D-7b). D-7e adds a third for take groups.
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

    /// Hand out the next `n` ids at once, as a range, and advance past all
    /// of them (ARCH-04 D-7d: the engine's clip-id grant). Every id in the
    /// block counts as issued from here on, used or not — an unused one is
    /// only a gap, never a candidate for [`Self::allocate`].
    pub(crate) fn allocate_block(&mut self, n: u64) -> std::ops::Range<u64> {
        let start = self.next;
        self.next += n;
        start..self.next
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

/// The app's own session-monotonic allocators (D-7a: pool assets; D-7b:
/// clips; D-7e will add take groups). Not project state and not snapshot
/// state: they live on `MediaState`, outside anything undo or a load
/// restores, and are only ever raised.
#[derive(Debug, Clone)]
pub(crate) struct EntityIds {
    /// Media-pool asset ids (`audio/asset_<id>.wav`). Seeded past every
    /// asset a loaded project holds AND every `asset_<id>.wav` a disk scan
    /// finds (`Resonance::seed_asset_ids_on_disk`) — see
    /// `update::project_io::replay::restore_pool_assets`.
    pub assets: IdCounter,
    /// The app's one clip-id allocator (D-7b), starting at
    /// [`CLIP_ID_BASE`]. **Never reset** — not by undo, `ClearAll` or a disk
    /// load (design doc D-6 §7a.2): a second project opened in the same
    /// session simply gets higher ids. That makes STATE-08 (an id that
    /// ever named a `clip_<id>.wav` is never reissued in the session) hold
    /// by construction, and it is why an undo needs no floor carried
    /// across the restore (the old `LiveCarry::derived_counter_floor`).
    ///
    /// A restore still raises it (`Resonance::restore_derived_clips`):
    /// past every restored audio and MIDI clip, every derived-map value
    /// and every take `clip_ref`, and on a disk load or a Save As into an
    /// existing bundle past every `audio/clip_<id>.wav` on disk
    /// ([`Resonance::seed_clip_ids_on_disk`], FU-A6c). A loaded project may
    /// have been saved by a session whose counter ran further than this
    /// one's.
    pub clips: IdCounter,
}

impl Default for EntityIds {
    fn default() -> Self {
        Self {
            assets: IdCounter::starting_at(1),
            clips: IdCounter::starting_at(CLIP_ID_BASE),
        }
    }
}

/// The numeric ids of every `audio/<prefix><id>.wav` under the project
/// directory `dir`. A missing or unreadable `audio/` yields none.
fn ids_on_disk(dir: &std::path::Path, prefix: &str) -> Vec<u64> {
    let Ok(entries) = std::fs::read_dir(dir.join("audio")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let digits = name.to_str()?.strip_prefix(prefix)?.strip_suffix(".wav")?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            digits.parse::<u64>().ok()
        })
        .collect()
}

impl crate::Resonance {
    /// Reserve the asset-id counter past every `audio/asset_<id>.wav` file
    /// under the project directory `dir` (D-7a, the pool's twin of
    /// [`Self::seed_clip_ids_on_disk`]). Without this an `asset_<id>.wav`
    /// an undone import left behind (or a stale backup) could be silently
    /// overwritten by the next import after a reopen — the pool itself
    /// only knows about the assets it currently holds, not an orphaned
    /// file with no asset pointing at it any more. A missing or unreadable
    /// `audio/` reserves nothing.
    pub(crate) fn seed_asset_ids_on_disk(&mut self, dir: &std::path::Path) {
        let ids: Vec<AssetId> = ids_on_disk(dir, "asset_");
        self.media.ids.assets.seed_past(ids);
    }

    /// Reserve the clip-id counter past every `audio/clip_<id>.wav` under
    /// the project directory `dir` (code review FU-A6c, folded into the
    /// one clip allocator by D-7b). Called on a disk load and on a Save As
    /// into an existing bundle.
    ///
    /// A clip's WAV outlives its clip: a backup, the autosave or an older
    /// undo state can still name it after the clip is gone from the saved
    /// file, so reissuing its id would let the next render, bounce or
    /// `PersistClipWavs` overwrite it (STATE-12). Ids below
    /// [`CLIP_ID_BASE`] (engine recordings from before D-7d) cannot raise
    /// the counter, which starts at the base. The engine's grant can
    /// predate this scan (a Save As keeps it), which is why the engine
    /// also skips a granted id whose WAV already exists
    /// (`ClipIdGrant::take_unused_wav`). Not persisted in `ProjectFile`: a monotonic value there
    /// would break the undo fixed point (A-6 §3).
    pub(crate) fn seed_clip_ids_on_disk(&mut self, dir: &std::path::Path) {
        let ids: Vec<ClipId> = ids_on_disk(dir, "clip_");
        self.media.ids.clips.seed_past(ids);
    }

    /// Grant the engine the next [`CLIP_GRANT_SIZE`](resonance_audio::types::CLIP_GRANT_SIZE) clip ids (ARCH-04
    /// D-7d, design doc D-6 §4.2): the ids its recordings, cycle-record
    /// passes, live-MIDI captures and realtime bounces are created under.
    /// They come from [`EntityIds::clips`] and count as issued now, so no
    /// app-allocated clip can ever land on one, used or not.
    ///
    /// Sent at startup ([`Resonance::new`](crate::Resonance::new)), as the
    /// last command of every disk-load replay (the `ClipIdGrant` reconcile
    /// domain — `ClearAll` revoked the old grant, and the counter is
    /// seeded past the loaded project by then), and on
    /// `AudioEvent::IdGrantLow` ([`Self::refill_clip_id_grant`]).
    pub(crate) fn send_clip_id_grant(&mut self) {
        use resonance_audio::types::{AudioCommand, IdGrantBlocks, CLIP_GRANT_SIZE};
        let clips = self.media.ids.clips.allocate_block(CLIP_GRANT_SIZE);
        let _ = self.engine.send(AudioCommand::GrantIds(IdGrantBlocks::clips(clips)));
    }

    /// `AudioEvent::IdGrantLow`: top the engine's grant up — unless a load
    /// is in flight. A low report raised before that load's `ClearAll`
    /// would otherwise grant from a counter not yet seeded past the
    /// incoming project, which may hold those very ids; the replay ends
    /// with its own grant instead.
    pub(crate) fn refill_clip_id_grant(&mut self) {
        if self.io.loading {
            return;
        }
        self.send_clip_id_grant();
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

    /// Create the fresh-session default track — every new session's
    /// "Track 1" — from the app side, synchronously, as part of
    /// construction (`Resonance::new`).
    ///
    /// Until FU-D4a the engine thread created this track itself,
    /// unprompted, as a literal id 1, right before its command loop ever
    /// read anything (`resonance-audio/src/engine/thread/mod.rs`, since
    /// removed). That raced the app's own counter, which independently
    /// also starts at 1: a GUI "Add Track" handled before the app had
    /// mirrored that unprompted `TrackAdded` echo called
    /// [`Self::allocate_track_id`], got id 1 too, and the engine refused
    /// the resulting `AddTrack` as a collision with the track it had
    /// already silently created — a click that visibly did nothing but
    /// raise an error banner.
    ///
    /// Routing the default track through the app's own allocator instead
    /// closes the window outright rather than narrowing it: this runs
    /// synchronously inside `Resonance::new`, before iced's event loop
    /// can deliver any message (GUI or control), so this call is
    /// unconditionally the *first* `allocate_track_id` call anywhere —
    /// nothing can ever again race it for id 1, the same way no two GUI
    /// clicks can race each other (each `update` call runs to completion
    /// before the next message is dispatched).
    pub(crate) fn send_startup_default_track(&mut self) {
        let id = self.allocate_track_id();
        let _ = self
            .engine
            .send(resonance_audio::types::AudioCommand::AddTrack { id, name: None });
    }
}
