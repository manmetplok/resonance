# Settings: expose render threads, backup retention and keymap unbind

> Stage: **intent**
> Status: **idea**
> Area: ui-shell
> Touches: Settings
> Source: code check at `8eac71b3`. `AppSettings` fields with no UI; the prototype's Preferences pages (`docs/history/prototypes/keyboard-shortcuts-command-palette/index.html`: Audio & MIDI, Appearance).

## Why

Some per-machine options that matter can only be changed by editing
`settings.json` by hand:

- **`audio.render_threads`** sets how many threads the live mixer renders
  on. It is the setting most worth changing on a machine where the RT
  workers starve USB audio (the audio-stutter fix merged at `8eac71b3`).
- **`autosave.backup_retention`** sets how many manual-save backups to keep.

The Keyboard page can't unbind a command either. `KeymapMsg::Unbind` exists,
but no button sends it.

## What it would do

- Add a **General › Audio** row "Render threads": Auto, 1, 2, …, N. A note
  says it applies on the next launch and that `RESONANCE_RENDER_THREADS`
  overrides it.
- Add a **General › Project** row "Keep N backups" (0 = off).
- Add an **Unbind** button on each Keyboard row that has a chord.

## What exists to build on

- `resonance-app/src/settings.rs`: `AudioSettings::render_threads`,
  `AutosaveSettings::backup_retention`, `persist`.
- `lib.rs` reads `render_threads` once at startup.
- `engine_events/project_io.rs` reads `backup_retention` after a manual
  save.
- `view/settings.rs`: the `toggle_button` / `pick_list` row patterns and
  `AUTOSAVE_INTERVALS`.
- `update/keymap.rs`: `KeymapMsg::Unbind` → `push_override(None)`.
- `docs/features/ui-shell/settings.md`, `keymap-editor.md`.

## Open decisions

1. Should a render-thread change apply live? **Recommend next launch
   only.** The pool is built at engine start. Say so in the UI.
2. What upper bound for render threads? **Recommend physical cores.**
3. Should the prototype's Appearance page (themes, density) be built?
   **Recommend no**: there is one theme.

## Slices

1. The Unbind button, with a test in `performance/keymap_settings.rs`.
2. The render-threads picker, with a settings round-trip test and a golden
   of the General page.
3. The backup-retention picker.

## How we'd know it works

- Settings round-trip tests.
- The updated `settings_overlay_*` goldens.
- By hand: relaunch and check the engine logs the chosen thread count.
