# Insert and remove bars

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none in `docs/history/specs/`; ba doc #275 P2 (the shift) and ba todo #1388 / doc #287 (carrying the global tracks), cited in code comments.

One edit that opens or closes whole bars at a point in the song and moves
everything after the cut with it: audio clips, MIDI clips, section
placements, markers, automation breakpoints, and the tempo and signature
events. It exists so a restructure is one undoable transaction instead of a
few hundred per-object moves. Today it is reachable only through the
control API.

## Behaviour

- **Insert `count` bars at 1-based `at_bar`.** Everything that starts at or
  after the cut moves later by `count` bars. The new bars take the meter in
  force *before* the cut (a signature event sitting on the cut moves with the
  music it was written for).
- **Remove `count` bars at `at_bar`.** Anything that *starts* inside the span
  is deleted: audio clips, MIDI clips and section placements (placements go
  through `purge_placement_outputs`, which also drops their derived clips and
  vocal audio). Everything after the span moves earlier.
- **Straddlers stay put.** An object that starts before the cut keeps its
  position and length even if it plays across the cut; nothing is stretched or
  trimmed.
- **Positions move musically.** Each absolute position is read as a tick under
  the old map, displaced by the tick span of the bars, and written back under
  the new map (the same `musical_anchors` / `reanchor_to_tempo` machinery a
  tempo change uses). Material *before* the cut is re-anchored too, because
  moving a tempo event reshapes the ramp leading up to it.
- **Placements** move by whole bars (`start_bar ± count`).
- **Tempo and signature events** move by whole bars, with three rules:
  - the bar-0 event never moves;
  - an event inside a removed span is clamped onto the cut, not dropped;
  - if two events of one kind land on one bar, the later one wins (the other
    is counted in `*_events_removed`).
- **Markers and automation points inside a removed span are kept** and stay
  where they were; automation lanes are re-sorted afterwards
  (`resort_automation`).
- The playhead and loop range are not displaced (they are re-anchored to their
  musical position, like a tempo change).
- The shift distance reported (`shift_samples`) is **measured** after the
  rebuild, not predicted, since moving tempo events changes bar lengths.
- After the shift the transport tempo / meter display is re-read at the
  playhead (`sync_transport_to_map`).
- A frozen track whose content fingerprint changed goes stale after the edit
  (see [track-freeze-ui.md](./track-freeze-ui.md)).

**Wire rules (control layer).**
- `at_bar` ≥ 1, `count` ≥ 1, and the span's last bar within `MAX_BARS`.
- `insert_bars` is refused if it would push the song's last bar past
  `MAX_BARS` (FU-M5a).
- `remove_bars` without `confirm: true` is refused with `needs_confirmation`
  whenever something starts inside the span, naming how many audio clips,
  MIDI clips and placements would go.
- One call records exactly one undo entry.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Shift engine | `resonance-app/src/update/arrangement.rs` | `ArrangementMessage::{InsertBars, RemoveBars}`, `insert_bars`, `remove_bars`, `removal_casualties`, `shift`, `carry_events`, `EventShift`, `resort_automation`, `sync_transport_to_map` |
| Musical anchors | `resonance-app/src/update/tempo_reanchor.rs` | `MusicalAnchors::displace_from`, `musical_anchors`, `reanchor_to_tempo` |
| Outcome record | `resonance-app/src/state/arrangement_shift.rs` | `ShiftOutcome` (held in `UiTransientState::last_arrangement_shift`) |
| Dispatch | `resonance-app/src/update.rs` | `Message::Arrangement` arm writes `last_arrangement_shift` |
| Control handlers | `resonance-app/src/update/control/arrangement.rs` | `check_range`, `check_song_end_after_insert`, confirm gate |
| Wire types | `resonance-control/src/methods/arrangement.rs` | `InsertBarsParams`, `RemoveBarsParams`, `ShiftResult` |
| Undo label | `resonance-app/src/undo/describe.rs` | `InsertBars` / `RemoveBars` descriptions |

Invariant: capture the cut and every anchor under the **old** map, move the
events, rebuild, then write positions back. Reordering these steps puts
material off the grid.

## Surfaces

- **GUI:** none. There is no menu item, palette command or ruler gesture for
  inserting or removing bars. See
  [intent: insert/remove bars in the GUI](../../intents/timeline/arrangement-bars-gui.md).
- **Control API / MCP:**
  - `arrangement.insert_bars` → `mcp__resonance__arrangement_insert_bars`
  - `arrangement.remove_bars` (`confirm`) → `mcp__resonance__arrangement_remove_bars`
  - Result (`ShiftResult`): `shift_samples`, `audio_clips_moved`,
    `midi_clips_moved`, `placements_moved`, `markers_moved`,
    `automation_points_moved`, `tempo_events_moved`, `signature_events_moved`,
    `tempo_events_removed`, `signature_events_removed`, `clips_deleted`,
    `placements_deleted`, `revision`.
- **Agent skills:** `resonance-agent-plugin/skills/song-structure` and
  `skills/drumming` describe the global-track carry rules.

## How it's tested

| What | Where | Run |
|---|---|---|
| Insert / remove, confirm gate, deletions, undo, bounds | `resonance-app/tests/control/control_arrangement_bars.rs` (11 tests) | `./scripts/run-tests.py -p resonance-app` |
| Tempo / signature carry, clamp, last-wins, bar-0 pin | `resonance-app/tests/control/control_arrangement_global_events.rs` (13 tests) | same |
| Tick ↔ sample round-trip across maps | `resonance-audio/tests/types/tempo_reanchor_math.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Frozen track goes stale after a shift | `resonance-app/tests/plugins/freeze_stale_on_content.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered: chord-track regions (they are not moved — see gaps); takes
(take groups are not moved — see gaps); vocal-lane data beyond what placement
purge removes.

## Decisions and deviations

- Removing deletes only what *starts* inside the span; straddlers are left
  whole rather than split, so the edit never silently trims audio.
- Global-track events clamp instead of being dropped, so the music after the
  splice keeps the tempo and meter it was written under.
- The control layer asks for confirmation using the same convention as
  `track.delete`.

## Known gaps

- No GUI — [arrangement-bars-gui](../../intents/timeline/arrangement-bars-gui.md).
- The chord track (`resonance-app/src/chord_track.rs`, sample-anchored regions
  and key changes) is not shifted; after an insert the chords sit on the old
  samples. Tracked in
  [tempo-edits-keep-content-on-bars](../../intents/timeline/tempo-edits-keep-content-on-bars.md).
- Take groups (`Resonance::take_groups`, slot ranges in samples) are not in
  `MusicalAnchors` and do not move with the music.
