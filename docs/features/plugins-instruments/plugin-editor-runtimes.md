# Plugin editor runtimes

> Verified against master @ `5d6a2993` on 2026-10-15 (egui 0.37 upgrade).
> Spec history: `docs/history/macos/macos-editor-plan.md`,
> `docs/history/macos/macos-linux-verification.md`.

Every first-party plugin draws its editor with egui in a floating window of
its own. Three crates make that work on Linux and macOS: a platform-neutral
core with the editor contract, theme and widgets, and one windowing runtime
per platform. A plugin never names a runtime; it gets the right one through
`resonance_plugin::editor_host`.

## Behaviour

- **Floating only.** Editors open as their own top-level window. Embedded
  hosting (`clap_gui.set_parent`) is not implemented; the bridge pretends it
  succeeded so hosts that call it anyway do not fail
  (`resonance-plugin/src/clap_bridge/gui.rs`).
- **Linux (`wayland-plugin-gui`).** One thread per editor opens its own
  `wl_display` connection, an `xdg_toplevel`, an EGL context, and runs an SCTK
  event loop that drives `egui_glow` each frame. The caller holds a `Send`
  `Editor` handle that sends show/hide/resize/destroy commands over a
  channel. The runtime draws its own client-side frame with a close button on
  every compositor, because wlroots compositors honour a server-side
  decoration request with no titlebar at all. `WPG_FORCE_SSD` asks for
  server-side decorations instead (useful on KWin); `WPG_FORCE_CSD`,
  `WPG_DUMP_FRAME[_AT]` and `WPG_TEST_CLOSE_AT` are debug/test switches.
- **macOS (`cocoa-plugin-gui`).** AppKit needs windows on the process main
  thread, so the model is inverted: the window, an `NSOpenGLView` and the
  `EditorApp` live in a main-thread controller; the `Send` `Editor` handle
  only dispatches to the main queue. A 60 Hz `NSTimer` paces repaints (only
  when egui asked). A reentrancy guard skips nested `drawRect:` while a modal
  `rfd` dialog runs inside `ui()`. Panics inside Cocoa callbacks are caught.
  `destroy()` from another thread is queued, not waited for, so quitting with
  an editor open cannot deadlock the engine thread against the main thread.
  Objective-C classes are registered under a per-image name
  (`window_main_thread/runtime_class.rs`) because every plugin bundle links
  its own copy of the crate.
- **Close.** Closing from the window (close button or compositor) calls
  `EditorApp::on_close` exactly once and notifies the host through
  `CloseNotifier`; the event loop and the Wayland connection really go away.
- **Teardown watchdog.** `Editor::destroy`/`Drop` joins the editor thread
  with a bound (`join_with_timeout`), and `Editor::new` waits on a bounded
  startup handshake (`await_startup`), so a plugin `ui()` stuck in a modal
  loop cannot wedge the host.
- **Sizes.** The runtime publishes every applied size into `SharedSize`;
  `get_size` reports it. Tiling compositors override the requested size,
  so tests assert requested sizes only.
- **Scope limits (both runtimes):** no clipboard, drag and drop or IME; no
  window retitle.
- **Off-platform builds.** Each runtime compiles its windowing body only on
  its own OS and leaves the `plugin-gui-core` re-exports, so the workspace
  builds on both.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Editor contract | `plugin-gui-core/src/{app,options,error,size,close,repaint}.rs` | `EditorApp`, `EditorOptions`, `EditorError`, `SharedSize`, `CloseNotifier` |
| Theme + widgets | `plugin-gui-core/src/theme.rs`, `widgets.rs`, `widgets/{chip,library,segmented,slider}.rs` | `theme::lavender`, `ThemedKnob`, `knob_themed_edit`, `GestureEdit` |
| Wayland runtime | `wayland-plugin-gui/src/{editor,egl_context,input,join}.rs`, `window_thread/{event_loop,delegates,paint,decorations,state}.rs` | `Editor`, `InputState`, `join_with_timeout` |
| Cocoa runtime | `cocoa-plugin-gui/src/{editor,gl_context,input}.rs`, `window_main_thread/{mod,view,delegate,runtime_class}.rs` | `Editor`, `RuntimeDefined` |
| Platform selection | `resonance-plugin/src/editor_host.rs` | `RuntimeEditor`, `native_api`, `RuntimeEditorHandle`, `AnnouncingApp` |
| Host side (opening a plugin's GUI) | `resonance-audio/src/clap_host/gui.rs` | see [clap-hosting.md](../audio-engine/clap-hosting.md#editors) |

Invariants: no plugin depends on a runtime crate directly, and a GUI toolkit
stays in its own crate (`tools/arch-invariants`). The Cocoa main thread must
never block on a thread that is inside `Editor::new` (the one remaining
synchronous dispatch).

## Surfaces

- **GUI:** the plugin editor windows, opened from the mixer's plugin slots.
- **Control API / MCP:** none. There is no control method to open or close
  an editor.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Widgets, knob input, repaint decision, close notifier | `plugin-gui-core/tests/{kit_widgets,knob_input,repaint,close_notifier}.rs` | `./scripts/run-tests.py -p plugin-gui-core` |
| Wayland headless: CSD hit-test, input bookkeeping, join watchdog | `wayland-plugin-gui/tests/{csd_close,input_state,join_watchdog}.rs` | `./scripts/run-tests.py -p wayland-plugin-gui` |
| Cocoa headless: NSEvent mapping | `cocoa-plugin-gui/tests/input_map.rs` | runs on every platform |
| Factory negotiation + teardown guard (headless half) | `plugins/resonance-gate/tests/editor_open.rs` | default suite |
| By hand, Wayland session | `editor_open` (lifecycle under a wall-clock watchdog), `wayland-plugin-gui` `editor_size`, `editor_close` (self-close) | `cargo test -p resonance-gate --test editor_open -- --ignored --nocapture` etc., see CLAUDE.md |
| By hand, macOS session (`harness = false`) | `plugins/resonance-gate/tests/editor_open_cocoa.rs`, `cocoa-plugin-gui/tests/{editor_size,modal_reentrancy}.rs` | `cargo test -p cocoa-plugin-gui --test modal_reentrancy -- --ignored --nocapture` etc. |
| Editor content goldens | per-plugin `tests/editor_*.rs`, `resonance-plugin/tests/{preset_bar_render,library_ui_render}.rs` | plugin suites |

Not covered by the default suite: anything that needs a compositor or
AppKit. The by-hand tests were last recorded passing on macOS on 2026-09-29
(commit `483464e2`); Cocoa-side changes after that date are type-checked on
Linux only.

## Decisions and deviations

- Thread-per-editor on Linux, main-thread controller on macOS (plan §1).
- Client-side decorations by default on Wayland, so the close button exists
  on every compositor.
- The macOS stack (`osx/step-1` … `osx/step-3h`, plus PLG-01..05 fixes and
  per-image ObjC classes) is merged. The plan's CVDisplayLink repaint became
  an `NSTimer` in common run-loop modes.
- Plan item 3g (embedded mode and hosting third-party Cocoa editors from the
  main thread) was deferred and is not built.
- **egui 0.37 / egui_glow 0.37, glow 0.17.** glow stays on 0.17 because
  `egui_glow::Painter` takes the runtime's `glow::Context`, and egui_glow
  0.37 still links glow 0.17. What the 0.34 → 0.37 upgrade changed for us:
  - Held modifiers reach egui as `Event::ModifiersChanged` (the
    `RawInput::modifiers` field is gone). Wayland sends one on every
    `wl_keyboard.modifiers`; Cocoa and the headless harnesses state them
    at the start of every frame. Wheel events carry
    `MouseWheelSource::Unknown`.
  - A dropped `TexturesDelta` is a debug assertion: the runtimes hand it to
    the painter by `&mut`, every headless harness `clear()`s it, and the
    Cocoa paint clears it when the view was torn down mid-frame.
  - `theme::apply` pins what egui 0.37 restyled — button padding 4×1 and
    row height 18 (0.37: 8×2 and 20), widget corner radii 2–3 (0.37: 4–6)
    and the hovered/active button fills — so every editor keeps its
    measured layout and look. Because spacing is read when a pass starts,
    `apply_once` asks egui to discard and re-run the first frame; layout
    probes skip a pass that will be discarded.
  - `Panel::exact_size` is now the outer size including the frame stroke
    and a 1 px separator line (0.34 added the stroke on top), so fixed
    header/status bars are 2 px shorter outside and 3 px inside.
  - `Visuals::clip_rect_margin` no longer does anything: clips are exact.
    That exposed the drums pad grid running 2 px past its scroll area at
    the minimum width (its card stroke was not budgeted), now fixed.
  - Text is shaped with harfrust (kerning), so some labels are a few px
    narrower.

## Known gaps

- Embedded editors, third-party macOS editor hosting, macOS golden baselines
  and a by-hand re-run after Cocoa changes:
  [macos-port-open-checks](../../intents/plugins-instruments/macos-port-open-checks.md).
- No clipboard, drag and drop or IME in editors.
