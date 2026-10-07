# Drum pad ↔ piece ↔ MIDI note map over MCP

> Status: **idea**
> Area: control-api (wire + tools); the data lives in
> [plugins-instruments](../../features/plugins-instruments/)
> Source: feature request 2026-10-07, found while writing a prog-metal
> drum part for SM Drums via MCP (was
> `docs/intents/control-api/drum-pad-map-mcp.md`).
> Re-checked against master @ `8eac71b3` on 2026-10-07: still open.

## Why

An agent writing drum MIDI over MCP cannot find out which note plays
which piece of the loaded kit. Nothing on the control surface links the
three:

- `drum_kits_list` returns `piece_names` (display names only, e.g.
  "China", "Ride 17 Bell", "Tom 3"): no note, no pad index
  (`resonance-control/src/methods/drum_kits.rs`,
  `DrumKitEntry::piece_names`).
- `track_plugin_params` on `com.resonance.drums` exposes `Pad 0..29
  Volume / Pan / Output / Choke / Articulation / …`
  (`plugins/resonance-drums/src/params.rs`, `PadParams::new`) but not the
  pad's MIDI note or the piece it currently holds. `Pad 1 Articulation`
  reads "ring ring" and `Pad 10 Articulation` "tom 2": the only hints, and
  only on pads with an alternate.
- `song_notes` gives pitch names (`F#2`), which say nothing about the kit.

The information exists, but only to someone who reads files and Rust
source: `KitMeta::pads` (`_meta.pads.<piece>.note`,
`resonance-common/src/drumkit_library/manifest.rs`), the GM contract in
`resonance-common/src/drum_map.rs` (`GM_PADS`), and the resolution in
`plugins/resonance-drums/src/pad_map.rs` (`KitPads::resolve`). The
session that found this had to read `drum_samples.json` from
`~/.local/share/resonance/drumkits/`.

Consequences:

- An agent guesses GM, which is wrong for every non-GM slot (SM Drums
  puts Crash 13 on 29, Crash 15 on 58, Ride 17 on 59, Ride 17 Bell on 55,
  Snare Hybrid 1 on 21, Hi-Hat Closed→Open on 24, Foot Splash on 27).
- A kit whose `_meta.pads` remaps GM notes plays the wrong drums
  silently.
- Per-pad mixing (`Pad N Volume`, `Pad N Output`) cannot be aimed at "the
  china" or "the floor tom" without knowing N.

## What it would do

1. **`drum_kits.list` gains a `pads` array per kit** (alongside or
   replacing `piece_names`):
   `[{piece: "China", name: "China", note: 52, pad: 21, articulation_of?: "...", port?: "Cymbals", choke?: ...}]`
   — the resolved mapping as `pad_map.rs` computes it, not just the raw
   `_meta.pads` hints, so GM fallback and articulation pairing are
   included. Pieces the kit has but no pad plays are listed with
   `note: null`.
2. **A track-scoped view of the loaded kit**, e.g.
   `drums.pad_map {track_id}` (or a `pads` block in `track.plugin_params`
   for `com.resonance.drums`): per pad index → note, piece key, display
   name, active articulation, output port, choke group. This is what an
   agent needs right before writing notes or setting `Pad N *` params,
   and it reflects runtime changes (articulation switches, a different
   `kit_select`).
3. **Pad params named by piece**, in their `text` or a new field, e.g.
   `Pad 21 Volume` carrying `piece: "China"`, so a
   `track_set_plugin_param` for "china volume" needs no index lookup.
4. (Nice to have) `song.notes` on a drums-track clip adds the piece name
   next to `pitch_name` (`52 → "China"`).

Done when an MCP client can, with tool calls only, write a part that
hits SM Drums' Ride 17 Bell (note 55) and Crash 13 (note 29) correctly,
and the same for a kit whose `_meta.pads` overrides a GM note.

## What exists to build on

- `drum_kits.*` methods and handler:
  `resonance-control/src/methods/drum_kits.rs`,
  `resonance-app/src/update/control/drum_kits.rs` (reads the same library
  files the plugin does, via `resonance_common::drumkit_library`).
- Pad resolution: `plugins/resonance-drums/src/pad_map.rs` (`KitPads`,
  `KitPad`, `piece_for`, `output_group`, `choke_group`,
  `KitPadsHandle::articulation_text`).
- GM contract: `resonance-common/src/drum_map.rs` (`GM_PADS`).
- Plugin-param view and per-param metadata:
  `track.plugin_params` (`resonance-control/src/methods/track.rs`).
- The drumming skill's `references/drum-map.md` documents the built-in
  map by hand today.

## Open decisions

1. Where the resolved map is computed for `drum_kits.list`: the app does
   not link plugin crates (arch rule), so `KitPads::resolve` must move to
   `resonance-common` or be mirrored there. Recommended: move the pure
   resolution into `resonance-common::drumkit_library` and have the
   plugin call it.
2. Track-scoped view: a new `drums.pad_map` method vs. a `pads` block on
   `track.plugin_params`. Recommended: a block on `track.plugin_params`
   for `com.resonance.drums`, so there is no new namespace and the agent
   already calls it before setting `Pad N *`.
3. How the app learns the runtime state (active articulation, loaded
   kit) of a running drums instance: from the plugin's param texts, or a
   new engine query. Recommended: derive from `kit_select` + articulation
   params plus decision 1's shared resolver, no new engine command.

## Slices

1. `pads` on `drum_kits.list` (wire field, handler, MCP description,
   test in `resonance-app/tests/control/control_drum_kits.rs`).
2. Track-scoped pad map on the loaded kit (wire, handler, tool, test).
3. Piece name on `Pad N *` params.
4. Piece names in `song.notes` for drums tracks.
5. Update the drumming skill to read the map instead of trusting
   `drum-map.md`; the lockstep test keeps the tool names honest.

## How we'd know it works

- `control_drum_kits.rs` asserts SM-Drums-shaped fixtures resolve 55 to
  Ride 17 Bell and 29 to Crash 13, and a `_meta.pads` override wins over
  GM.
- A live MCP session writes a part using only tool output and the right
  pieces sound.
