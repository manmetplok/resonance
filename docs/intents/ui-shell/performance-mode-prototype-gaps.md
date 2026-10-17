# Performance mode: finish the teleprompter prototype

> Stage: **intent**
> Status: **partly built**
> Area: ui-shell
> Touches: Performance mode
> Source: `docs/intents/prototypes/performance-mode-teleprompter/index.html` compared with `resonance-app/src/view/performance/` at `8eac71b3`.

## Why

Performance mode is meant to be read from a few metres away while playing.
The built version shows the chord, the diagram, the beat ring and the next
chords. It leaves out several cues the prototype relied on for timing:

- how many beats are left on this chord;
- what comes next, and in how long;
- which chord the take will start on during a count-in.

There is also a stale placeholder column that wastes stage width.

## What it would do

- **Status bar during a count-in:** "Armed · counting in", with the clock
  turning mint.
- **Chord meta line** under the symbol: "Current · bar 12 · 2 beats left",
  and for slash chords the inversion ("first inversion · bass A").
- **Diagram caption:** "Open position · Guitar 6" / "Boxed · 5fr".
- **Beat ring:** the beat number in the centre ("3 / 6"), and an "F7 in 2
  beats" line under it (`beats_until_next` is already computed).
- **Count-in:**
  - the first chord of the take shown dimmed on the stage
    (`primed_position` passed to the stage and the lane);
  - the lane labelled "Starting";
  - a caption such as "Em ready · audio in 2 beats".
- **Tidy-up:**
  - remove `center_stage`'s empty 200 px `ring_placeholder`;
  - update the "placeholder" / "lands in todo #310" comments and the
    primed-chord claim in `beat_cue.rs`.
- **Optionally**, F switches the OS window to fullscreen, as its footer hint
  ("F fullscreen") suggests.

## What exists to build on

- `resonance-app/src/query/performance.rs`:
  - `ChordReadout` already carries `beats_until_next`, `bars_until_next`,
    `beat_in_bar`, `beats_per_bar` and `priming`;
  - `ChordQuery::primed_position`.
- `view/performance/mod.rs::performance_cue_state`: it already derives the
  count-in and the primed position for the ring.
- `view/performance/center_stage.rs::hero` and `beat_cue.rs::draw_cue`.
- The goldens in `resonance-app/tests/performance/performance_scaffold.rs`.
- `docs/features/ui-shell/performance-mode.md`.

## Open decisions

1. Should F mean OS fullscreen, or keep it as the in-app view? **Recommend
   keeping F as the view toggle** and changing its hint from "fullscreen" to
   "performance". OS fullscreen can be a separate command if wanted.
2. Should the beat-ring centre number use mono, as in the prototype, or
   serif? **Recommend mono**, matching the clock.
3. Lyrics are not in the prototype. **Recommend leaving them out** of this
   intent.

## Slices

1. Tidy-up: remove the placeholder column and fix the comments and the hint
   text. Re-bless the four scaffold goldens.
2. Count-in: primed chord on stage and in the lane, the "Starting" label, and
   the status-bar state. Add a golden of the count-in.
3. Timing cues: the chord meta line, the ring centre number, and the
   until-next line. Add a golden of a slash chord with its meta line.
4. Diagram caption.

## How we'd know it works

- New goldens for count-in and slash chord.
- `performance_beat_cue.rs` and `performance_chord_readout.rs` extended for
  the primed stage.
- A live check while recording with a count-in.
