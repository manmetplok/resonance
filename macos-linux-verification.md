# macOS port — Linux verification checklist

**Date:** 2026-08-21 · **Scope:** the stacked branches `osx/step-1` → `osx/step-2` →
`osx/step-3a` → `osx/step-3b` → `osx/step-3c` → `osx/step-3d` → `osx/step-3e` →
`osx/step-3f` → `osx/step-3h` (tip `445cbfe6` + this doc) · **Gate:** none of these
branches merge to master until every item below passes on the canonical Linux box.

Everything in the stack was written and verified on a Mac, which cannot compile
Linux. All Linux-side edits were deliberately mechanical (cfg gates, moves,
re-exports, import swaps) with zero intended behavior change — this checklist is
the proof pass. Check out `osx/step-3h` and run top to bottom.

---

## 1. Build

- [ ] `cargo check --workspace --all-targets` — clean, no new warnings.
  Watches every itemized risk spot at once; the rest of this section is where to
  look first if it fails.
- Specific compile-risk spots, by commit:
  - **3a (`17a2fd24`)** — `wayland-plugin-gui/src/lib.rs` root module aliases
    (`use plugin_gui_core::{app, error, size}`) + gated `pub use`s;
    `editor.rs`'s `EditorOptions` now a re-export from `plugin-gui-core`;
    `#![cfg(target_os = "linux")]` added to `tests/csd_close.rs`; the
    Wayland/EGL/glow deps moved under `[target.'cfg(target_os = "linux")']`
    (confirm `egui_glow` still resolves via the workspace dep there).
  - **3c (`ecd3e374`)** — `editor_host.rs` `RuntimeEditor` cfg pair +
    `native_api()`; `resonance-plugin/Cargo.toml` target tables (confirm the
    macOS-side `dep:cocoa-plugin-gui` feature entry resolves on Linux cargo);
    the 11 factory files; wayland stub-Editor removal; `examples/hello.rs`
    main gating; `tests/editor_size.rs` live-module gate;
    `tests/preset_bar_render.rs` now imports `plugin_gui_core::egui`.
  - **3d (`6e1ad82c`)** — `clap_host/gui.rs` compiles the aliased
    `CLAP_WINDOW_API_WAYLAND` into the identical call sequence;
    `engine/plugins.rs` assert helper is an empty no-op on Linux; the `libc`
    dep table is macOS-gated and must be invisible to Linux resolution.
  - **Step 1 (`96c48b57`)** — `output_pipewire` gating and the `MixFn` move to
    `mixer/callback/context.rs`; Linux paths were kept semantically identical
    but never compile-checked.

## 2. Test suite

- [ ] `./scripts/run-tests.py` — fully green, goldens included (this box is
  canonical for golden images; the Mac never re-blessed anything).
- [ ] The two new `harness = false` binaries (`resonance-gate` →
  `editor_open_cocoa`, `cocoa-plugin-gui` → `editor_size`) and 3h's
  `modal_reentrancy` print `skipped: …` and exit 0 on Linux — they must not
  fail, hang, or run anything.

## 3. Live Wayland editor guard (from a Wayland session)

The stack rewired the editor plumbing every plugin shares, so run both
`#[ignore]`d guards by hand (unchanged commands, per CLAUDE.md):

- [ ] `cargo test -p resonance-gate --test editor_open -- --ignored --nocapture`
  (note: its live test now calls `create(native_api(), true)` — behavior-
  identical on Linux, but this run is what proves that)
- [ ] `cargo test -p wayland-plugin-gui --test editor_size -- --ignored --nocapture`

## 4. bundle.sh output parity (3e, `20c2156e`)

- [ ] Run `scripts/bundle.sh` on this branch and on `master`, then diff the two
  `target/bundled/` trees and the script stdout — **byte-identical** expected.
  Only new executions on Linux: `while read` loops replacing `mapfile`, one
  `uname -s`, one false `[ "$os" = "Darwin" ]` per plugin.

## 5. App smoke (Wayland session)

- [ ] Launch `resonance-app`: PipeWire output negotiates as before, plugin scan
  finds `~/.clap` + `/usr/lib/clap` + `target/bundled/` (scan dirs unchanged on
  Linux; step 2 added only `$CLAP_PATH` — set it to a scratch dir with a .clap
  in it and confirm it's picked up, then unset).
- [ ] Open a bundled plugin's editor from the mixer — Wayland floating window,
  as before (negotiation now goes through `native_api()` / the aliased
  constant; same strings at runtime).
- [ ] `resonance-mcp` connects over the control socket at
  `$XDG_RUNTIME_DIR/resonance/control.sock` (step 2 moved the resolution into
  `resonance_control::socket`; XDG stays first on Linux, uid now via
  `libc::getuid()` — same value as the old `/proc/self` read).

## 6. After everything passes

Merge the stack in order (each branch fast-forwards onto the previous). Known
macOS-only debts, tracked in `macos-editor-plan.md` and the session notes, that
do NOT block the merge: per-platform hashes for the aarch64-failing parity/DSP
baseline tests, a macOS golden-image baseline set, plan item 3g
(embedded/`set_parent` hosting), and an editor open/close control method for
MCP parity.
