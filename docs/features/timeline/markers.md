# Arrangement markers

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none in `docs/history/`; built under todos #367–#370 (design doc #161), cited in code comments.

Named flags and regions in the Arrange ruler. A user can drop a marker at
the playhead, drag it, stretch it into a region, rename and recolour it,
jump between markers, loop a region, or seed one region per Compose
section placement. Markers are app-only metadata: the engine never sees
them, except as the seek and loop commands they trigger.

## Behaviour

- A marker is a **point** (`end_sample: None`, drawn as a flag) or a
  **region** (`end_sample: Some`, drawn as a translucent labelled span with
  start and end handles). Positions are in samples.
- **Add at playhead** (`Shift+M`, palette "Add Marker at Playhead"): snapped to
  the grid, named `Marker {id}`, coloured from a six-colour palette cycled by
  marker count.
- **Drag** the flag to move the start (snapped); drag a region's end edge to
  resize it (snapped). Each drag gesture coalesces into one undo entry per
  marker (`CoalesceKey::MarkerMove` / `MarkerResize`).
- **Double-click** a flag to rename inline. Committing an empty name is
  ignored.
- **Right-click menu:** Rename, Recolor (six swatches), Loop to section, Play
  from here, Convert to region / Convert to point, Delete. "Convert to region"
  spans to the next bar line.
- **Navigation:** `.` / `,` jump to the next / previous marker, wrapping
  around; transport prev/next buttons do the same. The transport flag button
  opens an overview popover listing every marker (swatch, name, bar); clicking
  a row seeks there and leaves the popover open.
- **Loop to section:** a region loops over `[start, end]`; a point loops to the
  next marker's start. No-op on the last point marker.
- **Seed from sections** (button in the Compose strip): replaces every
  `seeded` marker with one region per Compose section placement (section name
  and colour, span from `start_bar` + `length_bars`). Hand-placed markers stay.
  Seeding is manual; editing placements afterwards does not update the seeded
  markers.
- **Follows the music:** a tempo change through `CommitBpm` re-anchors markers
  musically, and insert / remove bars moves markers after the cut (see
  [tempo-and-signature-map.md](./tempo-and-signature-map.md),
  [arrangement-bars.md](./arrangement-bars.md)). A marker that starts inside a
  removed span is not deleted; it stays where it was.
- Ids are app-local, allocated by `ArrangementMarkers::allocate_id`, restored
  as `max + 1` on load.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model | `resonance-app/src/state/markers.rs` | `ArrangementMarker` (`seeded` flag), `ArrangementMarkers` (`allocate_id`, `next_marker`, `prev_marker`, `move_start`, `set_region_end`) |
| Edits + navigation | `resonance-app/src/update/marker.rs` | `MarkerMessage`, `seed_from_sections`, `loop_to_region`, `MARKER_PALETTE` |
| Transient UI (selection, menu, rename) | `resonance-app/src/update/marker_ui.rs` | `MarkerUiMessage` (only `CommitRename` is undoable) |
| Ruler draw + hit test | `resonance-app/src/view/timeline/draw/grid.rs`, `view/timeline/hit_test.rs` | `draw_markers`, `marker_hit`, `MarkerHit::{Flag, EndEdge}` |
| Pointer input | `resonance-app/src/view/timeline/input/pointer.rs`, `input/hover.rs` | `MarkerDrag` |
| Context menu + rename field | `resonance-app/src/view/menus.rs` | marker menu, `marker_rename_overlay` |
| Overview popover | `resonance-app/src/view/markers_overview.rs` | `view_markers_overview_overlay` |
| Commands | `resonance-app/src/commands/{mod,bindings,resolve}.rs` | `NextMarker`, `PrevMarker`, `AddMarkerAtPlayhead`, `ToggleMarkersOverview` |
| Persistence | `resonance-app/src/project/model.rs` | `ProjectFile::arrangement_markers` |

Undo rides the `ProjectFile` snapshot; there is no marker-specific undo
bookkeeping.

## Surfaces

- **GUI:** Arrange ruler band; transport marker buttons and overview popover;
  Compose strip "seed markers" button; shortcuts `.`, `,`, `Shift+M`.
- **Control API / MCP:** none. Markers are moved by `arrangement.insert_bars` /
  `remove_bars` (reported as `markers_moved`), but no method lists, adds or
  edits them. See [intent: markers control API](../../intents/timeline/markers-control-api.md).
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Model | `resonance-app/tests/timeline/test_arrangement_markers.rs` | `./scripts/run-tests.py -p resonance-app` |
| Reducers (add, seed, move, loop, jump, undo) | `resonance-app/tests/timeline/arrangement_marker_reducers.rs` | same |
| Selection / menu / rename | `resonance-app/tests/timeline/marker_ui_reducers.rs` | same |
| Hit geometry | `resonance-app/tests/timeline/marker_hit_test.rs` | same |
| Overview wiring | `resonance-app/tests/timeline/markers_overview_ui.rs` | same |
| Goldens | `tests/snapshots/timeline_markers_render_in_ruler-wgpu.png`, `markers_overview_popover_renders-wgpu.png` (`timeline_markers_snapshot.rs`, `markers_overview_snapshot.rs`) | `RESONANCE_BLESS=1 cargo test -p resonance-app --test timeline` |

Not covered: the right-click menu's Recolor and Convert items as rendered
(only reducers), and markers surviving a tempo change (the re-anchor path is
shared with clips and covered there).

## Decisions and deviations

- `seeded` separates generated regions from hand-placed ones so a re-seed is
  idempotent and never touches user markers.
- Markers stay app-only; no engine command carries a marker id.
- Region end is not validated against the start; the GUI only produces ends
  after the start.

## Known gaps

- No control API — [markers-control-api](../../intents/timeline/markers-control-api.md).
- Seeded regions do not follow later section edits; the user re-seeds by hand.
