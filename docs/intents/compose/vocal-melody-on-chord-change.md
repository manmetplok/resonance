# Vocal melody on a chord change: ask, don't overwrite

> Status: **designed**
> Area: compose (vocals) + control-api
> Source: [BUG-70](../bugs.md); design Q&A with Jorrit 2026-10-13
> ("show a popup asking the user to choose between the options, also when
> generated").

## Why

A chord or section-length edit re-derives every vocal lane's melody in that
section (`update/compose/regenerate.rs` `propagate_chord_change` →
`update/compose/vocal_render.rs` `rederive_vocal_melody`). A melody written
by hand, imported from MIDI, or generated and then tuned is replaced
without a word. User `_` holds survive in the lyrics; the notes do not.

## What it would do

**GUI.** When a chord or section-length edit would change the melody of one
or more vocal lanes, a modal asks what to do. It lists every affected lane
(section, track, and whether its notes were hand-edited since the last
generate). Each lane gets one of:

| Choice | Effect |
|---|---|
| **Keep melody** (default for hand-edited lanes) | Notes untouched. The lane is flagged `melody_predates_chords` until it is regenerated, re-fitted or its notes are edited again. |
| **Re-fit pitches** | Rhythm, lyrics, holds and note count stay. Each pitch moves to the nearest note that fits the new chord at that position (chord tones on strong beats and long notes, scale tones elsewhere), keeping the contour. |
| **Regenerate** (default for untouched generated lanes) | Today's behaviour: re-derive the melody from the generator. |

- "Apply to all lanes" sets every row at once.
- "Remember for this project" stores the choice per project. The modal is
  then skipped and the remembered choice applies. Settings › Compose has
  "Ask again about vocal melodies" to clear it.
- Esc / Cancel undoes the chord edit itself.
- The chord edit and the chosen vocal outcome are **one undo entry**.
- The lane inspector and the vocal-roll header show a "Melody predates
  chord change" badge with Re-fit and Regenerate buttons while the flag is
  set.

**Control / MCP** (there is no modal):
- Every call that can change chords or a section length (`harmony.*`,
  `section.resize`, `arrangement.*` where they re-derive) accepts an
  optional `vocal_melody: "keep" | "refit" | "regenerate"`.
- Without it, the lane keeps its melody and is flagged. The result lists
  `vocal_lanes_affected: [{definition_id, track_id, outcome}]` so the agent
  knows what happened.
- A project's remembered choice (GUI) also applies to control calls
  without the field.
- `song.vocal` lanes report `melody_predates_chords: bool` and
  `hand_edited: bool`.
- New method `vocal.refit_melody {track_id, section_id?}` applies Re-fit
  on demand; `vocal.generate` already covers Regenerate.

**Hand-edited tracking.** A lane is `hand_edited` once its notes change by
anything other than the generator: a GUI note edit, `notes.*`, or a MIDI
import. It resets when the lane is regenerated. It is persisted with the
lane and rides undo.

## What exists to build on

- `propagate_chord_change` and `regenerate_lane_as_side_effect`
  (`update/compose/regenerate.rs`).
- `rederive_vocal_melody` (`update/compose/vocal_render.rs`), which keeps
  generator holds and user `_` holds through `derive_vocal_with_holds`.
- The render-state settle and debounce (`update/compose/vocal_render_state.rs`):
  a kept, re-fitted or regenerated lane goes stale and follows the usual
  GUI-debounce or MCP-explicit rule.
- Chord-tone and scale helpers in `resonance-music-theory` for Re-fit.
- Existing modals in the app (export, voicebank library) as the pattern.

## Decisions

1. Defaults in the modal: Keep for hand-edited lanes, Regenerate for
   untouched generated lanes (pre-selected; the user can change any row).
2. MCP default without `vocal_melody`: keep and flag (never lose notes
   silently), unless the project remembers a choice.
3. Re-fit never changes rhythm, note count, lyrics or holds; when two
   candidates are equally near it prefers the one that keeps the previous
   interval direction.

## Slices

1. `hand_edited` and `melody_predates_chords` flags (model, persistence,
   undo, `song.vocal`); chord edits keep and flag by default over control;
   `vocal_melody` param and `vocal_lanes_affected` result. Deletes BUG-70.
2. Re-fit (music-theory function + `vocal.refit_melody` + MCP tool).
3. GUI modal with per-lane choices, remember-per-project, Settings reset,
   one undo entry; lane badge with Re-fit / Regenerate. Goldens.

## How we'd know it works

- A hand-written melody survives a control chord edit; the lane is
  flagged; `refit` keeps rhythm, lyrics and note count while each pitch
  lands on a chord or scale tone of the new chord.
- `regenerate` matches today's output.
- The GUI modal flow is one undo entry, and Cancel restores the old chords.
- Remember-per-project skips the modal.
- Goldens for the modal and the badge.
