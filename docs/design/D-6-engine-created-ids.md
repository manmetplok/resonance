# D-6: ids the engine creates at times the app can't pre-decide

Design for `refactor-intent.md` Epic D item 3 (`arch-migration-plan.md`
ARCH-04 A4-4 step 6). Written against master `fde89f24`. Design only; the
implementation is D-7 (§6).

**Recommendation in one paragraph.** There should be one app-owned clip-id
allocator, which is today's `fresh_derived_clip_id`, promoted out of
`ComposeState` and never rewound. Every create the app decides (drawn MIDI
clip, import, split, bounce, compose, control) takes a mandatory id from it.
For the moments the app *cannot* decide (a record run's first buffer, each
loop-seam pass, a live-MIDI clip opened by the first note, a new take lane),
the app keeps the engine supplied with a **standing grant** of ids
(`AudioCommand::GrantIds`). The engine draws from that grant in order, asks
for a refill below a low-water mark (`AudioEvent::IdGrantLow`), and never
counts on its own. `ClearAll` revokes the grant, and every replay ends by
sending a fresh one. Asset ids need no grant: an import knows how many files
it has, so the app allocates one id per file up front. Take-group ids come
from the same grant under their own counter. The plan's per-command block
(`ArmTrack { clip_ids }`) is rejected in §3 because it has no answer for
unbounded loop passes, and none for live-MIDI capture during plain Play,
which needs no Record command at all.

---

## 1. Inventory: every id the engine still invents

Found with `grep -rn 'next_[a-z_]*_id' resonance-audio/src/engine` (65 hits)
plus every `AudioCommand` that creates an entity without an app-supplied id.
Reference and marker ids (`ReferencePlayer::next_ref_id`) are D-5 and are
left out here.

### 1a. Allocation sites (the engine *creates* an id)

| # | Id space | Site | Trigger | When | Count per trigger | Echo | App consumer |
|---|---|---|---|---|---|---|---|
| C1 | clip | `engine/midi/clips.rs::handle_create_midi_clip` | `AudioCommand::CreateMidiClip` (compose canvas "create MIDI clip in section", `update/compose/section.rs:96`) | on command | 1 | `MidiClipCreated` | `engine_events/midi.rs::clip_created` (mirrors into `r.midi_clips`) |
| C2 | clip | `engine/clips.rs::handle_import_clip` | `AudioCommand::ImportClip` | on command | 1 | `ClipImported` | `engine_events/clips.rs::imported`. **No app sender left.** Only `resonance-audio/tests/engine/loop_record_takes.rs` uses it; timeline imports go through the pool (`place_clip` → `fresh_derived_clip_id`). Dead |
| C3 | clip | `engine/transport.rs:~291` (`begin_recording_stream`) | `AudioCommand::Record`, and `BounceTrackRealtimeToAudio` (same stream path) | record start, after precount | 1 per armed track that captures audio | `RecordingFinished` (non-loop, at stop) or `TakeCaptured` (loop, pass 0) | `engine_events/clips.rs::recording_finished` / `engine_events/takes.rs::take_captured` |
| C4 | clip | `recording.rs::roll_audio_pass` (`*next_clip_id += 1` at `reopen`) | loop seam during cycle record | every pass boundary | 1 per capturing track per pass, **unbounded**: passes = run length ÷ loop length | `TakeCaptured { content: Audio { clip_ref } }` for the *finished* pass | `take_captured` (`r.take_groups`, then reads `clip_<ref>.wav` peaks) |
| C5 | clip | `engine/midi/live.rs:~299` (`handle_record_midi_event`) | first NoteOn on an **armed instrument track while the transport is playing**. Plain Play is enough; `Record` is not needed | first note of a run | 1 per armed MIDI track per run (cleared at stop) | `MidiClipCreated` | `midi::clip_created` (records an undo entry only if `transport.recording`) |
| G1 | take group | `engine/takes.rs::resolve_take_group` via `transport.rs::capture_take` | loop seam / stop, for a pass on a track + slot with no matching lane (`take_group_for_slot`, ±`SAME_SLOT_TOLERANCE_FRAMES`) | first pass of a run over a new slot | ≤ 1 per armed track per run (the slot is fixed for the run; audio and MIDI halves of one track share the group) | `TakeCaptured.group_id` | `take_captured` → `TakeGroupsState::take_captured` (the app never creates a group itself) |
| A1 | asset | `engine/import_pool.rs::handle_import_audio_to_pool` | `AudioCommand::ImportAudioToPool { paths }` (`update/pool.rs::import`, control `pool.import` / `clip.place`) | on command, engine thread | 1 per path | `ImportProgress`, then `AssetImported` / `ImportFailed` | `engine_events/pool.rs`. Placement is matched back by **source path** (`PendingImports::take_matching`), because the app doesn't know the id |

Take ids are not a separate space. `push_take` derives them from the
group's own takes, so they need nothing here. MIDI takes
(`TakeContent::Midi`) carry their notes inline and use no clip id.

### 1b. Reservation / high-water sites (the engine *follows* ids it's handed or finds)

| Site | What it does | Exists because of |
|---|---|---|
| `engine/clips.rs::reserve_clip_id` (bumps only below `DERIVED_CLIP_ID_BASE`, FU-A6a), called from `submit_clip_load` (`LoadClipFromWav`, `LoadTakeClipFromWav`), `handle_load_take_clip_from_wav`, `midi/clips.rs::handle_load_midi_clip_direct`, `takes.rs::restore_take_groups_in_place` | `next_clip_id = max(next, id+1)` | C1–C5 must not reissue a loaded id |
| `engine/clips.rs::{reserve_clip_ids_in_project_dir, start_clip_id_scan, settle_clip_id_scan}` + `SetProjectDir` | scans `audio/clip_*.wav` below the base on a worker; each C-site calls `settle_clip_id_scan(state, true)` first | STATE-08 / STATE-12: C3/C4 must not overwrite a deleted clip's WAV that a backup or the redo stack still names |
| `AudioCommand::ReserveAssetIds { above }` (`dispatch/clips.rs:23`), sent by `replay/restore.rs::restore_pool` | `next_asset_id = max(next, above+1)` | A1 must not reissue a loaded asset id (#276 BUG 2) |
| `AudioCommand::RestoreTakeGroups` → `restore_take_groups_in_place` | bumps `next_take_group_id` past every restored group, and `next_clip_id` past every audio take's `clip_ref` | G1 must not reissue group ids (#1393); C3/C4 must not overwrite a take WAV |
| `engine/tracks.rs::handle_clear_all` | `next_take_group_id = 1` (the clip counter is kept on purpose, STATE-08) | project scoping for groups |

### 1c. Counters that go away

`HandlerState::{next_clip_id, next_asset_id, next_take_group_id}`
(`engine/thread/mod.rs:101-119`, initialised to 1 at `:283-287`), plus the
`next_clip_id: &mut ClipId` parameters threaded through
`roll_audio_pass`, `resolve_take_group`, `capture_take_event` and
`restore_take_groups_in_place`, and the `test_support` accessors
(`next_clip_id()`, `next_take_group_id()`, `set_next_take_group_id`).

### 1d. Already app-owned (no change except the allocator's home)

`LoadMidiClipDirect`, `LoadClipFromWav`, `SplitClip { new_clip_id }`,
`BounceTrackToAudio { target_clip_id }` and the vocal render installs all
carry ids from `ComposeState::fresh_derived_clip_id`. Its call sites are
`update/compose/mod.rs:121`, `vocal_midi_install.rs:36`,
`vocal_audio_install.rs:76`, `update/track.rs:885` (bounce target),
`update/import.rs:588` (MIDI file import), `engine_events/pool.rs:195` (pool
placement), and `update/control/{clip.rs:292,515, notes.rs:460}`. FU-A6a
already notes this is the app's de-facto general clip allocator.

---

## 2. Constraints the design must meet

1. **The id must exist before the entity does.** A recording's clip id is
   baked into its file name (`audio/clip_<id>.wav`) when the writer opens, at
   record start or at a seam. It is on disk long before any echo. Renaming
   after the fact (engine-proposes/app-confirms) would mean renaming files
   under an open mmap.
2. **Unbounded consumption.** C4 uses one id per capturing track per loop
   pass. A 1-beat loop at 200 bpm is 0.3 s per pass: 16 tracks for 10
   minutes is about 32 000 ids. No fixed per-command block is safely large
   and also meaningful.
3. **No command marks the moment.** C5 fires on Play (not only Record) for
   any armed instrument track, and an armed track can be armed mid-run.
4. **Monotonic across `ClearAll` / undo / load (STATE-08).** An id that ever
   named a WAV must never be reissued in the session, and not after reopen
   either while that WAV is on disk (STATE-12 / FU-A6c).
5. **Engine thread only.** Every allocation site (C3–C5, G1, A1) runs on the
   engine command thread, not the audio callback. Popping from a `Range` is
   fine. Nothing here touches the RT path.
6. **The control API needs synchronous ids.** It already has them via
   `fresh_derived_clip_id`, and that must not regress.
7. **JSON safety.** Ids reach MCP clients as JSON numbers. Anything that
   burns ids in blocks must stay far below 2^53. From a base of 2^40 that
   leaves about 9 × 10^15 ids.

---

## 3. Options

| | Option | Verdict |
|---|---|---|
| A | **Status quo, formalised.** The engine keeps a counter below `DERIVED_CLIP_ID_BASE`; the app keeps everything above it. | Works today (A4-1 guards it), but fails Epic D's done-when. It keeps two allocators, the STATE-08 disk scan in the engine, `ReserveAssetIds`, and the `RestoreTakeGroups` bump. Every new creating command has to decide its side again. This is the baseline, not a design. |
| B | **Per-command block** (the plan: `ArmTrack { clip_ids: Range }` or `Record { clip_ids }`). | Fine for C3 (the armed set is known at Record). Fails §2.2: C4 either runs out (a defined error, but hit in ordinary use with short loops) or needs a refill protocol, and then that protocol *is* option C. Fails §2.3: C5 fires on Play with no Record, and arming mid-run gets no block. `ArmTrack` specifically is worse: arm state is not undoable and survives `ClearAll`, so a block attached to it outlives the numbering it came from (see §4.4). |
| C | **Standing grant.** One app allocator; the app hands the engine a range up front, the engine draws from it anywhere, asks for more below a low-water mark, and `ClearAll` revokes it. | Covers C3, C4, C5, G1 and realtime bounce with one mechanism and one failure path. The app counter moves past the whole grant when it issues it, so the grant's ids count as issued: never reused, no in-use scan, gaps are harmless. **Recommended.** |
| D | **Engine proposes, app confirms/remaps.** | Rejected by §2.1: the id is already a file name. Remapping would also need a clip-id translation table on both sides for the lifetime of the take, and every in-flight command naming the old id would race the remap. |
| E | **Deterministic composite ids.** The app gives a run id R; the engine derives `R·2^k + pass·tracks + i`. | Just option B with arithmetic. It needs a pass cap `2^k`, doesn't cover C5 arming mid-run, and packs meaning into ids that the control API exposes. |
| F | **Engine-random 64-bit ids.** | No coordination, but the engine still invents ids (fails done-when), it breaks the 2^53 JSON bound unless masked, and it loses monotonicity, which is the whole STATE-08 argument. |

**One shared app-owned clip space, or ranges per kind?** Use one space. FU-A6a
already found that the derived range and the engine's range were one clip
list and one `audio/clip_<id>.wav` namespace. Kinds are not partitions:
recordings, drawn clips, imports and derived clips live in the same maps and
the same folder. A range per kind would bring back the hand-partitioned bases
Epic D exists to delete. Take groups and assets get **their own** counters
because they are different key spaces (`TakeGroupStore`, `asset_<id>.wav`),
not ranges of the clip space.

---

## 4. The design

### 4.1 App side: three allocators in `state/ids.rs`

```rust
/// Session-monotonic id counter: never rewound by undo, a load or `ClearAll`.
/// `seed_past` only raises it.
pub(crate) struct IdCounter { next: u64 }
impl IdCounter {
    pub fn allocate(&mut self) -> u64;                     // one id
    pub fn allocate_block(&mut self, n: u64) -> Range<u64>; // a grant
    pub fn seed_past(&mut self, ids: impl Iterator<Item = u64>);
}

pub(crate) struct EntityIds {
    pub clips: IdCounter,        // starts at CLIP_ID_BASE (= today's DERIVED_CLIP_ID_BASE, 1 << 40)
    pub take_groups: IdCounter,  // starts at 1
    pub assets: IdCounter,       // starts at 1
}
```

- `Resonance::ids: EntityIds` replaces `ComposeState::next_derived_clip_id`.
  `fresh_derived_clip_id()` becomes `r.ids.clips.allocate()` at its nine
  call sites.
- **Never rewound, including on disk load.** Today `ComposeState::clear()`
  resets the derived counter to the base on every load and then reserves
  past what it finds. With one allocator for every clip there is no reason
  to reset. A session-monotonic counter is the simplest STATE-08 argument
  there is, and it makes the A-6 `counter_floor` / `LiveCarry::
  derived_counter_floor` carry unnecessary. The cost: clip ids in a project
  opened second in a session start higher than they would in a fresh
  session. Nobody reads them.
- **Seeding on load, and on Save As into an existing bundle.** `clips`:
  past every loaded audio and MIDI clip, every derived-map value, every take
  `clip_ref` (`TakeContent::Audio`), and every `audio/clip_<id>.wav` ≥ base
  on disk (today's `reserve_derived_clip_ids_on_disk`, FU-A6c).
  `take_groups`: past every restored group. `assets`: past `pool.max_asset_id()`
  **and** every `audio/asset_<id>.wav` on disk. The disk scan is new for
  assets; see §8.3.
- **`CLIP_ID_BASE` stays** (renamed from `DERIVED_CLIP_ID_BASE` and moved back
  to the app; the engine stops knowing it). Its remaining jobs: (a) legacy
  projects hold engine-allocated clip ids below it and legacy WAVs named
  after them, and starting above means none of that needs a scan; (b)
  `rebuild_derived_clips`' drum rule (`clip.id >= base`) is still right for
  the only files it runs on, those with `derived_clips: None` (pre-A-6). The
  serializer always writes `Some` (`serialize.rs:486`), so no file written
  after D-7 reaches that heuristic, and a drawn or recorded clip ≥ base
  can't be misclaimed. The partition therefore survives only as "where the
  one allocator starts". It stops being a partition between two owners.

### 4.2 The grant protocol

```rust
// resonance-audio/src/types/commands.rs
/// Hand the engine ids it may use for entities it has to create at a moment
/// the app cannot decide. The engine appends each range to its grant and
/// never allocates outside it. Silent.
AudioCommand::GrantIds { clips: Range<ClipId>, take_groups: Range<TakeGroupId> },

// resonance-audio/src/types/events.rs
/// A grant fell below its low-water mark. Sent once per crossing, with no
/// repeat until a `GrantIds` lifts it above the mark again.
AudioEvent::IdGrantLow { clips_left: u64, take_groups_left: u64 },
```

Engine: `HandlerState::grant: IdGrant { clips: VecDeque<Range<u64>>,
take_groups: VecDeque<Range<u64>>, low_sent: bool }` with
`fn take_clip(&mut self, tx) -> Option<ClipId>` and `fn take_group(..)`.
`take_*` pops the front id, and sends `IdGrantLow` if the total left is
now below the mark and `!low_sent`. `handle_clear_all` empties the grant.

App:
- **Startup:** send `GrantIds` right after the engine is up, next to the
  existing `SetProjectDir`.
- **After every replay** (disk load, template, both undo paths): send
  `GrantIds` from the seeded counters as the last replay command. The
  `Globals` stage of `reconcile` is the natural home, or right after
  `replay_loaded_project`'s last step. Pin the order in
  `reconcile_order.rs`.
- **On `IdGrantLow`:** `r.ids.clips.allocate_block(GRANT_CLIPS)` and
  `...take_groups.allocate_block(GRANT_GROUPS)`, then send `GrantIds`.
  **Ignored while `io.loading`.** A low event raised before a `ClearAll`
  and handled after it (but before seeding) would otherwise grant ids from
  an unseeded counter that could overlap the incoming project. The replay's
  own closing grant covers that window.
- Sizes (tunables in `state/ids.rs`): `GRANT_CLIPS = 1024`, low-water 512;
  `GRANT_GROUPS = 64`, low-water 32. The worst consumer in §2.2 uses about
  50 ids a second; 512 ids of slack against a refill that takes one event
  drain (≤ 200 ms idle, faster while playing) is a margin of about 10
  seconds. Ids are free (§2.7), so the numbers can be generous.

Consumers switch from `state.next_*_id += 1` to `state.grant.take_*()`:

| Site | On an empty grant (defined behaviour) |
|---|---|
| C3 record start | Checked **before** the stream opens: `grant.clips_len() >= capturing tracks`, else no track records. The response is the existing "Failed to start recording" branch (the transport still rolls, as it does today when the input stream fails) with `EngineError { kind: Busy, "no clip ids available — try again" }`. All tracks or none: no half-armed take. |
| C4 seam reopen | The existing `reopen` failure branch in `roll_audio_pass`: the pass that just finished is kept and captured; that track records no further passes; `write_errors` gets "Recording stopped on this track: no clip id available". This behaviour already exists and is tested for a failed file open. |
| C5 first MIDI note | The note is not captured. One `EngineError::Busy` per run (latched on `midi_recording`). Later notes retry and succeed once a refill has landed. |
| G1 new lane | Pre-drawn at loop-session open for every audio-capturing track with no matching lane, so an audio take never ends up without a lane (an ungoverned take clip would play raw on top of the comp; see `park_take_clip`'s rationale). If that draw fails, record start fails as in C3. A MIDI-only track armed mid-run draws lazily. On failure its MIDI take is dropped with one `Busy` error. |
| realtime bounce | Goes through C3. D-7d must check (unverified here) that a C3 refusal during a realtime bounce ends in `TrackBounceCancelled`/error and that the app removes the pre-created target track, as it does for a user cancel. |

All of these are reachable only if the app stops draining events for
seconds, or in engine unit tests with a deliberately small grant. That is
the point: the failure mode is defined, tested, and non-destructive. No
file is overwritten and no id is reused.

### 4.3 Commands that become mandatory-id (no grant)

- `CreateMidiClip { clip_id, .. }`. The app allocates in
  `section::handle_create_midi_clip` and mirrors optimistically, as
  `CreateEmptyClip` does. The echo stays idempotent.
- `ImportClip` is **deleted**, because it has no app sender. The one engine
  test that uses it switches to `LoadClipFromWav`.
- `ImportAudioToPool { files: Vec<PoolImportFile { asset_id, path }> }`.
  `PendingImport` and the control `JobToken::PoolImport` key on `asset_id`
  instead of the source path (§8.2). The engine opens `asset_<id>.wav` with
  `create_new` and refuses with `ImportFailed { reason: "asset id in use" }`
  if the file exists. That is the engine's "reject a colliding id" for a
  space it keeps no registry of.
- **Collision rejection for clips** (epic done-when): a
  `reject_if_clip_id_in_use` in the create handlers (`CreateMidiClip`,
  `LoadMidiClipDirect`, `SplitClip.new_clip_id`, `BounceTrackToAudio`)
  emits `EngineErrorKind::Internal`, the D-1 template. `LoadClipFromWav`
  and `LoadTakeClipFromWav` already have the binding check under the write
  lock in `submit_clip_load` (idempotent re-loads by design) and keep it.
  Compose's "reuse the slot's id" (#275 P1.7) sends `DeleteMidiClip` before
  `LoadMidiClipDirect` on the same FIFO, so it is not a collision. The step
  that adds this must audit those callers (§6, D-7f).

### 4.4 Why `ClearAll` revokes the grant

A grant is a promise from one numbering. The app counter is never rewound
(§4.1), so on undo the outstanding grant is still valid. A **disk load** is
different: project B may already hold ids inside the range granted while A
was open (B was saved by another session whose counter went further). The
engine would then record onto a B clip's id and its `clip_<id>.wav`. Revoking
at `ClearAll` for every replay kind (load, template, full undo) is one rule
instead of two, and every replay ends with a fresh grant anyway. `ClearAll`
already stops recording (`handle_clear_all`), so no writer is holding a
revoked id.

### 4.5 Composition with the existing guarantees

- **`allocate_unused` and its in-use scan.** Not needed for the three
  counters here. Each is the *only* allocator of its space, and granted ids
  are consumed from the counter when granted, so a partly used block is
  gaps, never a candidate. The scan stays where it earns its keep (tracks
  and track groups share a space; plugin/send/bus mirrors). The module doc
  table in `state/ids.rs` loses its "engine rule on a hint" column.
- **STATE-08 (monotonic across `ClearAll`/replay).** Stronger than today:
  the counter is session-monotonic across disk loads as well, and the
  engine's special case "don't reset `next_clip_id` in `ClearAll`" goes
  away with the counter. The guard `tests/io/id_allocation.rs` gains a
  record → full undo → record case asserting the second take's id is above
  the first.
- **STATE-12 / FU-A6c (on-disk seeding).** Moves wholly to the app:
  `reserve_derived_clip_ids_on_disk` becomes `ids.clips.seed_past(disk
  scan)` and gains an `asset_*.wav` twin. The engine-side worker scan
  (`start_clip_id_scan` / `settle_clip_id_scan`, FU-M12b) is deleted. The
  app scan runs once per load on the UI thread today (FU-A6c); if a
  thousands-of-WAVs bundle makes that measurable, move it into the
  `project::load_project` worker that already reads the bundle.
- **V6 `PersistClipWavs` (FU-V5b).** Unaffected in mechanism: it writes
  `clip_<id>.wav` for in-RAM clips lacking one, keyed by the clip's id, and
  every id is still unique and never reissued. One point to pin in a test:
  a recorded take's WAV is written *before* the app mirrors the clip, under
  a granted id the app already counted as issued, so a snapshot taken
  mid-run can't allocate that id to something else and let `PersistClipWavs`
  overwrite it.
- **Import epochs (M12 / A4, FU-M4a / FU-A4b).** The engine's
  `clear_generation` fence is about staleness, not ids, and stays
  unchanged. App-issued asset ids add a second, app-side fence: an
  `AssetImported` / `ImportProgress` whose id is not pending is from a batch
  a load or undo discarded, and the session-monotonic asset counter
  guarantees such an id never aliases a new batch. `PendingImports::clear`
  on load keeps its UPD-04 meaning.
- **Loop recording: how many ids can a take use?** A *take* uses exactly
  one clip id (audio) or none (MIDI). A *run* uses `capturing_tracks ×
  passes` clip ids plus at most `armed_tracks` group ids. The grant has no
  per-run cap; it is refilled as the run goes (§4.2).
- **Control-API clip creation.** Unchanged in behaviour.
  `notes.create_clip`, `clip.place` and `clip.split` already allocate
  synchronously and now call `r.ids.clips.allocate()`. No `PROTOCOL_VERSION`
  bump: no wire type changes, and ids stay below 2^53.
- **The derived-range partition.** Becomes moot as a partition. See §4.1:
  one allocator, one base, kept only as a start point. The FU-A6a engine
  rule and `derived_clip_id_partition.rs` are deleted with the engine
  counter.
- **A-13 reconcile.** `LiveCarry::derived_counter_floor` becomes dead,
  because the counter lives on `Resonance`, not in replayed state, and is
  never lowered. It should be removed in D-7b. The closing `GrantIds` is a
  new tail command for the reconcile driver.

---

## 5. Behaviour changes (call out in each step's commit)

1. **Recorded, drawn and live-MIDI clip ids are large** (≥ 2^40, like every
   derived and control clip today). This matters where the id is in a name:
   the engine names clips `"Recording {id}"`, `"Take {id}"`,
   `"MIDI Take {id}"`, so users would see "Recording 1099511627812". The
   proposal is for the engine to stop numbering names (`"Recording"`,
   `"Take"`, `"MIDI Take"`) and for the app to number them on the echo from
   a per-track ordinal. This needs a decision (§7 Q1).
2. **Pool imports of the same file twice** place each copy where its own
   gesture asked for it. Today two batches of one path can swap placements
   if the second batch finishes first (§8.2).
3. **Out-of-ids** (§4.2) is a new, visible error, reachable only if the app
   stalls. The user sees a "no clip ids available" banner. Recording either
   doesn't start (C3/G1), or stops on that track after the current pass
   (C4), with the passes so far kept. For live MIDI the first note is
   dropped.
4. Asset ids stop resetting to `pool max + 1` per session and never reuse an
   orphaned `asset_<id>.wav` (§8.3).
5. Clip ids no longer restart at the base when a second project is opened in
   the same session. This can't be seen in the GUI; a control client might
   notice.

---

## 6. Migration: D-7a … D-7f

Each step can land on its own and leaves master green. Order by
independence and mechanicalness; D-7d depends on D-7b.

**D-7a — asset ids (app-owned, no grant).** Mechanical and self-contained.
- `EntityIds.assets` (put it on `MediaState` if `EntityIds` doesn't exist
  yet); `ImportAudioToPool { files }`; `PendingImport` and
  `JobToken::PoolImport` keyed by `asset_id`; seed from the pool plus an
  `asset_*.wav` scan; engine `create_new` refusal. Delete `next_asset_id`,
  `ReserveAssetIds`, and `restore_pool`'s `reserve_engine_ids` branch.
- Guards: `tests/io/id_allocation.rs` asset case (import → reload → import:
  the id is above the pool max and above an orphaned `asset_N.wav`); a
  same-path-twice test with two targets, out of order; FU-A4b cancellation
  tests stay green; engine test: a colliding asset id → `ImportFailed`,
  existing file untouched.
- Size ~300 lines. Conflict: low (`update/pool.rs`, `engine_events/pool.rs`,
  `engine/import_pool.rs`, `control/clip.rs` job, `replay/restore.rs`).
  Risk: low.

**D-7b — promote the clip allocator (app-only refactor).**
- `state::ids::{IdCounter, EntityIds}`, `Resonance::ids`, rename
  `fresh_derived_clip_id` → `r.ids.clips.allocate()` (9 sites), stop
  resetting on load, seed as in §4.1 (including take `clip_ref`s), delete
  `ComposeState::next_derived_clip_id` and `LiveCarry::derived_counter_floor`,
  rename `DERIVED_CLIP_ID_BASE` → `CLIP_ID_BASE` (keep a
  `#[deprecated]` alias for one step if the diff gets noisy).
- Guards: A-6's `undo_snapshot_fixed_point` derived cases;
  `id_allocation.rs` "counter never lowered by load" case (open A with a
  high counter, open B with low ids, allocate: the id is above both).
- Size ~250. Conflict: **medium**. `compose/state.rs` and
  `reconcile/derived_clips` are A-13 group (6) territory (A-13i touches clip
  add/remove), so don't run it concurrently with A-13h/i. Risk: low
  (app-only, no engine change).

**D-7c — `CreateMidiClip { clip_id }`, delete `ImportClip`.** Mechanical.
- Optimistic mirror in `section::handle_create_midi_clip`. Engine uses the
  given id; `loop_record_takes.rs` stops using `ImportClip`.
- Guards: the FU-A6a test (`id_allocation.rs`, "drawn clip between two
  derived ones") now asserts the drawn id came from `r.ids.clips`; engine
  test that `CreateMidiClip` honours the id.
- Size ~150. Conflict: low. Risk: low.

**D-7d — grant protocol + recording clip ids (C3, C4, C5, realtime
bounce).** The riskiest step.
- `GrantIds` / `IdGrantLow`, `HandlerState::grant`, `ClearAll` revoke, app
  startup + replay-tail + low-water refill (ignored while loading), the §4.2
  failure paths.
- `test_support`'s engine harness grants a default block at construction so
  the ~40 recording and take tests keep their shape. Tests that assert
  literal ids (`next_clip_id()` in `loop_record_takes.rs`,
  `take_comp_render.rs`, `load_clip_offthread.rs`,
  `recording_write_failure.rs`) switch to "ids come from the grant, in
  order".
- Guards (new, in the `engine` group): (1) three-pass cycle record with a
  grant of exactly `tracks × 2` → pass 3 hits the C4 failure branch, the
  two takes are kept, no file is overwritten; (2) `IdGrantLow` fires once
  at the mark; (3) `ClearAll` revokes, so a record after `ClearAll` without
  a new grant fails C3 cleanly; (4) C5 with an empty grant → no clip, one
  error. App side (`timeline` or `io` group, captured engine): replay ends
  in `GrantIds` from the seeded counter; `IdGrantLow` while `loading` is
  ignored; STATE-08 record → full undo → record gives a strictly larger id.
- Size ~450. Conflict: **high**. `engine/transport.rs`, `recording.rs` and
  `engine/clips.rs` are shared with Epic B, so don't run it concurrently
  with a B todo touching clips or recording. Risk: highest (real-time
  capture path, many tests, a new protocol).

**D-7e — take-group ids via the grant (G1).**
- `resolve_take_group` takes from `grant.take_groups`; loop-session open
  pre-draws for audio-capturing tracks (§4.2). Delete
  `next_take_group_id`, `ClearAll`'s reset, and `RestoreTakeGroups`' group
  **and** clip bumps (`restore_take_groups_in_place` loses both counter
  params); fix the `commands.rs` docs that explain the bumps.
  `EntityIds.take_groups` is seeded on restore.
- Guards: `take_lanes_persistence.rs` load → record → new lane gets an id
  above every restored one (the #1393 regression, now app-side);
  `take_group_mirror`; a same-slot re-run still joins the existing lane and
  uses no group id.
- Size ~200. Conflict: medium (`engine/takes.rs`, Epic B's comp
  publishing). Risk: medium.

**D-7f — delete the engine's clip counter and add collision rejection.**
- Delete `HandlerState::next_clip_id`, `reserve_clip_id` and its 4 callers,
  `reserve_clip_ids_in_project_dir` / `start_clip_id_scan` /
  `settle_clip_id_scan` and the `SetProjectDir` scan,
  `DERIVED_CLIP_ID_BASE` in `resonance-audio`, and
  `tests/engine/derived_clip_id_partition.rs` (superseded).
  `reject_if_clip_id_in_use` in the create handlers (§4.3) after auditing
  the reuse-after-delete callers. Rewrite the `state/ids.rs` module doc.
  After D-5 has also landed, add
  `arch-invariants::engine_has_no_id_counters` (the epic's done-when grep
  `next_[a-z_]*_id` over `resonance-audio/src/engine`, as a test).
- Guards: engine test that a duplicate `LoadMidiClipDirect` /
  `CreateMidiClip` → `Internal` error and the first clip is untouched;
  `id_allocation.rs` stays green, and its `FakeEngine::clip(None)` path is
  deleted because the fake can no longer allocate.
- Size ~200 (net negative). Conflict: medium (`engine/clips.rs`). Risk:
  low-medium (the rejection can surface a latent double-create; that is the
  point, but have the error visible in the log).

Dependencies: D-7a, D-7b and D-7c are independent of each other. D-7d
needs D-7b (the grant draws from `r.ids.clips`). D-7e needs D-7d. D-7f needs
D-7c, D-7d and D-7e.

---

## 7. Open questions for a human

1. **Clip names.** Should recorded and drawn clips stop embedding the id
   (`"Recording 1099511627812"`)? The proposal is that the engine emits bare
   names and the app appends a per-track ordinal on the echo. The
   alternative is to accept the long names. This is a user-visible change
   either way.
2. **Never rewinding the clip counter on disk load** (§4.1). It is simpler
   and closes STATE-08 fully, but ids in a second project opened in a
   session no longer start at the base. Acceptable? (No real users yet.)
3. **Out-of-ids at record start**: roll the transport without recording
   (consistent with today's stream-open failure), or refuse to roll at all?
   The proposal keeps today's behaviour.
4. **Grant sizes** (1024/512 clips, 64/32 groups): fine as tunables, or
   should the refill scale with the number of armed tracks?

---

## 8. Found along the way (not fixed here)

1. **`AudioCommand::ImportClip` has no app sender**; only
   `resonance-audio/tests/engine/loop_record_takes.rs` uses it. It is dead
   API with its own UPD-09 fence. D-7c deletes it.
2. **Pool-import placements are matched by source path**
   (`PendingImports::take_matching`, FIFO). Each batch runs on its own
   thread, so two drops of the same file onto different tracks can finish
   out of order and swap placements. Latent and low severity. D-7a fixes it
   by keying on the asset id.
3. **Asset WAVs have no STATE-12 seeding.** After reopen the engine's asset
   counter is raised only to `pool.max_asset_id()`. An `audio/asset_N.wav`
   above that (an import that was undone before saving, still named by a
   versioned backup) is overwritten by the next import. Low severity.
   D-7a fixes it with the disk scan.
4. **Live MIDI is captured on plain Play** for armed instrument tracks
   (`handle_record_midi_event` checks `playing` and `record_armed`, not
   `recording`). `midi::clip_created` records an undo entry only when
   `transport.recording`, so a clip captured during Play has no undo entry
   of its own. Unverified whether that is intended.
5. FU-D4a (the engine creates a default track id 1 unprompted at startup)
   has the same "engine creates without an app id" shape. It is out of scope
   here (D-4 follow-up), but the startup `GrantIds` send in D-7d sits next
   to where that fix would go.
