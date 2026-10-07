# Finish track groups

> Status: **partly built**
> Area: timeline (with a mixer and control-api slice)
> Source: `docs/intents/prototypes/track-grouping-folder-tracks/DESIGN.md` (epic #36, doc #200); gaps found in code on 2026-10-07

## Why

Groups can be created, folded, muted and soloed, but several things the
design promised do not work:

- The group **level trim** slider moves and is saved, but changes nothing you
  can hear — a user who pulls it down expects the group to get quieter.
- Membership can only be set at creation time. There is no way to add a track
  to an existing group, take one out, nest a group, or ungroup.
- A group cannot be renamed, recoloured or deleted.
- An agent cannot see or edit groups at all.

## What it would do

- The trim scales every member's output by `macro_level` in live playback and
  in every offline render (bounce, freeze, stems), without touching member
  faders; unity restores the exact previous sound.
- Drag a track header onto a group header to join it, onto open space to
  leave; drag a group header onto another to nest it (one level), with the
  drop-target highlight and destination chip from the design.
- Group header context menu: Rename, Colour, Ungroup (members stay, group
  goes).
- Control API: groups listed in `song.tracks`; methods to create, rename,
  set members, set macro mute/solo/level, ungroup — one MCP tool each.

## What exists to build on

- `resonance-common/src/track_group.rs` — `TrackGroup` already carries
  `macro_level`, `nesting_parent`, ordered members.
- `resonance-app/src/update/group.rs` — `start_membership_drag`,
  `update_membership_drag`, `drop_membership`, `apply_membership_change`
  are implemented and tested (`tests/mixer/group_membership_drag.rs`); only
  the view emitters are missing. `GroupMessage::SetMacroLevel` writes the
  value.
- `TrackGroupRegistry::remove_group`, `remove_member`, `set_nesting_parent`.
- The effective mute/solo path (`effective_mute`/`effective_solo` →
  `AudioCommand::SetTrackMute/Solo`) is the pattern for an effective gain.
- Feature doc: [../../features/timeline/track-groups.md](../../features/timeline/track-groups.md).

## Open decisions

1. **Where the trim is applied.** Recommended: the app sends each member an
   effective group gain (product of containing groups' `macro_level`) as a
   separate engine gain stage, so the member's own volume and its automation
   stay untouched. Alternative: fold it into the volume sent to the engine —
   simpler, but then volume automation and the trim fight.
2. **Ungroup with nested children.** Recommended: children move up to the top
   level rather than being deleted.
3. **Control-API shape.** Recommended: `group.create(track_ids, name?)`,
   `group.set_members`, `group.set_macro`, `group.rename`, `group.delete`;
   groups appear as their own list in `song.tracks`.

## Slices

1. Trim audible: engine gain stage + app send on `SetMacroLevel`, project
   restore and undo diff; bounce/freeze honour it; tests in
   `resonance-audio/tests/mixer` and `resonance-app/tests/mixer/group_macro_level.rs`
   asserting a level change, not just the stored value.
2. Membership drag in the header column (view emitters + drop visuals +
   golden).
3. Group context menu: rename, recolour, ungroup (+ undo tests).
4. Control API + MCP tools, with `agent_plugin_lockstep` updated.

## How we'd know it works

- A render with the trim at 0.5 measures −6 dB on members versus unity, and
  unity is bit-identical to no group.
- Dragging a header onto a group in a headless canvas test changes the
  registry; goldens for the drop highlight.
- Undo restores every one of these edits in one step.
