# Tempo and meter edits keep content on its bars

> Status: **partly built**
> Area: timeline (see `docs/features/timeline/tempo-and-signature-map.md`)
> Source: ba doc #275 P1.4 (built for the bar-1 tempo only); behaviour pinned by `a_global_tempo_event_keeps_a_lane_on_its_sample` in `resonance-app/tests/control/control_automation.rs`

## Why

Only one tempo edit re-anchors the song musically: `TransportMessage::CommitBpm`
(the transport BPM field and `transport.set_tempo`). Every other change to the
map — a tempo event added, dragged, edited or removed on the shelf or through
`global.*`, and any meter change, including `transport.set_time_signature` and
`CycleTimeSignature` — leaves clips, automation and markers on their samples,
so material after the change lands on different bars and stops lining up with
the section placements (which are bar-based and do not drift). The
`song-structure` skill tells agents to set the map before writing material
for this reason.

Separately, the chord track (`resonance-app/src/chord_track.rs`) is
sample-anchored and not in `MusicalAnchors` at all, so even `CommitBpm` and
`arrangement.insert_bars` / `remove_bars` leave chord regions and key changes
on their old samples. Take groups (slot ranges in samples) have the same gap.

## What it would do

Every tempo-map edit keeps each absolute position on its bar and beat, the
way `CommitBpm` does, unless the user explicitly asks for "keep time". The
chord track and take groups move with everything else.

## What exists to build on

- `resonance-app/src/update/tempo_reanchor.rs`: `musical_anchors`,
  `reanchor_to_tempo`, `MusicalAnchors::displace_from`.
- `resonance-app/src/update/arrangement.rs` already runs the
  capture → change map → re-anchor sequence for bar shifts.
- Math tests: `resonance-audio/tests/types/tempo_reanchor_math.rs`.

## Open decisions

1. Default for shelf / `global.*` edits: **keep bars** (recommended; it is
   what `CommitBpm` and bar shifts do). Offer "keep time" as a modifier / param
   for audio that must not move.
2. During a tempo-point drag, re-anchor only on `EndTempoDrag` (recommended)
   so the drag stays cheap.
3. Audio-clip durations: keep not stretching them (as today).

## Slices

1. Route every `GlobalTrackMessage` mutation and `SetTimeSignature` /
   `CycleTimeSignature` through capture/re-anchor; flip the pinned test;
   undo stays one entry.
2. Add chord-track regions / key changes and take-group slots to
   `MusicalAnchors` (and to the bar-shift displacement).
3. `keep_time` param on `global.*` + MCP schema + skill text.

## How we'd know it works

After adding a 7/8 event at bar 17, a MIDI clip, automation point, marker and
chord region at bar 25 still report bar 25 through `song.*` /
`automation.lanes`; a frozen track goes stale; one undo restores everything.
