# macOS port: open checks and embedded editors

> Status: **partly built**
> Area: plugins-instruments (editor runtimes), with audio-engine for host
> threading
> Source: `docs/history/macos/macos-editor-plan.md` (item 3g, §7),
> `docs/history/macos/macos-linux-verification.md` §6,
> `docs/history/reviews/code-review-todo.md` (FU-M1a, FU-H4a).
> Register items covered: FU-H4a (review 2026-10-02), macOS plan item 3g, refactor-intent §4 macOS.

## Why

The macOS stack is merged (`osx/step-1` … `osx/step-3h`, PLG-01..05, per-
image Objective-C classes) and on 2026-09-29 all 13 editors opened, closed
and reopened in the app on a Mac, with `editor_open_cocoa`, `editor_size`
and `modal_reentrancy` passing (commit `483464e2`). What is left:

- **Re-verification.** Cocoa-side changes since then (shared param-bound
  knob and edit announcing, 2026-10-03; workspace dependency moves,
  2026-10-04) were only type-checked on Linux. The three by-hand Cocoa
  tests have to be re-run from a logged-in macOS session after any editor
  change.
- **Embedded editors and third-party macOS editors (plan item 3g).** The
  bridge's `set_parent` is a stub; our editors are floating only. Hosting a
  third-party plugin's Cocoa editor needs every CLAP GUI call on the real
  AppKit main thread, which the host does not do.
- **macOS baselines.** No golden-image set for macOS and no per-platform
  hashes for the aarch64-sensitive parity/DSP tests; the canonical Linux
  box is the only golden source.
- **Test plumbing (FU-H4a).** `resonance-gate` has a macOS-only
  dev-dependency on `cocoa-plugin-gui` for the NSApplication pump; the pump
  should come through `editor_host` instead.

## What it would do

Editors embed in a host window when the host asks; a third-party Cocoa
editor opens in Resonance; the by-hand tests are part of a macOS checklist
run after every editor change; macOS goldens exist or macOS is explicitly
excluded from golden runs.

## What exists to build on

- `cocoa-plugin-gui` (main-thread controller, per-image classes, async
  destroy), `resonance-plugin/src/editor_host.rs`,
  `resonance-plugin/src/clap_bridge/gui.rs`,
  `resonance-audio/src/clap_host/gui.rs` (Cocoa API negotiated, floating).
- `docs/features/plugins-instruments/plugin-editor-runtimes.md`.

## Open decisions

1. Embedded mode on both platforms or macOS only. Recommended: macOS first
   (Wayland has no cross-client embedding).
2. Golden images on macOS. Recommended: keep Linux canonical and skip image
   goldens on macOS, but add per-platform hashes for the numeric parity
   tests that differ on aarch64.

## Slices

1. Re-run the three Cocoa tests and an all-editors open/close pass; record
   the result.
2. Move the NSApplication pump behind `editor_host` (FU-H4a).
3. Per-platform parity hashes.
4. Host-side main-thread marshalling of GUI calls on macOS, then
   `set_parent` for our editors, then third-party Cocoa editors.

## How we'd know it works

The three `harness = false` Cocoa tests pass from a Mac session; a
third-party CLAP plugin's editor opens and closes twice from the app on
macOS; the suite is green on both platforms.
