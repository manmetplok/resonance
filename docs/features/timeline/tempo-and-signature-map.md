# Tempo and time-signature map

> Verified against master @ `6137b6fb` on 2026-10-15.
> Spec history: none in `docs/history/specs/`; the control surface came from ba doc #286 (epic #205), the re-anchoring from ba doc #275 P1.4. Both are only cited in code comments.

The song's tempo and meter live on two bar-addressed lists, drawn as the
Tempo and Signature lanes of the global-tracks shelf at the top of the
Arrange view. A user can add, drag and delete tempo points and add or
re-meter signature changes there; an agent can do the same through the
`global.*` methods. Everything that converts between bars and samples
(ruler, snap, clip placement, the engine's playback position) reads one
`TempoMap` built from these two lists.

## Behaviour

**The model.**
- Two sorted lists on `Resonance`: `tempo_events: Vec<TempoEvent>` and
  `signature_events: Vec<SignatureEvent>` (aliases for the engine's
  `TempoPoint { bar, bpm }` and `SignaturePoint { bar, numerator, denominator }`).
  Bars are **0-based** in state and **1-based** on the wire.
- Each list always has an event at bar 0 (the song's opening tempo / meter).
  It can be edited but not moved or removed: the GUI helpers silently skip
  index 0; the control methods refuse with `invalid_params`.
- **One event per bar.** Adding at an occupied bar replaces that event
  (`AddTempoEvent` upserts since ba todo #1382; `AddSignatureEvent` always did).
  A tempo drag onto an occupied bar leaves the event on its old bar.
- Tempo is clamped to 20–300 BPM (`MIN_BPM` / `MAX_BPM`); a non-finite value is
  rejected (`sanitize_bpm`). Stored tempos that are `null` or out of range load
  as 120 / clamped (`deserialize_bpm`).

**How the map plays.**
- Tempo **ramps linearly** between consecutive events (`bpm_at_bar`,
  `avg_bpm_for_bar`). A tempo event at bar 33 after one at bar 1 is a 32-bar
  accelerando, not a step at bar 33. The math supports steps (two events on
  one bar, "last wins"), but the one-event-per-bar rule means neither the GUI
  nor the control API can create one any more. See
  [intent: tempo step changes](../../intents/timeline/tempo-step-changes.md).
- Bar length counts both halves of the signature: a 7/8 bar is 3.5 quarters
  (`bar_len_ticks`, 480 ticks per quarter).
- The bar table (`TempoMap::rebuild_bar_table`) runs to the last event + 200
  bars, capped at 10 000 bars.

**Display versus project tempo.** `transport.bpm` / `time_sig_*` are display
values that follow the playhead (`sync_tempo_display`). Anything that
describes the project (save, map fallback) reads `project_tempo()`, which
is the bar-0 events (code review ARCH2-10).

**GUI editing (global-tracks shelf).**
- Tempo lane: click a point to start a drag; vertical drag changes BPM
  (1 px = 1 BPM), horizontal drag moves it to another bar (not the bar-0 point).
  Double-click empty space adds a point at the interpolated BPM for that bar.
  The selection follows the dragged point across a re-sort (review VIEW-15).
- Signature lane: click a marker to select it; selecting shows numerator
  (1–16) and denominator (2/4/8/16) pick-lists in the lane header.
  Double-click adds an event using the transport numerator over 4.
- The timeline delete key (`CommandId::TimelineDeleteSelection`) removes the
  selected event (`DeleteSelectedEvent`) when no automation breakpoint is
  selected.
- The transport BPM field (`SetBpmText` + `CommitBpm`) shows the tempo under
  the playhead and rewrites the event governing it (the last one at or before
  the playhead's bar). The time-signature control (`SetTimeSignature`,
  `CycleTimeSignature`) rewrites the **bar-0** event only.

**What moves when the map changes.**
- `CommitBpm` (the transport field and `transport.set_tempo`) re-anchors
  everything musically when the value changes: audio and MIDI clip starts,
  automation breakpoints, markers, the playhead and loop range keep their bar
  and beat (`musical_anchors` → rebuild → `reanchor_to_tempo`). Audio-clip
  *durations* are not stretched.
- Every other map edit — a tempo or signature event added, dragged, edited or
  removed on the shelf or through `global.*`, and any meter change including
  `transport.set_time_signature` — keeps absolute positions in **samples**, so
  material after the change lands on different bars. This is pinned by
  `a_global_tempo_event_keeps_a_lane_on_its_sample`. See
  [intent: tempo edits keep content on its bars](../../intents/timeline/tempo-edits-keep-content-on-bars.md).
- Insert / remove bars carries both lists with the music; see
  [arrangement-bars.md](./arrangement-bars.md).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Map type + math | `resonance-audio/src/types/tempo/{map,conversion,bars,signature}.rs` | `TempoMap`, `TempoPoint`, `SignaturePoint`, `rebuild_bar_table`, `bpm_at_bar`, `bar_to_sample`, `sample_to_abs_tick`, `tick_to_abs_sample`, `bar_len_ticks_at`, `sanitize_bpm` |
| App state | `resonance-app/src/state/global.rs` | `TempoEvent`, `SignatureEvent`, `GlobalTrackKind`, `SelectedGlobalEvent` |
| Update | `resonance-app/src/update/global_track.rs` | `GlobalTrackMessage`, `rebuild_and_send_tempo`, `rebuild_tempo_map`, `project_tempo`, `sync_tempo_display`, `remove_tempo_event`, `remove_signature_event` |
| Transport tempo / meter | `resonance-app/src/update/transport.rs` | `TransportMessage::{CommitBpm, SetTimeSignature, CycleTimeSignature}` |
| Re-anchoring | `resonance-app/src/update/tempo_reanchor.rs` | `MusicalAnchors`, `musical_anchors`, `reanchor_to_tempo` |
| Snap | `resonance-app/src/view/timeline/snap.rs` | `snap_sample_to_grid_tempo` (uses the map's bar table as soon as there is more than one tempo or signature point; review FU-V1a) |
| View | `resonance-app/src/view/timeline/draw/global_tracks.rs`, `view/timeline/input/global_tracks.rs`, `view/track_header/shelf.rs` | `handle_global_track_click`, `view_tempo_lane_header`, `view_signature_lane_header` |
| Control handlers | `resonance-app/src/update/control/global.rs`, `update/control/transport.rs` | `try_handle`, `edit_tempo_event`, `remove_selected`, `validate_bpm`, `validate_time_signature` (in `update/control/mod.rs`) |
| Persistence | `resonance-app/src/project/model.rs` | `ProjectFile::tempo_events`, `signature_events` |

Invariants a change must keep:
- Every mutation goes through `Resonance::update` so the undo classifier sees
  it. `UpdateTempoEvent` is `Skip` (it is the drag step); its undo entry comes
  from the `StartTempoDrag` (`Begin`) / `EndTempoDrag` (`Commit`) bracket, so
  `global.edit_tempo_event` dispatches all three. Removal over the wire selects
  then sends `DeleteSelectedEvent`.
- After touching either list call `rebuild_and_send_tempo` (GUI map + engine
  `SetTempoEvents`).
- Never hand list indices to a client; resolve bar → index right before
  dispatch.

## Surfaces

- **GUI:** Arrange → global-tracks shelf (toggle with the shelf caret,
  `UiMessage::ToggleGlobalTracks`), Tempo and Signature lanes; transport BPM
  field and time-signature control.
- **Control API / MCP:**
  - `global.list_events` → `mcp__resonance__global_list_events`
  - `global.add_tempo_event` → `mcp__resonance__global_add_tempo_event`
  - `global.edit_tempo_event` (`bpm`, `new_bar`) → `mcp__resonance__global_edit_tempo_event`
  - `global.remove_tempo_event` → `mcp__resonance__global_remove_tempo_event`
  - `global.add_signature_event` → `mcp__resonance__global_add_signature_event`
  - `global.edit_signature_event` (no `new_bar`) → `mcp__resonance__global_edit_signature_event`
  - `global.remove_signature_event` → `mcp__resonance__global_remove_signature_event`
  - `transport.set_tempo` / `transport.set_time_signature` (bar-1 event only) →
    `mcp__resonance__transport_set_tempo` / `mcp__resonance__transport_set_time_signature`
  - `song.summary` also carries both lists; its `tempo_bpm` / `time_signature`
    are the values at the playhead.
  - Wire validation: BPM 20–300; numerator 1–32; denominator a power of two
    1–32. `global.list_events` sits behind the mutation gate (answers `busy`
    with no project open).
- **Agent skills:** `resonance-agent-plugin/skills/song-structure` and
  `skills/drumming` (meter changes via `global_add_signature_event`).

## How it's tested

| What | Where | Run |
|---|---|---|
| Map math, ramps, steps, round-trips | `resonance-audio/tests/types/tempo_map.rs`, `tempo_position_to_bars.rs`, `tempo_bar_at_sample_exact.rs`, `tempo_reanchor_math.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Shelf edit cycle + shelf goldens | `resonance-app/tests/compose/global_tracks_edit_cycle.rs`; `tests/snapshots/global_tracks_shelf_*-wgpu.png` | `./scripts/run-tests.py -p resonance-app` |
| Tempo drag selection / one-bar-one-tempo | `resonance-app/tests/timeline/tempo_drag_selection.rs` | same |
| BPM field NaN guard | `resonance-app/tests/timeline/bpm_input_validation.rs` | same |
| Snap against a signature map | `resonance-app/tests/timeline/snap_signature_map.rs` | same |
| `global.*` add / edit / remove / undo / refusals | `resonance-app/tests/control/control_global_events.rs` (56 tests) | same |
| `transport.set_tempo` / `set_time_signature` | `resonance-app/tests/control/control_transport.rs` | same |
| Summary carries the lists | `resonance-app/tests/control/control_song_summary_tempo_map.rs` | same |
| Re-anchor vs. sample-pinned behaviour | `resonance-app/tests/control/control_automation.rs` (`set_tempo_keeps_a_lane_on_its_bar`, `a_global_tempo_event_keeps_a_lane_on_its_sample`) | same |
| Saved tempo is bar-1 tempo | `resonance-app/tests/io/project_tempo_not_playhead.rs` | same |

Not covered: a clip-level test that `transport.set_tempo` keeps audio and MIDI
clips on their bars (only automation is asserted end to end; the math is
covered in `tempo_reanchor_math.rs`); the GUI signature pick-lists; the
10 000-bar table cap.

## Decisions and deviations

- **Addressed by bar, never by index** on the wire, because both lists re-sort
  on every edit.
- **Refuse rather than clamp** on the wire: 500 BPM is `invalid_params`, while
  the GUI drag clamps to the same range.
- **Edit vs add:** `add_*` upserts; `edit_*` and `remove_*` refuse an empty bar
  so a wrong belief about the song is reported, not silently acted on.
- **Only `CommitBpm` re-anchors.** It compares the new value against the
  *display* BPM (the tempo at the playhead) to decide whether to re-anchor,
  but it always writes the bar-0 event. With the playhead in a later tempo
  region, typing that region's tempo changes bar 0 without re-anchoring, and
  typing any other value edits bar 0, not the tempo the field showed.
- The GUI's signature pick-lists offer narrower ranges (1–16 over 2/4/8/16)
  than the wire accepts (1–32 over 1–32).

## Known gaps

- No step tempo changes; every change ramps from the previous event —
  [tempo-step-changes](../../intents/timeline/tempo-step-changes.md).
- Tempo / meter event edits other than the bar-1 tempo leave clips,
  automation, markers and the chord track on their samples —
  [tempo-edits-keep-content-on-bars](../../intents/timeline/tempo-edits-keep-content-on-bars.md).
- The chord track (`resonance-app/src/chord_track.rs`, sample-anchored) is not
  in `MusicalAnchors`, so even `transport.set_tempo` leaves chord regions and
  key changes on their old samples (same intent).
