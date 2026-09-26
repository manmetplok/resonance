# A-6: fold `compose_derived_clips` + `compose_next_derived_clip_id` out of `UndoExtras`

Design for `refactor-intent.md` Epic A item 6 (`arch-migration-plan.md`
ARCH-01 A1-2 step 8). Written against master `22cb2ee3`.

## 1. What the map is

`ComposeState::derived_clips: HashMap<(definition_id, placement_id, TrackId), ClipId>`
(`compose/state.rs`). It records which timeline clip the compose model
generated for one lane (track) of one placement of one section. The value is
a **MIDI clip** id. Vocal *audio* renders have their own map,
`vocal_audio.clips`, which is out of scope here: it was never in `UndoExtras`,
and both paths rebuild it from `r.clips` (`rebuild_vocal_audio_clips`).

### Writers

| Writer | What it does |
|---|---|
| `update::compose::install_derived_midi_clip` (regenerate, drum materialise, section edits) | remove key, then `DeleteMidiClip` the old id; **reuse the slot's id** (#275 P1.7) or `fresh_derived_clip_id()`; `LoadMidiClipDirect`; insert key. Mirrors into `midi_clips` now (`Immediate`) or leaves it to the `MidiClipCreated` echo (`OnEcho`) |
| `VocalMidiInstall::install` | same without id reuse, always mirrors now |
| `purge_placement_outputs`, `forget_track`, track-removed echo | remove entries (and their clips) |
| `demo::seed_demo_vocal_melody` | inserts directly (clip id 16, **below** `DERIVED_CLIP_ID_BASE`) |
| `ComposeState::load_from_project` | clears it (every load, both undo paths) |
| `rebuild_derived_clips` | disk load: positional heuristic — a clip whose start is exactly a placement's bar, on a track with a lane generator for that section (or a drum track, id ≥ base) |
| `Resonance::restore_derived_clips` | both undo paths (U1 / FU-H2a): the snapshot's map; the slow path keeps only entries whose clip was replayed |

Only one writer uses `OnEcho`: the GUI's `materialize_drum_clips` (every drum
editor edit, `ArrangementMessage::*`, the demo seed). Every other install
mirrors the clip synchronously.

### Readers

Regenerate/install (tear-down + id reuse), `rederive_section_clips` (a section
resize rebuilds **only lanes that have an entry**), the vocal pipeline
(`vocal_render`, `expression`, `control::vocal`), the control view model
(`view_model/lane.rs`, `generate.rs`), the vocal lane / vocal roll views, and
UPD-05's `revalidate_frozen_content` (it skips a frozen track while any entry
on it points at an unmirrored clip, "echo pending").

### Relation to `midi_clips`

Values are ids of `midi_clips` entries, but the map is not a function of
`midi_clips` in live state. An entry can point at a clip that is not mirrored:

1. **In flight** — an `OnEcho` install before its `MidiClipCreated` lands.
   The engine has the clip; the mirror will get it.
2. **Dangling** — the user deleted a derived clip on the timeline;
   `midi::clip_deleted` does not touch the map. The regen path then
   `DeleteMidiClip`s a dead id (harmless) and reuses the id for the slot.
   *Fixed by FU-A6b:* the user-delete sites (`MidiClipMessage::DeleteMidiClip`,
   `remove_bars` casualties) call `ComposeState::forget_deleted_derived_clip`.
   Not the echo: a regeneration deletes and re-installs a slot under the
   same id, and that teardown's `MidiClipDeleted` lands after the new entry.

Conversely a mirrored clip can belong to an entry the positional rebuild would
never find (a derived clip the user moved or retimed), and the rebuild can
claim a clip the compose model never made (a hand-drawn clip on a
lane-generator track that starts exactly on a placement bar — the next
regenerate then deletes the user's clip).

## 2. The in-flight echo (FU-H2a) and the representation

FU-H2a: a snapshot taken while an `OnEcho` echo is pending has the entry but
not the clip (`build_project_file` writes `midi_clips` only). The diff replay
keeps the engine's clip, so the echo lands after the restore; if the entry was
dropped the clip is an orphan and the next regenerate stacks a duplicate.
That is why U1 made both paths restore **the snapshot's map**, not a rebuild.

### Options

**(a) `ProjectMidiClip.derived_from: Option<DerivedKey>`** (the plan's
suggestion). The mapping exists only for clips in the file. It cannot hold an
in-flight entry, nor a dangling one. To keep FU-H2a closed it needs a second
change: make the GUI drum materialise `Immediate`, so no entry is ever
unmirrored at insert time. It also can't tell a new file with no derived clips
from an old file that needs the positional rebuild without an extra marker, it
can't represent two keys on one clip, and it touches every `ProjectMidiClip`
literal.

**(b) Top-level `ProjectFile.derived_clips: Option<Vec<ProjectDerivedClip>>`,
the map verbatim** (key + clip id, sorted by key). **Chosen.**

- It is lossless, in-flight and dangling entries included. The undo paths get
  exactly the map `UndoExtras` carried, so the U1 restore rule is unchanged:
  the fast path keeps every entry (an echo may still land), and the slow path
  keeps only entries whose clip was replayed.
- `None` means the file predates the field. Old projects (and every
  `ProjectFile { .. Default }` literal: built-in templates, tests) take the
  positional rebuild exactly as today. `build_project_file` always writes
  `Some`, so an empty map is still authoritative.
- It is additive serde (`#[serde(default, skip_serializing_if = "Option::is_none")]`).
  No `PROJECT_FORMAT_VERSION` bump, the same as the `chord_track` precedent.
  An older build ignores the unknown key (no `deny_unknown_fields`).
- Referential integrity is enforced on read, not by structure: a disk load
  and a slow-path restore drop entries whose `clip_id` is not among the
  replayed `midi_clips`. This is the rule U1 already applies on the slow path.

Entries whose clip isn't in `midi_clips` yet: in the file they stay as data.
A fast-path restore keeps them (the engine still has the clip and its echo
will land). A slow-path restore or disk load drops them, because `ClearAll`
wiped the engine and the clip can never arrive. So a save taken mid-echo loses
that one entry on reload, exactly as today's rebuild would (the clip isn't in
the file either).

## 3. The counter

`ComposeState::next_derived_clip_id` is the app-side allocator for the derived
range (`state/ids.rs`, base `1 << 40`). Both derived MIDI clips and vocal
*audio* clips come from it (`fresh_derived_clip_id`). Today it is reset to the
base by `load_from_project` and then bumped by `reserve_derived_clip_ids`
(over MIDI clips in `rebuild_derived_clips`, over audio clips in
`rebuild_vocal_audio_clips`). An undo **rewinds** it to the snapshot's value,
then reserves past the restored MIDI clips.

Can it be derived from the file (max + 1)? Not as a *restore* value:

- The in-flight entry's id is in no clip list of the file. After a fast-path
  restore its echo lands, and max+1 over the file's clips may be ≤ that id,
  so the next allocation collides with a live clip.
- Rewinding at all re-issues ids the redo stack still names. A vocal audio
  clip's WAV is `audio/clip_<id>.wav` (`PersistClipWavs`, V6). An id re-issued
  after an undo lets a render that completes afterwards overwrite the WAV a
  redo snapshot points at: the derived-range twin of STATE-08. STATE-08 fixed
  that for the engine allocator by making it monotonic across `ClearAll`.

**Decision: the counter is session-monotonic and is not undo state.** It is
not persisted and not snapshotted. An undo never lowers it. Both paths take
`max(live counter, reserve past every restored MIDI clip, audio clip and map
value)`. A disk load (or new project) still resets to the base and reserves
past the loaded clips, as today, so ids stay deterministic per file. The fast
path keeps its pre-restore value across `load_from_project`; the slow path
reads it at the top of `replay_loaded_project` when the replay is an undo
(`pending_undo_extras.is_some()`, which becomes `io.restoring_undo` in A-7).

Persisting it (a third option) would give cross-session monotonicity too, for
the backups side of STATE-12. But a monotonic value in the file breaks the
undo fixed point, and an exact snapshot value keeps the rewind. Left as a
follow-up.

## 4. File format

```rust
/// ProjectFile
#[serde(default, skip_serializing_if = "Option::is_none")]
pub derived_clips: Option<Vec<ProjectDerivedClip>>,

/// project/sections.rs
pub struct ProjectDerivedClip { definition_id: u64, placement_id: u64, track_id: u64, clip_id: u64 }
```

- Save (`build_project_file`): `Some(map sorted by key)`.
- Load (`replay_loaded_project`, disk and slow-path undo): `Some` → entries
  whose clip was replayed. `None` → `rebuild_derived_clips` (unchanged).
- Fast-path undo (`apply_compose`): `Some` → all entries. `None` → rebuild.
  This is unreachable: snapshots come from `build_project_file`.

## 5. Code changes

- `UndoExtras` loses both fields and becomes an empty struct. The type,
  `pending_undo_extras`, `finalize_undo_restore` and the `extras` parameters
  stay for A-7. `undo_extras()`, `extras_equal` and `same_state` lose the
  field compares, and `finalize_undo_restore` loses the derived-clip restore.
- `Resonance::restore_derived_clips(&ProjectFile, echoes_in_flight,
  counter_floor: Option<u64>)` becomes the one restore for all three callers.
- `replay_vocal` calls it instead of `rebuild_derived_clips`. Legacy files
  are handled inside.

## 6. Guard tests (end of `tests/io/undo_snapshot_fixed_point.rs`, "A-6")

1. **Both paths, settled state**: the demo plus a section resize (which
   re-derives, keeping slot ids) between snapshot and restore. Assert the map
   equals the snapshot's (read from the file after the change, from the live
   map before), and that the counter never goes below its pre-restore value.
2. **In flight (FU-H2a) through both paths**: the existing
   `derived_clips_with_a_pending_echo_survive_both_restore_paths` is rewritten
   against the file field. The fast path keeps the pending entry; the slow
   path drops it and keeps the replayed ones.
3. **Counter**: after an undo, a fresh derived id is above every id issued
   before the undo (monotonic). Before the change this fails (the counter
   rewinds).
4. **Disk round trip**: save → `load_project` → replay. The map comes back
   identical, including a derived clip moved off its bar (the positional
   rebuild loses it: fails before the change). A hand-drawn clip on a
   placement bar is not claimed.
5. **Old project**: the same file with the `derived_clips` key removed loads
   and yields exactly `rebuild_derived_clips`' map.

The existing `check_both_paths` fixed point (demo + 4 templates) keeps
covering the settled case.

## 7. Behaviour changes

1. **Undo no longer rewinds the derived-clip id counter.** A clip generated
   after an undo gets a new id instead of re-using one the redo stack still
   holds. Ids are opaque, so this is visible only to control clients as
   different numbers. It fixes the derived-range STATE-08 analogue.
2. **A disk load of a project saved by this build restores the saved map
   instead of guessing by position.** A derived clip the user moved stays
   claimed, so regenerate replaces it instead of duplicating it. A hand-drawn
   clip that happens to start on a placement bar of a generator lane is no
   longer claimed, so regenerate no longer deletes it. Old projects load
   exactly as before.
3. A gesture whose only effect was a counter bump no longer counts as an edit.
   No such gesture exists: every allocation installs a clip or an entry, and
   both are in the file.

None of these needs a product decision: 2 makes save/load keep what the
session had, which undo already did.

## 8. Found along the way (not fixed here)

- **Engine/derived id overlap (for D-6/D-7):** the engine bumps `next_clip_id`
  past *any* id it is handed (`LoadMidiClipDirect`, `LoadClipFromWav`), so
  after the first derived clip its own allocations (GUI clips, recordings)
  land at `DERIVED_CLIP_ID_BASE + k`. That is where `fresh_derived_clip_id`
  will allocate next, and `fresh_derived_clip_id` does not skip mirrored ids.
  `state/ids.rs` names a non-existent `ComposeState::allocate_derived_clip_id`
  and claims the range is "always safe".
- Dangling entries (user-deleted derived clips) survive a fast-path undo, as
  they did before, and keep suspending UPD-05 on that track. They also make
  `rederive_section_clips` resurrect the deleted lane clip on a resize.
  **Fixed (FU-A6b):** a user delete drops the entry, so a resize leaves the
  lane empty (it re-derives only lanes with an entry) and an explicit
  regenerate installs a new clip under a fresh id; undoing the delete
  restores clip and entry. Drum clips are still re-materialised from the
  pattern by any drum edit, as before (the materialiser ignores the map).
- The derived counter is not seeded from `audio/clip_*.wav` on load, so a
  vocal WAV of a deleted derived clip that a backup still references can be
  overwritten after a reopen (STATE-12 for the derived range).
