# Cocoa editor runtime: test and teardown leftovers

> Status: **idea** (checklist; needs a Mac)
> Area: plugins-instruments (editor runtimes)
> Source: [code review 2026-09-26](../../history/reviews/code-review-todo.md) FU-M8a, FU-M8c;
> [code review 2026-10-02](../../history/reviews/code-review-2026-10-02.md) FU-A1a4

## Why

Items in `cocoa-plugin-gui` that are not in
[`macos-port-open-checks.md`](macos-port-open-checks.md) (which covers the
embedded editors, macOS goldens, FU-H4a and re-verification). Two are
latent hangs or tests that cannot fail.

## What it would do

- [ ] **FU-M8a** — since `Editor::destroy` became asynchronous, the
      teardown watchdogs in `cocoa-plugin-gui/tests/editor_size.rs:118-128`
      and `plugins/resonance-gate/tests/editor_open_cocoa.rs:129-139` pass
      trivially. Make them wait on `Editor::liveness()` like
      `modal_reentrancy` does.
- [ ] **FU-M8c** — `Editor::new` waits with no timeout on a synchronous
      main-thread dispatch (`cocoa-plugin-gui/src/editor.rs:31-37,79-110`,
      `window_main_thread/mod.rs:138-140`); add the PLG-10 bound the
      Wayland runtime has. `hide()` only calls `orderOut` and does not
      release held mouse buttons; release them as on Wayland.
- [ ] **FU-A1a4** — build on a Mac and clear any macOS-only warnings.
- [ ] Update the comment at `resonance-plugin/src/clap_bridge/gui.rs:130`,
      which cites "macos-editor-plan.md item 3g" (now archived), to point
      at [`macos-port-open-checks.md`](macos-port-open-checks.md).

## What exists to build on

[`../../features/plugins-instruments/plugin-editor-runtimes.md`](../../features/plugins-instruments/plugin-editor-runtimes.md);
the Wayland runtime's bounded `Editor::new` and held-button release (PLG-10).

## Open decisions

None.

## Slices

One Mac session: watchdog fix first (so the other changes are guarded),
then the bound and the release.

## How we'd know it works

The three `--ignored` Cocoa tests in CLAUDE.md pass, and a deliberately
wedged main thread makes `editor_size` fail instead of pass.
