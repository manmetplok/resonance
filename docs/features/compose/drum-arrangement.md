# Drum patterns and section drum arrangement

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: prototype `docs/history/prototypes/drum-pattern-arrangement-within-section/index.html` (doc #170, epic #38); `docs/history/specs/drums-plugin-rework.md` §8 (kit picker).

Drums in Compose come from a project-wide **pattern bank**. A pattern is a
set of step-sequenced **drum groups** (kick, snare, hats …), each with its
own grid, cycle length and phase, so polymeters are possible. Each section
has an ordered **drum arrangement**: a list of entries that say which
pattern plays for how long, with an optional one-bar fill on the last bar.
The arrangement is rendered to derived MIDI clips on the drum tracks.

## Behaviour

- **Bank.** A new project starts with "Main" (default groups) and an empty
  "B section". Patterns can be added, duplicated, renamed, recoloured and
  deleted; deleting a pattern removes every arrangement entry (and fill)
  that names it. Patterns have an intrinsic `length_bars` (default 1).
- **Groups.** A `DrumGroup` has `grid` (steps per beat: 2, 3, 4, 5, 6, 7),
  `cycle` (steps before it repeats — a 7-step cycle on a 4-grid runs
  against the bar), `phase` (rotation), pads (`DrumGroupPad`: MIDI note,
  weight, one cell per step) and knobs `density`, `swing`, `accent`,
  `humanize`, `fills`, a free-text `style` and a `seed`. Groups belong to
  their pattern; duplicating a pattern forks its groups.
- **Generate (Euclidean).** `generate_group_pattern` spreads
  `round(cycle × density)` hits (at least 1) over the pads in proportion to
  their weight, evenly spaced and rotated by a seeded xorshift draw. It
  overwrites every step. Clicking a cell in the drum grid toggles it.
- **Arrangement.** `PatternEntry { pattern_id, length, fill }` with
  `EntryLength::RepeatN(n)` (n × the pattern's bars) or `Bars(n)` (tile the
  pattern to n bars). `resolve_arrangement` turns the entries into
  section-relative spans and an `ArrangementCoverage` (exact, gap, or
  overflow). Gap bars are silent; overflow is clipped at the section end.
  "Fill to end" and "Trim to fit" fix coverage. An **empty** arrangement
  falls back to the project default pattern for the whole section.
- **Rendering.** `materialize_drum_clips_for` writes one derived clip per
  placement on **every** drum track (instrument tracks with
  `InstrumentType::Drum`, no sub-tracks) — all drum tracks get the same
  notes. Step `n` starts at `n × TPQ / grid` ticks; the cycle index runs on
  the absolute section step so phase stays continuous across spans.
  Velocity is `0.70 + min(weight/200, 0.25)`, plus `accent × 0.25` on
  beat-start steps.
- **Knobs that do nothing yet.** `swing`, `humanize` and `fills` are stored
  and have sliders, but neither generation nor rendering reads them. The
  per-step cell value (a 0..=100 velocity scale in built-in patterns) is
  treated as on/off.
- **Built-in grooves.** `library.rs` holds ten authored templates:
  `silence`, `halftime`, `four-on-floor`, `industrial`, `breakbeat`,
  `blast`, `sparse`, `toms`, `build`, `fill` (16th grid, 4/4). They are not
  in the bank until `generate.drums` instantiates one for a section;
  `apply_density` thins authored steps by metrical weight (16ths first,
  then off-8ths), every voice keeping its first hit. `silence` pins a
  section drumless.
- **Kit pads.** The Drum Groups Manager lists pads from the drum track's
  Resonance Drums kit report (`refresh_kit_pads`), falling back to the
  General MIDI table (`default_kit_pads`).
- **Motif-mode drum voices.** Legacy per-pad `DrumVoiceMode::Motif` voices
  on old projects still follow the section motif
  (`regenerate_drum_motif_voices`); nothing creates them now.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Section arrangement model | `resonance-app/src/compose/section.rs` | `PatternEntry`, `EntryLength`, `set_primary_pattern`, `fill_to_end`, `trim_to_fit` |
| Resolver | `resonance-app/src/compose/arrangement.rs` | `resolve_arrangement`, `ArrangementSpan`, `ArrangementCoverage` |
| Bank, groups, pads | `resonance-app/src/compose/drumroll/{pattern,groups,drum_map}.rs` | `DrumPattern`, `DrumGroup`, `DrumGroupPad`, `KitPadInfo`, `DrumPadMap` |
| Built-in grooves | `resonance-app/src/compose/drumroll/library.rs` | `TEMPLATES`, `instantiate_builtin`, `apply_density` |
| Handlers + render | `resonance-app/src/update/compose/drum_groups.rs` | `handle`, `handle_arrangement`, `generate_group_pattern`, `materialize_drum_clips_for`, `build_drum_notes`, `control_generate_drums`, `refresh_kit_pads` |
| Views | `resonance-app/src/view/compose/drumroll/{canvas,ribbon,pattern_picker}.rs`, `lane_inspector/drums/{arrangement,meter,rhythm}.rs`, `drum_groups_manager/` | drum grid, tiling ribbon, ARRANGEMENT strip, pattern bank card, entry inspector, manager modal + kit picker |
| Persistence | `resonance-app/src/project/sections.rs`, `project/model.rs` | `ProjectPatternEntry`, `ProjectFile::drum_patterns` (legacy `drum_groups` / `drum_pattern_id` migrated on load) |

Invariant: arrangement edits only re-materialize when a mutator returns
`true`; a regenerate after load must replace, not duplicate, drum clips
(drum clips are claimed by `rebuild_derived_clips` only at id ≥
`CLIP_ID_BASE`).

## Surfaces

- **GUI:** Compose tab → drum lane: drum grid (click cells), ARRANGEMENT
  strip and tiling ribbon; right rail: pattern bank card (click to add an
  entry), entry inspector (pattern, Repeat ×N / Fixed bars, fill toggle,
  duplicate, remove), per-group meter and rhythm knobs, Generate; Drum
  Groups Manager modal (groups, pad assignment, kit picker).
- **Control API / MCP:** `generate.drums` → `mcp__resonance__generate_drums`
  with `section_id`, `track_id` (must be a drum track), optional `pattern`
  (bank name first, then built-in name), `density` (0..1, default 1),
  `seed`. Only that section is rebuilt. A shared bank pattern is cloned for
  the section before re-rolling, so other sections keep their drums. A
  built-in is installed as authored (not re-rolled). Returns `clip_id` +
  `clip_ids`. The arrangement itself (multiple entries, fills) has no
  control method. `drum_kits_list` belongs to the drums plugin
  ([drum-kit-library.md](../plugins-instruments/drum-kit-library.md)).
- **Agent skills:** `drumming`, `arranging`, `song-structure`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Resolver, edits, inspector, persistence | `resonance-app/tests/compose/compose_drum_arrangement{,_edit,_inspector,_persistence,_ribbon}.rs` | `./scripts/run-tests.py -p resonance-app` |
| Materialization | `tests/compose/compose_drum_materialize_arrangement.rs`, `compose_drum_grid_bar_spans.rs`, `drum_grid_tuplet_drift.rs`, `drum_section_coverage.rs` | same |
| Built-in library + density | `tests/compose/drum_pattern_library.rs` | same |
| Kit pads / picker / cell click | `tests/compose/drum_kit_pads.rs`, `drum_kit_picker.rs`, `drum_cell_click_phase.rs` | same |
| Control | `tests/control/control_generate.rs`, `control_generate_after_load.rs` | same |
| Goldens | `tests/snapshots/arrangement_strip_*`, `compose_ribbon_*`, `drum_grid_chained_arrangement_tints_separator`, `compose_drum_rail_*` (all `-wgpu.png`) | `RESONANCE_BLESS=1 ./scripts/run-tests.py -p resonance-app` |

Not covered: swing/humanize/fills (they have no effect to test);
multiple drum tracks receiving identical material.

## Decisions and deviations

- Prototype vs build: the strip, tiling ribbon, gap/overflow states, fix
  actions, pattern bank and entry inspector are built. Dragging a bank
  pattern onto the strip is replaced by click-to-add (Iced has no
  cross-widget drag payload).
- `set_primary_pattern` writes a `Bars(length_bars)` entry, not
  `RepeatN(1)`, so assigning a one-bar pattern fills the section (ba doc
  #272 V-2a).
- `generate.drums` is section-scoped and clones shared patterns because
  re-rolling a shared pattern used to rewrite other sections.
- `generate_group_pattern`'s doc comment says swing/accent/humanize/fills
  "apply at MIDI rendering time inside `regenerate.rs`"; only accent does,
  in `emit_span_notes`.
- The `DrumGroup` doc comment says grouping is project-scoped; groups are
  owned by a pattern.

## Known gaps

- Swing, humanize, fills and per-step velocity are not applied —
  [drum-groove-feel](../../intents/compose/drum-groove-feel.md).
- Every drum track gets the same material; no per-track drum parts —
  [drum-material-per-track](../../intents/compose/drum-material-per-track.md).
- No control method to edit a section's arrangement entries or fills.
