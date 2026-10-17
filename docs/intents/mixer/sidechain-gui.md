# Sidechain routing in the GUI

> Stage: **intent**
> Status: **partly built**
> Area: mixer
> Source: ba todo #1312 (GUI half of #1311); design doc #157; prototype `docs/history/prototypes/external-sidechain-routing/index.html`
> Register items covered: plugin audit P4, C1.

## Why

Key routing works end to end — engine, persistence, undo and six control
methods — but a user in the GUI cannot see or set it. A route set by an
agent is invisible: nothing on the strip or in the CHAIN row says that a
compressor is ducking from the kick, and there is no way to change or
remove it without the control API.

## What it would do

- In the inspector, on a CHAIN row whose plugin declares a key port, a
  "Key" control: a source picker (tracks, sub-tracks, busses; "none —
  internal detector"), an on/off toggle, and the current route shown.
- A visible marker that a slot is keyed (a small "key" glyph on the CHAIN
  row and on the strip slot line), and on the source strip while the route
  is live.
- Refusals worded in place (no key port; the eight-source cap reached).
- Done when a route set in the GUI reads back identically over
  `track.set_sidechain`'s read side, and vice versa, and survives
  save/load and undo.

## What exists to build on

- `PluginMessage::SetPluginSidechain` (undoable, mirrors optimistically)
  and `SidechainState::route_for` — the GUI only needs to emit and read.
- Key-port detection and the "first keyable plugin" rules in
  `resonance-app/src/update/control/sidechain.rs` (`require_key_port`,
  `first_keyable`).
- CHAIN row anatomy and `slot_menu_entries` in
  `resonance-app/src/view/mixer/inspector/chain.rs`.
- Feature doc: [`../../features/mixer/sidechain-routing.md`](../../features/mixer/sidechain-routing.md).

## Open decisions

1. Where the control sits: an inline sub-row under the CHAIN row, or a
   ☰ menu entry opening a card. **Recommended:** inline sub-row, shown only
   for keyable plugins, like the missing-plugin recovery row.
2. Whether to surface the eight-source cap before it bites.
   **Recommended:** yes — refuse in the app (GUI and control API) with a
   plain message instead of letting the engine ignore the route.
3. Whether key gain, key HPF and "audition key" (doc #157, never built)
   belong in this slice. **Recommended:** no; separate intent once the
   basic picker exists. Compressor/gate already have internal detector
   filtering.

## Slices

1. Read-only: key glyph on keyed slots (CHAIN row and strip), source name
   in the row; fingerprint hashing. Golden.
2. Source picker + toggle emitting `SetPluginSidechain`; refusal text;
   app-side source-cap check shared with the control handlers. Tests in
   `resonance-app/tests/mixer/`.

## How we'd know it works

- Mixer-group tests: set/clear from the GUI, read back via `song.tracks` /
  `*_plugin_params`; undo/redo; save/load round trip.
- Goldens of a keyed compressor row and a refused route.
