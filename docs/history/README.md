# History

Archived design specs, review registers, refactor plans and prototypes.
They record why things were built the way they were; they are **not** the
current truth. What exists is described in [`../features/`](../features/);
what is wanted but not built is in [`../intents/`](../intents/), with known
defects in [`../intents/bugs.md`](../intents/bugs.md).

**Rule: history is never updated, except the archive banner** under each
file's title, which points at the feature docs and intents that replaced
it. Open items found here are moved into `docs/intents/`, never ticked off
in place.

## specs/ — feature specs, as written before and during the build

- [automation-control-api.md](specs/automation-control-api.md) — `automation.*` control methods and MCP tools (built 2026-09-28).
- [command-palette.md](specs/command-palette.md) — command palette, keyboard shortcuts and transport commands (built 2026-09-30).
- [deharsh-resonance-suppressor.md](specs/deharsh-resonance-suppressor.md) — W12 design of the de-harsh resonance suppressor.
- [drums-plugin-rework.md](specs/drums-plugin-rework.md) — drums plugin rework: kit library, plok.org downloads, editor, sampler (K0–K9).
- [melody-generation-research.md](specs/melody-generation-research.md) — research notes on improving melody and harmony generation (2026-06-10).
- [mixer-cleanup.md](specs/mixer-cleanup.md) — Mix tab cleanup: slim strips, one editing surface in the inspector.
- [nam-model-library.md](specs/nam-model-library.md) — NAM amp-model library and the shared library foundation.
- [plugin-preset-library.md](specs/plugin-preset-library.md) — one preset browser, tags, favourites and genre for every plugin.
- [realtime-multithreading.md](specs/realtime-multithreading.md) — parallel graph rendering on a realtime worker pool (P0–P4).
- [reverb-algorithms.md](specs/reverb-algorithms.md) — the nine reverb algorithms and the decay meter (R0–R9).
- [todo.md](specs/todo.md) — the melody/harmony improvement todo derived from the research notes.
- [warmth-width-depth.md](specs/warmth-width-depth.md) — mix-character tools for warmth, width and depth (W0–W12).

## research/ — research reports and their source notes

- [singing-voicebank-options-2026.md](research/singing-voicebank-options-2026.md) — are TIGER, Lilia and Meiji still the right singing voicebanks? Open SVS models, licensing, integration (2026-10-07); notes in `research/singing-voicebank-options-2026-notes/`.

## reviews/ — review registers and field reports

- [code-review-todo.md](reviews/code-review-todo.md) — whole-workspace code review of 2026-09-26 (145 findings) and its fix campaign.
- [code-review-2026-10-02.md](reviews/code-review-2026-10-02.md) — whole-workspace code review of 2026-10-02 (121 findings) and its fix campaign.
- [plugin-audit.md](reviews/plugin-audit.md) — plugin capability-vs-exposure audit (2026-08-15).
- [plugin-audit-plan.md](reviews/plugin-audit-plan.md) — the resolution plan (waves) for the plugin audit.
- [field-report-2026-10-06.md](reviews/field-report-2026-10-06.md) — field report: Arp xrun, `set_send`, depth estimate, Room decay.

## refactor/ — architecture epics and their designs

- [refactor-intent.md](refactor/refactor-intent.md) — architecture epics A–E and their status.
- [arch-migration-plan.md](refactor/arch-migration-plan.md) — incremental plan for ARCH-01…ARCH-10 from the 2026-09-26 review.
- [A-6-derived-clips.md](refactor/A-6-derived-clips.md) — moving the derived-clip map out of `UndoExtras`.
- [A-12-resonance-fields.md](refactor/A-12-resonance-fields.md) — survey of the remaining `Resonance` fields.
- [A-13-reconcile.md](refactor/A-13-reconcile.md) — the `Reconcile` trait: one domain order for load and undo.
- [D-6-engine-created-ids.md](refactor/D-6-engine-created-ids.md) — ids the engine creates at times the app can't pre-decide.
- [arch-test-grouping.md](refactor/arch-test-grouping.md) — the file lists of the grouped `resonance-audio` test binaries (ARCH-03).

## macos/ — the macOS port

- [macos-editor-plan.md](macos/macos-editor-plan.md) — scoping plan for the Cocoa plugin-editor runtime (port step 3).
- [macos-linux-verification.md](macos/macos-linux-verification.md) — Linux verification checklist for the stacked macOS branches.

## prototypes/ — HTML design prototypes whose feature is built

See [prototypes/README.md](prototypes/README.md) for each prototype and the
feature doc that now describes it. Prototypes with work left live in
[`../intents/prototypes/`](../intents/prototypes/README.md).
