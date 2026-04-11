# Deferred refactors — status

Four architectural refactors were identified during the review of
2026-04-11. Their progress on this branch:

## ✅ #2 — Timeline event-handler split — `77a1253`

`timeline.rs` (977 LOC) was split into:
- `timeline.rs` — shrinks to dispatcher + per-event handler methods
  (`handle_wheel`, `handle_press`, `handle_move`, `handle_release`,
  `handle_key`, `report_viewport`) on `TimelineCanvas`.
- `timeline/hit_test.rs` — pure hit-testing helpers shared between
  audio and MIDI clip lanes (clip_rect, hit_test, sorted_arrange_tracks).
- `timeline/scrollbar.rs` — `ScrollbarRects` + `scroll_from_thumb_pos`
  used by both axes, removing four copies of clamp + ratio math.

## ✅ #3 — `Resonance` god-object split — `a062f99`

~40 flat fields on `Resonance` moved into six sub-structs defined in
`state.rs`:
- `TransportState` — playhead/tempo/metronome/loop range
- `ArrangeViewport` — zoom, scroll, viewport/content size
- `ClipInteractionState` — selections, drag/trim, open MIDI editor
- `ProjectIoState` — project_path, save/load, bouncing
- `MixerUiState` — plugin selection, mixer menus, settings
- `TrackRegistry` — tracks, busses, id counters, `with_track_mut`
  helpers

Top-level `Resonance` keeps `engine`, `sample_rate`, `master_volume`,
`input_devices`, `available_plugins`, `error_message`, `view_mode`,
`clips`, `midi_clips`, and `compose` — cross-cutting fields that don't
sit inside a single concern.

## ✅ #1 — `Message` enum sub-enums — `77bdc39`

~110 flat `Message` variants regrouped into ten sub-enums mirroring the
sub-state layout: `TransportMessage`, `TrackMessage`, `BusMessage`,
`ClipMessage`, `MidiClipMessage`, `MidiEditorMessage`, `PluginMessage`,
`ViewportMessage`, `ProjectIoMessage`, `UiMessage`. `Message::Tick`
stays top-level to avoid wrapping cost on the hot path. `ComposeMessage`
was already in this shape and was not touched.

## ✅ #4 — `engine.rs` split — complete

First pass (`b4a8cc4`): extracted the two handlers with no shared
mutable state:
- Renamed `engine.rs` → `engine/mod.rs` so submodules can sit alongside.
- New `engine/scan.rs` — `ScanPlugins` handler.
- New `engine/bounce.rs` — `BounceToWav` handler (pure read).

Second pass: extracted every remaining handler via a
`HandlerCtx` / `HandlerState` plumbing pass. `engine/mod.rs` shrank
from 1497 → 343 LOC and now contains only `AudioEngine::new` and the
public API.

- `engine/thread.rs` — `HandlerCtx<'_>` (shared Arcs + channels),
  `HandlerState` (id counters, `rec`, `bundles`, `active_imports`),
  and the `engine_thread` dispatch loop.
- `engine/transport.rs` — Play/Stop/Pause/Record/SeekTo, BPM/time
  signature/metronome, and loop range.
- `engine/tracks.rs` — audio track CRUD + sub-track creation,
  volume/pan/mute/solo/arm/mono/monitor, input-device routing,
  master volume, clear-all.
- `engine/clips.rs` — audio clip import (spawns decode thread),
  move/trim/delete, direct load, bulk export.
- `engine/midi.rs` — instrument track, MIDI clip/note CRUD,
  live note-on/off.
- `engine/plugins.rs` — track plugin CRUD, set-param, GUI open/close,
  state save/load (single + bulk). `cmd_tx_retry` is threaded through
  for lock-retry on every path that touches a plugin instance.
- `engine/busses.rs` — bus CRUD, volume/pan/mute/name, track→bus
  routing, bus-plugin CRUD. Reuses `ensure_bundle` / `resolve_plugin_id`
  from `plugins`.

Handlers are free functions that take `&HandlerCtx` + `&mut HandlerState`
— no hidden state, no trait indirection. Each command variant maps to
a single `handle_*` call in `thread::dispatch`, so the match arms stay
readable at a glance.
