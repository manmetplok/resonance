# Open product decisions from the reviews

> Stage: **intent** (blocked)
> Status: **blocked** (each item waits on a decision by Jorrit)
> Area: project-core
> Touches: cross-area register
> Source: [code review 2026-09-26](../../history/reviews/code-review-todo.md) FU-B1, LIB-06;
> [code review 2026-10-02](../../history/reviews/code-review-2026-10-02.md) DEP-04 / FU-X1b, FU-S1a, FU-D1b6, FU-A1b1, FU-A1b4, FU-W1e;
> [refactor intent](../../history/refactor/refactor-intent.md) §4, A-13j

## Why

The review campaigns stopped at these because the right behaviour is a
product call, not a code fact. Each has a recommended option so it can be
answered with "yes" and built.

## What it would do

Answer each; then the work moves to the named area.

| # | Id | Area | Question | Recommended |
|---|---|---|---|---|
| 1 | FU-B1 | compose | Resizing a section re-generates its chord and vocal lanes from the seed, losing hand edits to generated notes (and re-rendering vocals). `update/compose/section.rs:434,519-522`, `update/compose/regenerate.rs:133-168`. Keep, warn, or preserve? | Preserve edits inside the old length and generate only the new bars; warn when shrinking would drop edited notes. |
| 2 | LIB-06 | plugins-effects | The compressor release fix (`cea54427`) changed the sound. Keep it or revert? | Keep (it fixed a real bug); confirm by ear once. |
| 3 | DEP-04 / FU-X1b | project-core | The licence is decided (MIT OR Apache-2.0), but LAME (LGPL) is statically linked through the default `mp3` feature (`resonance-audio/Cargo.toml:39,52-53`) and no generated third-party notices file exists (`THIRD_PARTY_LICENSES.md` "What's NOT done"). | Make `mp3` dynamically linked or opt-in for distributed builds; generate notices with `cargo about` in `bundle.sh`. |
| 4 | FU-S1a | control-api | Control calls that only read the open project are exempted from undo by a list (`reads_open_project_only`). Replace it with a split-gesture fix in `record_undo`? The comment at `undo/mod.rs:121` is stale either way. | Keep the busy answer; fix the stale comment; revisit only if the list grows. |
| 5 | FU-D1b6 | plugins-instruments | The amp sums stereo input to `0.5*(l+r)` (`plugins/resonance-amp/src/dsp/processor.rs:286-295`). For a one-sided DI on a stereo track that is −6 dB. Louder channel, L only, or a selector? | An input selector (Sum / L / R) defaulting to Sum. |
| 6 | FU-A1b1 | ui-shell | Should a running bounce or freeze block Quit and the autosave toggles? | Quit asks for confirmation and cancels the job; toggles stay live. |
| 7 | FU-A1b4 | mixer | Multi-out effects on a bus or master: the `PluginAdded` output-port info is ignored. Support routing their extra ports, or declare main-out only? | Main-out only for busses and master; document it. |
| 8 | FU-W1e | hardware-io | MIDI controller maps address targets by id, so a map does not carry to another project. Address by track name/role instead? | Keep ids inside a project; add name-based matching only for exported controller maps. |
| 9 | A-13j | project-core | Undoing the removal of a missing plugin re-raises the missing-plugins warning unless the user dismissed it (`reconcile/restored.rs:193-194`) — "a default, not a decision". | Do not re-raise a warning the user already saw this session. |

## What exists to build on

The cited code locations; each source register has the original analysis.

## Open decisions

The table above.

## Slices

None until decided. Each decided row becomes an intent or a bugs.md row in
its area, and is removed from this table.

## How we'd know it works

This table is empty.
