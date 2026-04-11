# Deferred refactors

Four architectural refactors identified during the review of 2026-04-11 that
were deemed too risky to bundle into a single pass. Each one is worth doing on
its own dedicated branch with thorough integration testing (especially audio
playback, recording, and project save/load round-trips).

Ordered roughly by risk/value tradeoff — easier wins first.

---

## 1. Split `Message` enum into sub-enums

**Scope:** `resonance-app/src/message.rs` + every handler call site across the
app crate.

**Goal:** Flatten the monolithic `Message` enum (~110 variants in one file)
into per-concern sub-enums so features can be added without touching a shared
enum that everything imports.

```rust
pub enum Message {
    Compose(ComposeMessage),     // already exists — extend the pattern
    Transport(TransportMsg),
    Track(TrackMsg),
    Bus(BusMsg),
    Clip(ClipMsg),
    MidiClip(MidiClipMsg),
    MidiEditor(MidiEditorMsg),
    Plugin(PluginMsg),
    Viewport(ViewportMsg),
    ProjectIo(ProjectIoMsg),
    Ui(UiMsg),  // settings, menus, errors
}
```

**Why risky:** touches every `view/*.rs` file's `.on_press(Message::...)`
calls, every `update/*.rs` handler, and the top-level `update()` dispatch.
Mechanical but huge diff.

**Pairs well with:** the god-object split (#3) — each sub-enum naturally maps
to one of the sub-state structs.

**Estimated effort:** ~1 day of mechanical editing + careful testing of every
interaction (transport, clip drag, plugin param changes, save/load).

---

## 2. Timeline event-handler split

**Scope:** `resonance-app/src/timeline.rs` (currently ~977 LOC after the
Phase 6 extraction of `timeline_draw.rs`).

**Goal:** Split the 280-line `update()` match arm and de-duplicate the audio
clip vs MIDI clip hit-test loops.

Concrete plan:

- Factor hit-testing into `timeline/hit_test.rs` with pure functions:
  ```rust
  pub fn hit_test_clip(
      pos: Point,
      clip_rect: Rectangle,
  ) -> HitKind { Trim(ClipEdge), Move(f32), Miss }
  ```
  Use it for both `self.clips` and `self.midi_clips` with a common iter
  adapter that yields `(clip_id, clip_rect, ClipKind::{Audio,Midi})`.
- Extract `timeline/scrollbar.rs` with `Scrollbar::from_rects(...)` helpers
  that wrap the `h_scrollbar_rects`/`v_scrollbar_rects` + drag-grab math
  (currently duplicated between press/move handlers for H and V).
- Split `canvas::Program::update` into per-event methods:
  `handle_wheel`, `handle_press`, `handle_move`, `handle_release`,
  `handle_key`.

**Why risky:** `TimelineState` has state-coupling with the event dispatcher
(`dragging_loop`, `clip_interaction`, `h_scrollbar_grab`, `v_scrollbar_grab`,
`last_midi_click`). Splitting needs careful tracking of which state lives
where.

**Estimated effort:** ~half-day. Low feature risk since the logic is
well-scoped, but every clip-drag / trim / scroll interaction needs manual
smoke-test.

---

## 3. Split `Resonance` god-object into sub-state structs

**Scope:** `resonance-app/src/main.rs` (the `Resonance` struct, ~50 fields),
with cascading changes through `update.rs`, `view/*.rs`, `engine_events.rs`,
and every handler that touches state.

**Goal:** Group related fields into owned sub-structs:

- `TransportState { playing, recording, recording_start_sample, playhead,
  loop_*, dragging_loop, bpm, bpm_input, time_sig_*, metronome_enabled,
  precount_bars }`
- `ArrangeViewport { zoom, scroll_offset, scroll_offset_y, viewport_width,
  timeline_content_width, timeline_content_height }`
- `ClipInteractionState { selected_clip, selected_midi_clip, clip_drag,
  clip_trim, midi_clip_drag, midi_clip_trim }`
- `ProjectIo { project_path, save_state, loading, pending_load }`
- `MixerUiState { selected_plugin, collapsed_sub_track_parents,
  add_track_menu_open, settings_open }`
- `TrackRegistry { tracks, busses, next_track_order, next_bus_order,
  next_sub_track_id }` with `find_track_mut(id)`, `any_solo()`, etc.

`Resonance` then holds a handful of these sub-structs and becomes a thin
container.

**Why it's the biggest conceptual win:** every current handler has to borrow
the whole app because the state is flat. Sub-structs let handlers take only
what they need, which unlocks:
- Moving whole update handlers into the sub-state's own `impl` blocks.
- Handlers taking `&mut TransportState` instead of `&mut Resonance`, so they
  can run independently and be tested.
- Removing most of the `self.with_track_mut(...)` closure dance in favour of
  `self.tracks.find_mut(id)`.

**Why risky:** every `self.field` access in `update.rs`, `view/*.rs`,
`engine_events.rs`, `update/*.rs` has to become `self.sub.field` or
`r.sub.field`. Expect ~500+ individual edits. Borrow-checker gets strict when
a handler needs two sub-structs at once (e.g. `Tick` touches transport +
tracks + viewport).

**Pairs well with:** the `Message` split (#1). Do them together on a long
branch.

**Estimated effort:** 2-3 days. This is the foundational change that makes
everything else easier afterward.

---

## 4. Split `engine.rs` into per-concern handler modules

**Scope:** `resonance-audio/src/engine.rs` (still 1873 LOC after Phase 7).

**Goal:** The `engine_thread` loop is one giant `loop { match cmd { /* every
variant */ } }`. Break the match arms into per-concern handler modules:

```
engine/
├── mod.rs         // AudioEngine struct, spawn, build_stream, shared state
├── thread.rs      // engine_thread loop, dispatches to handlers by concern
├── transport.rs   // Play/Stop/Pause/Seek, recording start/finalize
├── clips.rs       // Import, Move, Trim, Delete, LoadClipDirect,
│                  // ExportAllClipData
├── midi.rs        // CreateMidiClip, Load..., Move, Trim, Delete, note edit
├── plugins.rs     // AddPlugin, ScanPlugins, SetParam, Save/LoadState,
│                  // OpenPluginEditor
├── busses.rs      // AddBus, RemoveBus, volume/pan/mute, routing
├── bounce.rs      // BounceToWav: ~400 lines of offline render
└── scan.rs        // ClapBundle scanning (~115 lines inline today)
```

**Why risky:** This is the most dangerous file to touch in the codebase.
Audio thread / control thread / plugin thread coordination lives here. Any
regression here can produce stuck notes, crashes, or glitchy output that's
hard to reproduce.

**Specific gotchas:**
- `engine_thread` holds `next_track_id` / `next_clip_id` / `next_plugin_id`
  as local mutable state. Handlers need to accept `&mut u64` or be grouped
  into an `IdCounters` struct.
- `rec: RecordingState` and `bundles: Vec<ClapBundle>` are also local state
  that's touched across handlers.
- `cmd_tx_retry` for retrying plugin editor open/close when the audio thread
  holds the lock must stay wired through to each plugin handler.
- The bounce handler (~400 LOC) does its own mini-mix loop — that's a
  natural standalone file but touches every part of the engine.

**Estimated effort:** 2-3 days with careful manual testing of playback,
recording, plugin scanning, plugin param automation, and bounce-to-WAV.

**Pairs well with:** nothing — do this one on its own branch.

---

## Suggested order

If tackling all four:

1. **#2 (timeline event-handler split)** — half-day, lowest risk, nice warmup.
2. **#3 (god-object split)** — 2-3 days, foundational.
3. **#1 (Message enum split)** — 1 day, natural follow-up to #3 since the
   sub-enums map to the sub-states.
4. **#4 (engine split)** — 2-3 days, independent, most dangerous, save for
   last when you have free integration-test time.
