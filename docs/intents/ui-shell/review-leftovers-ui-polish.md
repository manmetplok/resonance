# UI polish: review leftovers

> Stage: **intent**
> Status: **idea** (checklist)
> Area: ui-shell
> Touches: with mixer strips and Compose
> Source: [code review 2026-10-02](../../history/reviews/code-review-2026-10-02.md) FU-S2d, FU-U1c, FU-U1d, FU-U2b, FU-U2c, FU-U3b, FU-U3c, FU-W3b, FU-A1b5, UX-07/UX-19 remainders;
> prototype `keyboard-shortcuts-command-palette` (⌘↵)

## Why

Small UI items the 2026-10-02 campaign left open after its main fixes.
None loses data; together they are the rough edges a user notices.

## What it would do

- [ ] **FU-S2d** — transient errors live in one slot with no expiry or
      queue; a second error overwrites the first (`resonance-app/src/state/banners.rs:56`).
      Queue them, expire non-sticky ones.
- [ ] **FU-U1c** — the preset-delete confirm text wraps badly in the
      200 px menu (`view/menus.rs:96-99`).
- [ ] **FU-U1d** — the fixed-width undo-notice slot permanently pushes the
      view tabs right (`view/transport.rs:93-111`); make it an overlay.
- [ ] **FU-U2b** — only Compose's right rail is `lazy`; wrap the left
      column (section strip, scale stripe, group headers) too
      (`view/compose/page.rs:238`).
- [ ] **FU-U2c** — a `view_compose` timing benchmark (no benches in
      resonance-app yet).
- [ ] **FU-U3b** — bus, group and sub-track M/S buttons have no hint
      tooltip (`view/mixer/bus_strip.rs`, `group_strip.rs`,
      `view/track_header/group_header.rs`).
- [ ] **FU-U3c** — track-name ellipsis uses a 12-character budget, not a
      measured width (`theme.rs:584`).
- [ ] **FU-W3b** — no transport-bar control for the loop-record mode
      (only the palette and `transport.set_loop_record_mode`).
- [ ] **FU-A1b5** — remove the 5-line import shims
      `view/arrange_layout.rs` and `view/mixer/picks.rs`.
- [ ] **UX-07 / UX-19 remainders** — two `.size(8)` texts below the 9 px
      floor (`view/remote_indicator.rs:33`, `pattern_picker.rs:273`);
      ~51 inline `from_rgb(a)` colours not yet theme tokens.
- [ ] **Command palette ⌘↵** — "run and keep open" from the prototype.

## What exists to build on

[`../../features/ui-shell/theme-and-view-performance.md`](../../features/ui-shell/theme-and-view-performance.md),
[`../../features/ui-shell/overlays-and-dialogs.md`](../../features/ui-shell/overlays-and-dialogs.md),
[`../../features/ui-shell/command-palette.md`](../../features/ui-shell/command-palette.md),
[`../../guides/ux-guidelines.md`](../../guides/ux-guidelines.md).

## Open decisions

None.

## Slices

One commit per checkbox (the banner queue and the overlay first).

## How we'd know it works

iced snapshot goldens for each visual item (re-bless with
`RESONANCE_BLESS=1`); a unit test for the banner queue.
