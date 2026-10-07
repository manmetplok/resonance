# UI shell

The UI shell is the part of `resonance-app` around the editors:

- the window chrome and the four views (Arrange, Mixer, Compose,
  Performance);
- the command registry behind every keyboard shortcut, the command
  palette, and the keymap editor;
- the one-at-a-time root overlay chain that holds every dialog and popover,
  and the status area;
- Settings and `settings.json`;
- the startup screen, recent projects and the autosave-recovery prompt;
- the docked media browser and the host side of the plugin preset library;
- the generic plugin window and native editor toggles;
- Performance mode (a chord teleprompter);
- the theme tokens and view-performance mechanisms that keep `view()` cheap;
- the golden-PNG snapshot harness that checks all of it.

The editors themselves are in [../timeline/](../timeline/),
[../mixer/](../mixer/) and [../compose/](../compose/). Project I/O internals
(autosave writer, templates, pool) are in
[../project-core/](../project-core/). The always-true design rules are in
[`docs/guides/ux-guidelines.md`](../../guides/ux-guidelines.md).

| Feature | Doc | One-line summary |
|---|---|---|
| Command registry and shortcuts | [command-registry-and-shortcuts.md](command-registry-and-shortcuts.md) | 102 `CommandId`s with gates, scopes and availability. The global key dispatch order and the default keymap. |
| Command palette | [command-palette.md](command-palette.md) | ⌘K fuzzy command search with recents, suggestions, dimmed unavailable rows, and `:` `@` `#` `+` argument modes. |
| Keymap editor and presets | [keymap-editor.md](keymap-editor.md) | Settings › Keyboard: Logic Pro / Pro Tools presets, rebind by pressing a chord, conflict replace, overrides in `settings.json`. |
| Window chrome and views | [window-chrome-and-views.md](window-chrome-and-views.md) | The 1440×900 window, view tabs and `--tab`, the chrome row, the transport bar slot, the Remote chip, window file drops. |
| Overlays, dialogs and status area | [overlays-and-dialogs.md](overlays-and-dialogs.md) | One root overlay in a fixed priority that both renderer and key gate read, Esc dismiss, the quit/switch confirms, AUDIO/AUTOSAVE/ERROR lines. |
| Settings | [settings.md](settings.md) | `settings.json` sections and the General / Keyboard / MIDI overlay (autosave, follow playhead, latency, plugins, MIDI clock). |
| Startup screen and recovery | [startup-and-recovery.md](startup-and-recovery.md) | The modal New session screen, `recent.json`, and the "Recover unsaved work?" prompt on open and at boot. |
| Media browser panel | [media-browser-panel.md](media-browser-panel.md) | The docked Files / Pool / Presets panel in Arrange with audition transport. Drag-to-timeline is not wired. |
| Host preset browser and track presets | [preset-browser.md](preset-browser.md) | Preset bar, audition-on-arrow overlay with one-undo keep and Esc revert, Media › Presets with drag-to-header, track presets. |
| Plugin windows | [plugin-windows.md](plugin-windows.md) | The host-drawn floating generic parameter window, and native CLAP editor open/close mirrored from engine events. |
| Performance mode | [performance-mode.md](performance-mode.md) | A full-bleed chord teleprompter: hero chord, fingering diagram, beat ring and count-in, up-next lane, instrument/capo. |
| Theme and view performance | [theme-and-view-performance.md](theme-and-view-performance.md) | `theme.rs` tokens and styles, two-rate tick, cached pick lists, `lazy` regions, Canvas caches, and the arch-invariant checks. |
| UI snapshot testing | [ui-snapshot-testing.md](ui-snapshot-testing.md) | `iced_test` goldens via `assert_golden`, the absent-golden guard, `RESONANCE_BLESS=1`, `RESONANCE_SKIP_GOLDENS=1`. |

Open intents for this area are in
[`docs/intents/ui-shell/`](../../intents/ui-shell/).
