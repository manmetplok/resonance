# Plugin windows

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/mixer-cleanup.md` §4 (generic window);
> field report 2026-10-06 (editor self-close).

A plugin's controls can be opened from the mixer in two ways. A plugin with
its own CLAP GUI opens its **native editor**, a separate floating OS window
that the plugin draws itself. A plugin without one, or a user who asks for
"generic params", gets the **generic window**: a card drawn by the host
inside the main window, listing every parameter, with a preset bar. Only one
generic window is open at a time.

## Behaviour

**Opening.**

- `PluginMessage::OpenPluginWindow` comes from a double-click on a strip
  slot or a CHAIN row, or from the CHAIN ↗ toggle.
- `update/plugin_window.rs::open` opens the native editor when the slot has
  a GUI and the plugin is available, and the generic window otherwise.
- The CHAIN ☰ entry "generic params" (`OpenGenericParams`) forces the
  generic window.
- If a native editor fails to open, the app falls back to the generic
  window (`engine_events/plugins.rs::editor_state`).

**Generic window** (`view/plugin_window.rs::view_plugin_window`). It is a
`pin(opaque(card))` layer stacked between the base view and the root
overlays, so it is non-modal and a modal opened from it (the preset browser)
lands on top. The card holds:

- A title bar with the plugin name, the owner, Open/Close Editor (for
  plugins that have a GUI) and ×.
- The preset bar ([preset-browser.md](preset-browser.md)).
- The parameters from `plugin_ui::view_generic_params`, in a `lazy` region
  keyed on `plugin_params_fingerprint`. Right-clicking a parameter opens the
  MIDI Learn menu.
- For a missing plugin, a placeholder body.

How it behaves:

- **Position:** `PluginWindowState { instance_id, position, drag }`
  defaults to (240, 150).
- **Clamping:** the position is clamped so a 120×30 strip of the title bar
  stays on screen, and re-clamped on window resize.
- **Dragging:** dragging the title bar moves the card. The drag ends on
  release, when the pointer leaves the window, or when the window loses
  focus.
- **Esc** closes the window when no modal overlay is up.
- **Hidden** in Performance mode, and while the Arrange track menu or its
  preset prompt is open. The layer stays in the tree, so opening or closing
  it never resets scroll offsets underneath.
- **Focus:** opening focuses the slot (`focused_slot`) and selects its
  owning channel. Removing the slot closes the window.

**Native editor**

- `PluginMessage::OpenPluginEditor` / `ClosePluginEditor` send
  `AudioCommand::OpenPluginEditor` / `ClosePluginEditor`. Close also saves
  the plugin state.
- The app never sets `slot.editor_open` itself. It mirrors the engine's
  `AudioEvent::PluginEditorState { instance_id, open, failure }`.
- Editors are floating top-level windows: Wayland on Linux, Cocoa on macOS,
  never embedded.
- When the user closes the plugin's own window, the host is told through
  `clap_host_gui.closed()`. The engine destroys the GUI and emits
  `open: false`, so the app's toggle follows.

The engine side and the editor runtimes are documented in
[../plugins-instruments/plugin-editor-runtimes.md](../plugins-instruments/plugin-editor-runtimes.md)
and [../plugins-instruments/third-party-plugin-hosting.md](../plugins-instruments/third-party-plugin-hosting.md).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Window state | `resonance-app/src/state/plugin_window.rs` | `PluginWindowState`, `clamp`, `MixerUiState::{forget_plugin, close_plugin_window_for}` |
| Reducer | `resonance-app/src/update/plugin_window.rs` | `open`, `open_generic`, `focus`, `drag_end_event`, `viewport_resized`, `visible`, `escape`, `preset_target` |
| Generic params | `resonance-app/src/plugin_ui.rs` | `UiParam`, `PluginUiEvent`, `param_display`, `view_generic_params` |
| View | `resonance-app/src/view/plugin_window.rs`, `view/mod.rs` | `view_plugin_window`, `plugin_params_fingerprint`, the stack in `view` |
| Editor open/close | `resonance-app/src/update/plugin.rs`, `engine_events/plugins.rs` | `OpenPluginEditor`, `ClosePluginEditor`, `editor_state` |
| Engine side | `resonance-audio/src/clap_host/gui.rs` | `open_gui`, `take_gui_closed` |

## Surfaces

- **GUI:** mixer strip slots and the inspector CHAIN (double-click, ↗, ☰);
  Esc closes the generic window.
- **Control API / MCP:** windows can't be opened remotely. Parameters are
  reachable through `track.plugin_params` / `track.set_plugin_param` (and
  the bus/master twins) → `mcp__resonance__track_plugin_params`,
  `mcp__resonance__track_set_plugin_param`.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Focus, preset target, window lifecycle | `resonance-app/tests/mixer/plugin_focus_window.rs` | `./scripts/run-tests.py -p resonance-app` |
| `lazy` fingerprint | `resonance-app/tests/mixer/plugin_window_fingerprint.rs` | same |
| Open routing, editor-failure fallback | `resonance-app/tests/mixer/mixer_generic_param_panel.rs` | same |
| Editor state echo (engine) | `resonance-audio/tests/clap_host/plugin_editor_state.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Golden | `resonance-app/tests/snapshots/plugin_generic_window-wgpu.png` | `RESONANCE_BLESS=1 …` |
| Real editor windows | `cargo test -p resonance-gate --test editor_open -- --ignored`, `wayland-plugin-gui` `editor_size` / `editor_close` (Wayland session; see CLAUDE.md) | by hand |

Not covered: dragging the generic window with a real pointer, and resizing
the main window while the generic window is open (only `viewport_resized`
is unit-tested).

## Decisions and deviations

- One generic window at a time. Opening another plugin's window replaces it.
- The generic window lives inside the app window, not as an OS window, so
  it works the same on every platform and stays under the app's modals.

## Known gaps

- None recorded in code beyond the editor-runtime items in the
  plugins-instruments area.
