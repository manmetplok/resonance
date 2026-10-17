# Chord track editing surface

> Stage: **intent**
> Status: **partly built**
> Area: compose
> Source: code reading 2026-10-07 — `ChordTrackMessage` variants with no sender (`update/chord_track.rs`); epic #33 / doc #168 view todo #442.
> Register items covered: FU-U2a (review 2026-10-02).

## Why

The global chord track's model and handlers support adding regions with a
symbol, re-symboling, moving, resizing, pinning and key changes. Only
three are reachable: the palette commands add a **tonic** chord at the
playhead, delete it, and toggle its pin. So a user cannot pin anything but
the tonic, and pinned harmony — the feature's purpose — is mostly
unusable. Agents have no access at all besides `transport.set_key`.

Separately, a song key silently replaces every section's scale for lane
regeneration (`apply_chord_track_harmony`), so `section.set_scale` after
`transport.set_key` has no visible effect on generated parts.

## What it would do

- Click/drag editing of chord regions in the Arrange global-tracks chord
  lane: add, set symbol (text or picker), move, resize, pin, delete; key
  change markers.
- Control methods, e.g. `chord_track.add_region`, `edit_region`,
  `delete_region`, `set_pinned`, `add_key_change`, plus a read view.
- A visible rule for key vs section scale (see decisions).

## What exists to build on

- `resonance-app/src/chord_track.rs` (`ChordTrack`, `key_at`),
  `update/chord_track.rs` (all edit handlers, grid snapping, undo
  classification), `view/timeline/draw/chord_lane.rs` (drawing),
  `tests/compose/chord_track_messages.rs`.
- Feature doc: [chord track](../../features/compose/chord-track.md).

## Open decisions

1. Key precedence: recommended — section scale wins when set, chord-track
   key is the fallback for sections without one; today it is the reverse.
2. Should unpinned regions be generated from section chords (a read-only
   mirror) or be free-standing? Recommended: free-standing, as now.

## Slices

1. Control methods for regions + read view, with tests in
   `tests/control/`.
2. Timeline lane interactions (hit-test, drag, inline symbol edit) with
   goldens.
3. Key precedence change + `song.sections` reporting the effective scale.

## How we'd know it works

An agent can pin `Dm7` over bars 5–6 and the next `generate.part` follows
it; a user can do the same with the mouse; goldens for the edited lane.
