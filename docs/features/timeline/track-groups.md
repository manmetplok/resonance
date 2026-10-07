# Track groups (folder tracks)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/intents/prototypes/track-grouping-folder-tracks/DESIGN.md` (epic #36, docs #200/#203).

A group folds related tracks under one header row in the Arrange view. It
gives them a shared identity colour, can be collapsed to a single overview
lane, and carries a group mute, solo and level trim that apply on top of each
member's own settings. A group is not a bus: it adds no return channel and no
signal routing.

## Behaviour

- **Creating a group.** Select two or more track headers (Cmd/Ctrl- or
  Shift-click adds to the selection). A floating "N tracks selected · Group"
  bar appears; its button, the palette command *Group Selected Tracks* and
  Cmd/Ctrl+G all create the group. The group is named `Group N` (N = existing
  groups + 1), its colour cycles through `GroupIdentityColor::all()`, the
  selected tracks become its members in selection order, and the track
  selection is cleared. Fewer than two selected tracks: nothing happens (the
  palette shows the command as unavailable).
- **Ids.** A group id comes from the same allocator as track ids
  (`Resonance::allocate_track_id`), so a group can never share an id with a
  track. A project load advances the counter past saved group ids.
- **Header row.** Each group gets a 60 px header row (`GROUP_HEADER_HEIGHT`)
  above its members: caret, colour swatch, bold name, `N trk` count, a level
  trim slider with a dB readout, and macro M / S buttons, over a faint wash of
  the group colour.
- **Member headers** show a 3 px rail in the group colour on the left edge
  (two rails for a member of a nested group) and are indented.
- **Collapse.** The caret, or a click on the group's lane in the canvas,
  toggles `is_collapsed`. A collapsed group's member rows disappear from both
  the header column and the canvas, and the group lane draws every member clip
  flattened into one tinted overview strip
  (`view/timeline/draw/group_overview.rs`). Fold state is saved with the
  project and is an undoable edit.
- **Macro mute / solo.** Toggling a group's M or S never writes a member's own
  `muted`/`soloed` flag. The engine receives each member's *effective* state
  (own flag OR any containing group's macro flag), so clearing the group macro
  restores each member exactly. Members affected show a pink "via group" mute
  chip or an amber "via group" solo chip. A group solo dims ungrouped tracks
  through the engine's normal solo logic. Because the effective state is held
  in the engine's track state, offline bounces see it too.
- **Level trim.** The slider writes `macro_level` (1.0 = unity). It is saved,
  undoable (a drag coalesces into one undo entry) and shown in the Arrange
  header and the mixer group strip, but **it does not change the audio**:
  nothing sends it to the engine. See the intent below.
- **Nesting.** One level deep only. `set_nesting_parent` refuses a parent that
  itself has a parent; the registry also breaks membership cycles found in a
  loaded project (`break_membership_cycles`), which used to overflow the
  stack.
- **Deleting a track** removes it from every group's member list
  (`engine_events/tracks.rs`).
- **Membership drag-and-drop** (drag a track onto a group to join, onto open
  space to leave; drag a group onto a group to nest) exists as reducers only.
  No view emits `GroupMessage::StartMembershipDrag`, so a user cannot do it
  today.
- **Not built:** ungroup / delete a group, rename a group, change its colour,
  reorder members, any control-API or MCP access to groups.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model / persistence | `resonance-common/src/track_group.rs`, `resonance-common/src/group_identity.rs` | `TrackGroup`, `MACRO_LEVEL_UNITY`, `GroupIdentityColor` |
| Registry | `resonance-app/src/state/track_group_registry.rs` | `TrackGroupRegistry`, `create_group_from_selection`, `effective_mute`/`effective_solo`, `is_track_hidden_by_collapse`, `set_nesting_parent`, `get_all_member_ids`, `from_saved` |
| Reducers | `resonance-app/src/update/group.rs` | `GroupMessage`, `toggle_collapse`, `toggle_macro_solo`, `toggle_macro_mute`, `set_macro_level`, `apply_membership_change` |
| Own mute/solo composing with the macro | `resonance-app/src/update/track.rs` | `effective_mute` / `effective_solo` calls when a track's own M/S toggles |
| Project load / undo replay | `resonance-app/src/update/project_io/replay/restore.rs`, `update/project_io/reconcile/` | `restore_track_groups` |
| Row layout | `resonance-app/src/state/arrange_layout.rs` | `ArrangeRowKind::GroupHeader`, collapsed-member omission |
| Header view | `resonance-app/src/view/track_header/group_header.rs`, `view/track_header/track.rs` | `view_group_header`; rails and "via group" chips |
| Canvas | `resonance-app/src/view/timeline/draw/group_overview.rs`, `view/timeline/input/pointer.rs` | overview strip; group-lane click → `ToggleCollapse` |
| Selection bar | `resonance-app/src/view/selection_bar.rs` | floating "Group" pill |
| Mixer reflection | `resonance-app/src/view/mixer/group_strip.rs` | group strip with the same colour, caret, trim (mixer area) |

Invariants: a track is a member of at most one group (a join detaches it from
its old group first). The macro cascade must go through
`TrackGroupRegistry::effective_*` on every path (live toggle, own-flag toggle,
project restore, undo diff) so they cannot disagree.

## Surfaces

- **GUI:** Arrange track-header column (group rows, rails, chips), the
  floating selection bar, Cmd/Ctrl+G, the command palette (*Group Selected
  Tracks*), click on a group lane in the canvas to fold. Groups also show in
  the Mixer (see [channel-strips-and-inspector.md](../mixer/channel-strips-and-inspector.md)).
- **Control API / MCP:** none. `song_tracks` and friends do not report groups.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Registry, cycles, delete cleanup | `resonance-app/tests/timeline/track_group_cycles.rs`, `tests/timeline/track_delete_cleanup.rs` | `./scripts/run-tests.py -p resonance-app` |
| Creation, macros, level, drag reducers, id counter | `resonance-app/tests/mixer/group_creation_from_selection.rs`, `group_macro_mute.rs`, `group_macro_solo.rs`, `group_macro_level.rs`, `group_membership_drag.rs`, `group_id_after_load.rs` | same |
| Layout and hit-testing | `resonance-app/tests/timeline/arrange_layout.rs`, `timeline_group_hit_test.rs`, `timeline_group_lane.rs` | same |
| Persistence | `resonance-app/tests/io/project_track_groups_persist.rs` | same |
| Goldens | `tests/snapshots/group_header_*`, `group_identity_rail_*`, `group_member_track_header_via_group_*`, `timeline_group_lane_*`, `track_header_group_row_renders_inline-wgpu.png`, `selection_bar_*` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test mixer` (or `--test timeline`) |

Not covered: that a group mute/solo is audible in a bounce (only the engine
command is asserted); `group_macro_level.rs` pins the state write, not any
gain, because there is none.

## Decisions and deviations

- **Group header is a real arrange row**, sharing the track-id space, rather
  than a separate overlay — so the layout, virtualization and hit-testing all
  treat it like any other row.
- **Fold state is project data** (undoable, saved), unlike panel carets. This
  follows the design.
- **Deviation:** the design's drag-to-join / drag-out / drag-group-header
  gestures and the empty-state explainer card were not built in the view; the
  reducers for the drags exist and are tested.
- **Deviation:** the level trim was specified to scale members' output in live
  mix and bounce. Only the stored value exists (`update/group.rs`
  `set_macro_level` says the engine side "lands with the shared group→engine
  integration", which has not happened).

## Known gaps

- Level trim has no effect on audio; membership drag is not reachable from the
  UI; no ungroup/rename/recolour; no control API — see
  [../../intents/timeline/track-groups-completion.md](../../intents/timeline/track-groups-completion.md).
- Tracks cannot be reordered, inside or outside a group — see
  [../../intents/timeline/track-reordering.md](../../intents/timeline/track-reordering.md).
