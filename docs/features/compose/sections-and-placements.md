# Sections and placements

> Verified against master @ `f9438e38` + the chord-change vocal melody flow (BUG-70) on 2026-10-13.
> Spec history: none in `docs/history/specs/`; leads in `docs/history/refactor/A-6-derived-clips.md` and `docs/history/reviews/code-review-2026-10-02.md` (VIEW-04/05/12/13/17).

A song in the Compose tab is built from **section definitions** (Intro,
Verse, Chorus …: a name, a colour, a length in bars, a chord grid, a scale,
lane generators and a drum arrangement) and **placements** (one definition
put on the timeline at a bar). The same definition can be placed several
times; every placement plays the same generated material. A user or agent
creates, renames, resizes, recolours, places and deletes sections here.

## Behaviour

- **Create.** Length is 1..=100 000 bars (`MAX_SECTION_BARS`). The GUI form
  pre-fills the first unused stock name (Intro → Verse → Chorus → Bridge →
  Outro, then `Section N`) and cycles a six-colour palette
  (`SECTION_PALETTE`). A new definition starts with no chords, no scale,
  `beats_per_chord = 4`, triads, a generated motif seeded from its id, and an
  empty drum arrangement.
- **Implicit placement.** Creating a section also places it at the first
  free bar (`first_free_bar`) unless the control call passes
  `place: false`. A following `section.place` at the same spot then fails
  with "Placement would overlap an existing section".
- **Placements never overlap.** `placement_overlaps` rejects any place or
  grow that would overlap another placement; spans must end by bar 100 000.
  Placements are kept sorted by `start_bar` (0-based in the app, 1-based on
  the wire).
- **Resize.** Growing fails if any placement would overlap its neighbour.
  Shrinking fails if a chord would fall outside the new length. On success
  every lane that already has derived clips is re-derived at the new length
  (`rederive_section_clips`; a vocal lane whose melody that would rewrite
  follows `section.resize`'s `vocal_melody` — keep by default — see
  [vocal lyrics and melody](vocal-lyrics-and-melody.md#chord-and-length-changes-under-a-melody-bug-70));
  a drum arrangement that filled the old length
  is extended to fill the new one. Resizing never generates a lane that had
  no clips.
- **Meter.** A section's beats-per-bar and BPM come from the tempo map at
  its earliest placement (`section_meter`), never from the transport. When
  a time-signature edit makes a section shorter in beats, chords past the
  end are trimmed or dropped and the lanes re-derived
  (`revalidate_chords_after_meter_change`); this rides the signature edit's
  undo entry.
- **Delete.** The GUI refuses to delete a definition that still has
  placements. `section.delete` deletes the definition and all its
  placements in one undo step and requires `confirm: true`. Deleting a
  placement tears down its derived MIDI clips and installed vocal audio
  (`purge_placement_outputs`); the vocal WAV stays on disk so undo can
  restore it.
- **Placing does not derive.** `handle_place` only records the placement.
  The new placement gets clips the next time each lane is regenerated
  (generate, a chord edit, a resize). See
  [placement editing](../../intents/compose/section-placement-editing.md).
- **Moving.** There is no move for a placement in the GUI or the control
  API. Placements only move through `arrangement.insert_bars` /
  `remove_bars` (timeline area), which shift them by whole bars.
- **Deleting a track** removes its lane generator from every section and
  its derived and vocal clips (`forget_track`).
- **Undo.** Every section mutation is `UndoAction::Record`; one control call
  is one undo step.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Runtime model | `resonance-app/src/compose/section.rs`, `compose/state.rs` | `SectionDefinitionState`, `SectionPlacementState`, `ComposeState::{definitions, placements, fresh_id}` |
| Invariants | `resonance-app/src/compose/invariants.rs` | `MAX_SECTION_BARS`, `section_span_in_bounds`, `placement_overlaps`, `chord_fits_in_section` |
| Update handlers | `resonance-app/src/update/compose/section.rs` | `handle_create`, `handle_resize`, `handle_place`, `handle_delete_with_placements`, `purge_placement_outputs`, `rederive_section_clips` |
| Dispatcher + meter | `resonance-app/src/update/compose/mod.rs` | `handle`, `section_meter`, `meter_at_bar`, `forget_track` |
| Persistence | `resonance-app/src/project/sections.rs` | `ProjectSectionDefinition`, `ProjectSectionPlacement`; `ComposeState::to_project_definitions` / `load_from_project` |
| View | `resonance-app/src/view/compose/strip.rs`, `page.rs`, `layout.rs`, `global_tracks.rs` | section chips, create/edit forms |
| Control handlers | `resonance-app/src/update/control/section.rs` | `create`, `place`, `set_lane_generator`, `parse_key_scale` |

Every control handler synthesizes a `ComposeMessage` and routes it through
`run_via_update`, so a remote edit takes the same gates and undo path as a
GUI edit. GUI handlers report problems through `compose.last_error`; the
control layer drains it into a JSON-RPC error.

## Surfaces

- **GUI:** Compose tab section strip (`view/compose/strip.rs`): `+ Section`,
  click a chip to select a placement, edit form with Color, Place again,
  Delete here, Delete section; a "Seed markers" button
  (timeline markers). Command palette "Duplicate selection" places the
  selected section right after itself when that space is free.
- **Control API / MCP:** `section.create` → `mcp__resonance__section_create`,
  `section.rename` → `section_rename`, `section.resize` → `section_resize`,
  `section.delete` → `section_delete`, `section.place` → `section_place`,
  `section.remove_placement` → `section_remove_placement`,
  `section.set_scale` → `section_set_scale` (see
  [section chords](section-chords-and-progressions.md)), read via
  `song.sections` / `song.summary`.
- **Agent skills:** `resonance-agent-plugin/skills/song-structure`,
  `arranging`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Bounds, clamping on load | `resonance-app/tests/compose/section_bounds.rs` | `./scripts/run-tests.py -p resonance-app` |
| Delete cleanup | `tests/compose/section_delete_cleanup.rs`, `track_delete_lane_cleanup.rs` | same |
| Resize re-derive | `tests/compose/section_resize_rederive.rs` | same |
| Meter from tempo map | `tests/compose/section_meter_from_tempo_map.rs`, `compose_bar_sample_tempo_map.rs`, `chords_after_meter_change.rs` | same |
| Control methods | `tests/control/control_section_harmony.rs`, `tests/e2e_compose_via_control.rs` | same |
| Inline round-trips | `resonance-app/src/compose/tests.rs`, `compose/invariants.rs` (`#[cfg(test)]`, documented exception) | same |
| Goldens | `tests/snapshots/compose_section_banner_collapsed-wgpu.png`, `compose_both_banners_collapsed-wgpu.png` | `RESONANCE_BLESS=1` |

Not covered: the "Place again" button (it places at bar 0, which overlaps
whenever bar 0 is taken), and placing an already-generated section (no
test asserts the new placement stays empty until a regenerate).

## Decisions and deviations

- `section.create` places by default because early clients expected it; the
  `place` flag was added later (ba doc #269 FR-6) instead of changing the
  default.
- The control delete cascades to placements while the GUI refuses; the GUI
  path predates the control API and was kept.
- Section meter comes from the tempo map at the first placement (VIEW-13)
  so edits do not depend on where the playhead is.

## Known gaps

- New placements are not filled until a regenerate; no placement move; the
  "Place again" button targets bar 0 —
  [section-placement-editing](../../intents/compose/section-placement-editing.md).
