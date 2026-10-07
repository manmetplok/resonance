# Clip state read-back in `song.tracks`

> Status: **idea**
> Area: timeline (control-api surface)
> Source: review follow-up FU-W2c (`docs/history/reviews/code-review-2026-10-02.md`);
> gap found while documenting clip editing.
> Register items covered: FU-W2c (review 2026-10-02).

## Why

An agent cannot hear, so it checks every edit by reading it back. For audio
clips, `song.tracks` returns only id, name, start, length and a `midi` flag
(`ClipView` in `resonance-control/src/methods/song.rs`). After
`clip.trim`, `clip.set_fade`, `clip.set_gain` or `clip.set_warp` the only
evidence is the reply to that one call; a later session, or an edit the user
made by hand, is invisible. The agent cannot tell which part of a source a
clip plays, whether it has fades, or whether it is warped.

## What it would do

`song.tracks` (or a `song.clip` detail view) reports, for each audio clip:
`start_offset_samples`, `end_offset_samples`, source length, fade-in/out
length and shape, `gain_db`, the pool `asset_id` (or that it is a recorded
take), and warp (`enabled`, `original_bpm`, `transpose_semitones`,
`algorithm`, marker count or markers).

## What exists to build on

- All values are already on the app mirror `ClipState`
  (`resonance-app/src/state/clips.rs`, `state/clip_warp.rs`).
- The reply shapes exist: `TrimResult`, `FadeResult`, `WarpResult` in
  `resonance-control/src/methods/clip.rs`.
- The projection lives in `resonance-app/src/update/control/view_model/`.

## Open decisions

1. **Extend `ClipView` or add a detail method?** Recommended: add optional
   fields to `ClipView` for audio clips (skipped when default), so one read
   covers the song; markers only as a count there, full markers in a
   `song.clip` detail call if needed.
2. **Protocol version.** Adding optional fields is backward-compatible; bump
   `PROTOCOL_VERSION` only if the agent plugin must re-read it
   (`resonance-mcp/tests/agent_plugin_lockstep.rs`).

## Slices

1. Trim offsets, fades, gain and `asset_id` in `ClipView` + MCP output
   schema + tests in `resonance-app/tests/control/control_song_views.rs`.
2. Warp fields.

## How we'd know it works

A control test that edits a clip through the GUI messages and then reads the
same values back through `song.tracks`.
