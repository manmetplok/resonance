# Control API: review leftovers

> Stage: **intent**
> Status: **idea** (checklist)
> Area: control-api
> Source: [code review 2026-10-02](../../history/reviews/code-review-2026-10-02.md) FU-S1b, FU-S1c, FU-S1d, FU-A1b6;
> [macOS Linux verification](../../history/macos/macos-linux-verification.md) §6

## Why

Small control-surface items the review campaigns left open. None is a
user-visible defect today; each makes the surface harder to trust or
extend.

## What it would do

- [ ] **FU-S1b** — an end-to-end MCP `call_tool` test (today only the
      router is unit-tested); stop rebuilding `combined_router()` on every
      call (`resonance-mcp/src/server.rs:470`).
- [ ] **FU-S1c** — the first `presets.*` call with no plugin id builds and
      scans the `PresetLibrary` on the update loop; warm it off-thread at
      startup (`resonance-app/src/plugin_preset_library.rs:60-72`).
- [ ] **FU-S1d** — nested leaf specs still accept unknown fields
      (`AmountSpec` in `resonance-control/src/methods/clip.rs:118-120` has
      no `deny_unknown_fields`); add it everywhere strict params are
      promised.
- [ ] **FU-A1b6** — `chain_name` / `params_method` are free functions
      beside `ChainOwner` (`resonance-app/src/update/control/plugin_target.rs:24,33`);
      fold them into the owner type.
- [ ] **Plugin editor open/close method** — the GUI can open a plugin's
      editor, an agent cannot (no method in `resonance-control`); add
      `*.open_editor` / `close_editor` for parity, so an agent can show the
      user what it changed.

## What exists to build on

[`../../features/control-api/protocol.md`](../../features/control-api/protocol.md),
[`../../features/control-api/mcp-server.md`](../../features/control-api/mcp-server.md),
[`../../guides/control-api-conventions.md`](../../guides/control-api-conventions.md).

## Open decisions

None beyond each item.

## Slices

One commit per checkbox; the editor method is a full vertical slice.

## How we'd know it works

Each item's test: an MCP round-trip test, a strict-params rejection test,
a control test opening and closing an editor on the capturing engine.
