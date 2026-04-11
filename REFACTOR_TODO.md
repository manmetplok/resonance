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

## 🟡 #4 — `engine.rs` split — partial (`b4a8cc4`)

First pass extracted the two handlers with no shared mutable state:
- Renamed `engine.rs` → `engine/mod.rs` so submodules can sit alongside.
- New `engine/scan.rs` — `ScanPlugins` handler (115 LOC).
- New `engine/bounce.rs` — `BounceToWav` handler (281 LOC, pure read).

`engine/mod.rs` shrank from 1873 to 1497 LOC. The remaining transport /
clips / midi / plugins / busses handlers still live inline in `mod.rs`
because they share `next_track_id` / `next_clip_id` / `next_plugin_id` /
`rec: RecordingState` / `bundles: Vec<ClapBundle>` with each other and
need a `HandlerState` plumbing pass before they can be safely extracted.
That second pass is deferred as its own branch — the audio engine is
the most dangerous file in the repo and the safer to-extract handlers
are already out.

### Remaining work for #4 (future branch)

- Define `HandlerCtx<'_>` (Arcs + event_tx) and `HandlerState`
  (id counters, rec, bundles, active_imports) in `engine/thread.rs`.
- Move transport (Play/Stop/Pause/Record/Seek/tempo/loop) handlers
  into `engine/transport.rs`.
- Move audio clip CRUD into `engine/clips.rs`.
- Move MIDI clip/note/instrument handlers into `engine/midi.rs`.
- Move plugin CRUD + state + editor into `engine/plugins.rs`
  (critical: `cmd_tx_retry` must stay threaded through for lock-retry).
- Move bus CRUD + routing into `engine/busses.rs`.
- Shrink `engine/mod.rs` to just `AudioEngine::new` + dispatch.

Each of these needs hands-on audio testing (playback, recording,
plugin scanning + param automation, multi-output plugins, save/load
round-trip) after extraction.
