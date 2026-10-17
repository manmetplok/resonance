# App code: refactor and review leftovers

> Stage: **intent**
> Status: **idea** (checklist)
> Area: project-core
> Source: [refactor intent](../../history/refactor/refactor-intent.md) / [arch migration plan](../../history/refactor/arch-migration-plan.md) (ARCH-03 A3-3, ARCH-06, A-6 §8, A-12a, A-13 §15, FU-A4a, FU-UPD05a);
> [code review 2026-10-02](../../history/reviews/code-review-2026-10-02.md) STATE2-10, FU-P1d, ARCH2-11, DEP-02

## Why

The architecture epics met their done-when criteria, but each left small
debts, and two possible data bugs were noted without being verified.
Confirmed app defects are in [`../bugs.md`](../bugs.md); the id work is
in [`finish-app-owned-ids.md`](finish-app-owned-ids.md).

## What it would do

To verify (possible bugs, not reproduced):

- [ ] **A-6 §8** — undo/rebuild of the derived vocal/MIDI clip map is
      positional; a derived clip the user moved off its placement bar may
      lose its map entry. Write the test; fix or close.
- [ ] **FU-A4a residual** — on the diff restore path, a track that is
      live-Stale only because its freeze cache failed to load may be
      treated as attached. Write the test; fix or close.

Clean-ups:

- [ ] **ARCH-06** — `resonance-app/src/message.rs` has grown back to 776
      lines (target < 300) and still declares `PresetUiMessage`,
      `PluginMessage`, `ChainUiMessage`, `PluginWindowDrag`, `UiMessage`;
      move them beside their handlers.
- [ ] **ARCH-03 A3-3** — group the plugin and library test binaries
      (mastering 35, music-theory 42, amp 29, granular-delay 24 top-level
      test files) the way the app and audio crates were grouped.
- [ ] **A-12a** — stuttering field names in sub-states
      (`plugin_catalog.available_plugins`, `plugin_scan_failures`,
      `plugin_scan_in_progress`; `state/plugin_catalog.rs:16-27`).
- [ ] **A-13 §15** — fold `detach_and_delete_cache_in` into
      `detach_and_delete_cache` (`update/freeze.rs:307-314,584`).
- [ ] **FU-UPD05a** — the comment at `update/compose/mod.rs:117-119` says
      a track rename re-derives the generated clip; it does not
      (`update/track.rs:542-544`). Fix the comment.
- [ ] **STATE2-10** — `recent.rs` keeps an inline `#[cfg(test)]` module
      justified by a stale "binary crate with no lib.rs" comment
      (`recent.rs:127-131`); move it to `tests/` (no inline tests).
- [ ] **FU-P1d** — template capture uses the cached plugin blob and
      mirror with no engine round-trip (`update/project_io/mod.rs:848-863`);
      collect fresh state like Save does.
- [ ] **ARCH2-11 remainder** — `lib.rs:83` uses the wire type
      `PresetMetaInput`; `engine_events/pool.rs:117` matches on a wire
      method name. Keep wire types at the control boundary.
- [ ] **DEP-02 remainder** — ort's `download-binaries` fetch is unhashed
      and pinned `=2.0.0-rc.14`; only the `ORT_LIB_LOCATION` override is
      documented.

## What exists to build on

[`../../features/project-core/app-state.md`](../../features/project-core/app-state.md),
[`../../features/project-core/reconcile.md`](../../features/project-core/reconcile.md),
[`../../features/project-core/architecture-invariants.md`](../../features/project-core/architecture-invariants.md).

## Open decisions

None (see [`open-decisions.md`](open-decisions.md) for the ones that need Jorrit).

## Slices

The two verifications first (they may turn into bugs.md rows); the
clean-ups one commit each. A ratchet in `arch-invariants` for
`message.rs` size would keep ARCH-06 from regressing.

## How we'd know it works

Tests for the two verifications; `cargo test -p arch-invariants` for the
structural items.
