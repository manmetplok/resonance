# Performance mode (chord teleprompter)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: prototype
> `docs/intents/prototypes/performance-mode-teleprompter/index.html`;
> todos #308, #310, #372.

Performance mode is a fourth view for playing along while recording. It
fills the window with the chord under the playhead:

- a huge chord symbol and a fingering diagram for the chosen instrument and
  capo;
- a beat ring with a count-in;
- the next three chords;
- the current and next section names.

F toggles it and Esc leaves it. It is a chord teleprompter only. It does not
show lyrics.

## Behaviour

**Entering and leaving**

- Use the **Performance** tab, F (`TogglePerformanceMode`) or `--tab
  performance`.
- Esc (`ExitPerformanceMode`) returns to the view it was entered from, or to
  Arrange.
- It never opens by itself on record-arm, and switching never touches the
  transport.
- F is typing-gated and ignores key repeat.
- The startup screen blocks both commands.
- While in the mode, the generic plugin window is hidden and preset commands
  have no target.

**Layout** (`view/performance/mod.rs::view_performance_shell`). It replaces
the chrome and transport bar with four bands on `BG_0`, separated by
hairlines.

1. **Status bar** (56 px, `BG_1`). It shows, left to right:
   - a lavender "PERFORMANCE" pill and the project name;
   - RECORDING (`BAD` dot), REHEARSAL (playing) or STOPPED;
   - under that, the current section → next section, read from the
     **arrangement markers** (`section_readout`), in the marker colour;
   - bar.beat (mono, `WARM`), BPM, time signature and key;
   - an "Exit ⎋" button.
2. **Centre stage**, which fills the rest of the window:
   - The chord comes from `chord_readout` over the Compose section
     placements and their chords.
   - `center_stage::hero` draws the root in serif italic (184 px), the
     quality muted (86 px) and the slash bass dimmer.
   - Chord-tone chips follow, with the root chip lavender.
   - A `FingeringDiagram` Canvas from the shared `chord_box` layout shows
     the nut or "Nfr", O/X shapes, note names in the dots, a lavender root
     and the string names.
   - With no chord: a 200 px "—", "No chord under the playhead" and a hint
     to place sections in Compose.
   - While the transport rolls or counts in, a 260 px **beat cue** column is
     added (`beat_cue.rs`). It has ring pips (the current beat bright, past
     beats dim) and a `WARM` arc for what is left of the current chord.
   - During a record count-in (playing, recording, and recording start
     after the playhead), the ring shows a big mint count-down number and
     "COUNT-IN".
3. **Up next** (180 px, `BG_1`):
   - Up to 3 cards (`UPCOMING_COUNT`) separated by "›".
   - Each card has a chord symbol, a mini chord box (Canvas) and "this bar"
     / "in N bars".
   - The first card is emphasised and later ones dimmer.
   - The list wraps at the loop end when looping.
4. **Footer** (44 px):
   - INSTRUMENT segmented cells: Guitar 6, Guitar 8, Bass 4, Bass 5.
   - A CAPO stepper from 0 to 12.
   - Key hints from the registry: "Space play · R record · F fullscreen ·
     Esc exit".

**Instrument and capo** are saved in the project as `ProjectPerformance {
tuning, capo }`. An unknown tuning name loads as Guitar 6. Neither is
undoable state or an app setting.

**Frame cost.**

- The status bar rebuilds every frame.
- The stage, the up-next lane and the footer are `lazy`, keyed on
  fingerprints.
- The diagrams and the beat ring are Canvas programs with `canvas::Cache`,
  cleared only when their fingerprint changes.
- `chord_readout` itself runs three times per `view()`: stage, cue and lane.
- Root overlays (palette, dialogs) are drawn over this shell as over any
  other view.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Readouts (pure) | `resonance-app/src/query/performance.rs` | `chord_readout`, `ChordQuery`, `ChordReadout`, `ChordSlot`, `section_readout`, `SectionReadout`, `UPCOMING_COUNT` |
| State | `resonance-app/src/state/performance.rs` | `PerformanceState`, `MAX_CAPO`, `voicing` |
| Shell | `resonance-app/src/view/performance/mod.rs` | `view_performance_shell`, `performance_status_bar`, `performance_center_stage`, `performance_cue_state`, `performance_next_cards`, `performance_footer`, `key_hint_line` |
| Hero, diagram | `resonance-app/src/view/performance/center_stage.rs` | `hero`, `FingeringDiagram`, `hero_fingerprint` |
| Beat ring | `resonance-app/src/view/performance/beat_cue.rs` | `BeatCueState::derive`, `beat_cue`, `CUE_SIZE` |
| Up next | `resonance-app/src/view/performance/next_lane.rs` | `NextCard`, `Emphasis`, `bars_until_label`, `fingerprint` |
| Messages | `resonance-app/src/update/ui.rs` | `TogglePerformanceMode`, `ExitPerformanceMode`, `SetPerformanceTuning`, `SetPerformanceCapo` |
| Persistence | `resonance-app/src/project/model.rs`, `update/project_io/serialize.rs`, `replay/restore.rs` | `ProjectPerformance`, `restore_performance` |

## Surfaces

- **GUI:** the Performance tab; F / Esc; the footer cells.
- **Control API / MCP:** none. There is no method to switch views or to set
  the tuning or capo.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Chord readout (current, upcoming, gaps, loop wrap, primed position, tempo change) | `resonance-app/tests/performance/performance_chord_readout.rs` | `./scripts/run-tests.py -p resonance-app` |
| Section readout | `resonance-app/tests/performance/performance_section_readout.rs` | same |
| Beat cue derivation | `resonance-app/tests/performance/performance_beat_cue.rs` | same |
| Voicings, capo, fingerprints | `resonance-app/tests/performance/performance_center_stage.rs`, `performance_state.rs`, `performance_next_lane.rs` | same |
| Toggle / Esc / return view / transport untouched / typing gate | `resonance-app/tests/performance/performance_mode_toggle.rs` | same |
| Persistence | `resonance-app/tests/performance/performance_persistence.rs` | same |
| Render smoke | `resonance-app/tests/performance/performance_center_stage_render.rs`, `performance_next_lane_render.rs` | same |
| Goldens (1440×900) | `performance_scaffold_{empty_stopped,demo_stopped,rehearsal,recording}-wgpu.png` (`performance/performance_scaffold.rs`) | `RESONANCE_BLESS=1 cargo test -p resonance-app --test performance` |

Not covered: goldens of the count-in, the beat ring, a slash chord, a
tuning or capo change, and the section label.

## Decisions and deviations

Compared with the prototype:

- **Built:**
  - status bar, hero symbol, chord-tone chips, fingering diagram;
  - beat ring and count-in number;
  - up-next lane and footer, including working Instrument and Capo
    controls;
  - empty states (the copy matches).
- **Not built:**
  - the "Armed · counting in" status, and the clock turning mint during a
    count-in;
  - the chord meta line ("bar 12 · 2 beats left", "first inversion · bass
    A") and the diagram caption ("Open position · Guitar 6");
  - the number inside the ring and the "F7 in 2 beats" line;
  - the dimmed primed chord on stage during a count-in (the stage passes
    `primed_position: None`), and the "Starting" lane label;
  - OS fullscreen for F.
- **Deliberately dropped:** the gradient backgrounds, which are flat by
  design.
- **Added beyond the prototype:** the section names in the status bar
  (todo #372).
- The section names come from arrangement markers, while the chords come
  from Compose section placements. A project with sections but no markers
  shows chords and no section label.

## Known gaps

- The prototype items listed under "Not built" above
  ([intent](../../intents/ui-shell/performance-mode-prototype-gaps.md)).
- Stale code: `center_stage::hero` still adds a 200 px empty
  `ring_placeholder` column from before the beat cue existed. While the
  transport rolls, the stage therefore has an empty column and the cue
  column. Several "placeholder" comments in `view/performance/mod.rs` and
  the primed-chord claim in `beat_cue.rs` are out of date (same intent).
