# macOS plugin-editor runtime — scoping plan (port step 3)

**Date:** 2026-08-21 · **Input:** the macOS port assessment (steps 1–2 landed on
`osx/step-1` / `osx/step-2`: the workspace's core crates compile on macOS, CLAP
scanning knows the macOS directories, `.clap` bundle dirs dlopen correctly)
· **Goal:** plugin editors that open on macOS — ours in our host first, with the
architecture shaped so embedding in third-party hosts is an increment, not a rewrite.

This plan was scoped without the ba workflow; items are todo-sized and carry
their own verification so they can be picked up one at a time in any
environment.

---

## 0. The constraints this plan is built on

1. **Linux behavior is frozen.** Every item below either states "no Linux
   behavior change" or names the exact shared code it touches. The Wayland
   runtime is in production; the macOS runtime is additive. Nothing on this
   machine can compile-check Linux, so any item touching shared code must get a
   Linux build + `./scripts/run-tests.py` run before merge (goldens are blessed
   on the canonical Linux box and must not be re-blessed here).
2. **The editor contract is already narrow — keep it.** Everything a plugin
   needs is `Editor::new(app, EditorOptions)` + show/hide/set_size/get_size/
   is_resizable/destroy (`wayland-plugin-gui/src/editor.rs:56-148`) and the
   two-method `EditorApp` trait (`src/app.rs:12-18`). The Cocoa runtime
   implements exactly this surface; nothing new is invented for v1.
3. **AppKit owns the main thread.** NSWindow/NSView creation, event handling,
   and teardown must happen on the process main thread. The Wayland runtime's
   model (own thread per editor, `editor.rs:67-72`) is impossible on macOS, so
   the Cocoa runtime inverts it: the window lives on the main thread and the
   `Editor` handle is a `Send` command sender that dispatches onto it. Our
   host currently drives the CLAP gui extension from the **engine control
   thread** (`resonance-audio/src/engine/plugins.rs:518-534` — "main thread"
   there means CLAP's logical main thread, not the process main thread), which
   is exactly why the runtime must do its own main-thread dispatch internally
   rather than trusting its caller.
4. **The plugin decides scale, not the host.** `set_scale` stays refused
   (`resonance-plugin/src/clap_bridge/gui.rs:57-75` — the CLAP contract says
   returning false means "the plugin queries the OS directly"). On macOS the
   OS answer is `backingScaleFactor`; only the justification comment changes.
5. **No new dependency families.** `objc2`, `objc2-app-kit`,
   `objc2-foundation`, `dispatch2`, and `block2` are already in `Cargo.lock`
   (via winit/iced); `egui`/`egui_glow` are workspace deps. The Cocoa runtime
   is built from those. wgpu/Metal in every plugin cdylib is explicitly
   rejected for v1 (see §2).

---

## 1. Threading model (decision)

**The Cocoa runtime is a main-thread window controller behind a `Send`
handle.** Same two-piece shape as the Wayland runtime — handle + command
channel + `SharedSize` mirror (`editor.rs:47-54`) — with "the editor thread"
replaced by "the main thread":

- `Editor::new` may be called from any thread. It runs window construction on
  the main thread (dispatch2 main queue / `MainThreadMarker::run_on_main`),
  synchronously, mirroring the existing ready-handshake (`editor.rs:74-83`).
  The `EditorApp` (already `Send + 'static`, `app.rs:12`) moves into the
  main-thread controller; `ui()` is invoked on the main thread each frame.
- Repaint is driven by a display link (CVDisplayLink callback →
  `setNeedsDisplay` on main), paused while hidden; egui's
  `request_repaint` feeds the same path. The host's run loop pumps events —
  the runtime never runs a loop of its own, because inside a CLAP host it
  doesn't own one.
- `destroy()` dispatches teardown to the main thread and waits, exactly as the
  Wayland handle joins its thread (`editor.rs:142-147`). The failure mode is
  the same one `editor_open` guards against — a wedge, not an error — so the
  same wall-clock-watchdog test philosophy applies (§8).
- **Deadlock rule:** the main thread must never block on the engine control
  thread while the engine control thread is inside `Editor::new`/`destroy()`.
  Today the app thread only sends non-blocking `AudioCommand`s, so this holds;
  item 3b's verification includes stating and asserting it.
- Consequence for the bridge: none. `PluginEditor: Send`
  (`resonance-plugin/src/gui.rs:21`) is satisfied honestly — the handle owns
  no Objective-C objects, only the channel and the size mirror.

## 2. Rendering backend (decision)

**NSOpenGLView + `egui_glow` for v1.** OpenGL on macOS is deprecated but
ships and runs (GL 4.1 core); `egui_glow` is already the painter the Wayland
runtime uses, so the frame loop, mesh upload, and texture handling are the
same library with a different context provider (`NSOpenGLContext` instead of
EGL — the only GL-platform code in the Wayland runtime is
`egl_context.rs`, 226 lines, and the Cocoa equivalent is smaller).
egui-wgpu/Metal would put wgpu into all 11 plugin cdylibs — a large compile
and binary cost for zero v1 benefit. **Migration path:** the GL specifics
stay confined to one context module inside `cocoa-plugin-gui`; if Apple ever
removes GL, a `CAMetalLayer` + egui-wgpu backend swaps in behind the same
`Editor` API without touching plugins.

## 3. Floating vs embedded (decision)

**v1 ships floating NSWindow; the NSView is the unit it's built from.** Our
host only opens floating editors (`resonance-audio/src/clap_host/gui.rs:33,39`
passes `is_floating=true`), our product UX on Linux is floating windows, and
`RuntimeEditorHandle` (`resonance-plugin/src/editor_host.rs`) assumes the
floating lifecycle. So v1 = the plugin creates its own NSWindow whose content
view is the runtime's egui NSView — the exact Cocoa translation of today's
Wayland contract, and the smallest thing that makes editors open on macOS.

The runtime is structured so the egui NSView is a standalone component and
the floating NSWindow is a thin wrapper around it. Embedded mode
(`is_floating=false` + `set_parent` attaching that view to a host-supplied
NSView) is then an increment: it is what third-party macOS hosts (Live,
Bitwig, Reaper) actually negotiate, and it is deliberately deferred to item
3g rather than dropped — `set_parent`'s current pretend-success
(`clap_bridge/gui.rs:114-119`) is harmless while we only ever answer floating
support, and becomes a real implementation in 3g, not before.

## 4. Crate layout (decision)

**Extract a platform-neutral core crate; add a sibling runtime crate; route
all plugin imports through `resonance-plugin`.** Today 29 files across
resonance-plugin and the 11 plugins import `wayland_plugin_gui` directly, but
what most of them use is pure egui: `widgets.rs` (666 lines) and `theme.rs`
import no Wayland anything, and `resonance-plugin`'s `editor_widgets.rs` /
`preset_ui.rs` consume only those plus the `egui` re-export.

- **`plugin-gui-core`** (new crate): `EditorApp`, `EditorOptions`,
  `EditorError`, `SharedSize`, `theme`, `widgets`, the `egui` re-export.
  No windowing code, builds everywhere. A new core crate (rather than folding
  into resonance-plugin) is forced by the dependency direction:
  resonance-plugin already depends on the GUI runtime
  (`resonance-plugin/Cargo.toml:11,23`), so the runtime can't depend back on
  resonance-plugin.
- **`wayland-plugin-gui`** keeps `Editor` + `window_thread` + `egl_context` +
  `input` + `size`, now depending on `plugin-gui-core` for the shared types.
  Its Wayland deps move under `[target.'cfg(target_os = "linux")']` and its
  `lib.rs`/tests get a crate-level `#![cfg(target_os = "linux")]` with an
  empty stub otherwise — this is what lets it **stay a workspace member** (so
  `-p` addressing and `run-tests.py` keep working) while
  `cargo check --workspace` passes on macOS. Workspace `members` arrays
  cannot be per-platform; stubbing the crate body is the standard fix.
- **`cocoa-plugin-gui`** (new crate, same treatment inverted): the
  main-thread controller, NSOpenGL context module, input translation
  (NSEvent → egui), display link, and an `Editor` type with the identical
  public API. Deps (`objc2-app-kit` etc.) target-gated to macOS; stub
  elsewhere. Ships its own `examples/hello.rs` mirroring the Wayland one.
- **`resonance-plugin/src/editor_host.rs`** selects the runtime:
  `#[cfg(target_os = ...)] use {wayland,cocoa}_plugin_gui::Editor as
  RuntimeEditor;` and **re-exports `RuntimeEditor` + the core types**, so
  plugin factories import everything from `resonance_plugin::editor_host` and
  never name a platform crate again. That makes the 11-plugin migration an
  import swap (§6), and adding a future win32 runtime a one-line cfg.

## 5. Scale / Retina (decision)

`pixels_per_point` comes from the view's `backingScaleFactor`, re-read on
`viewDidChangeBackingProperties` (monitor moves, scaled-resolution changes) —
the direct analog of the Wayland runtime's
`CompositorHandler::scale_factor_changed` single-source-of-truth
(`clap_bridge/gui.rs:62-67`). All `EditorOptions` sizes and the
`get_size`/`set_size` contract stay in logical pixels, which is what both
Wayland and Cocoa window geometry natively speak — no unit change anywhere in
the plugin-facing API. `set_scale` stays refused; only its comment is
rewritten to cover both platforms.

---

## 6. Work breakdown

Each item: size (S/M/L), files, verification. Ordering is the listed order;
3e/3f are parallel-safe after their stated dependencies.

**3a. Extract `plugin-gui-core` and target-gate `wayland-plugin-gui` — M.**
Move `app.rs`, `error.rs`, `size.rs`, `theme.rs`, `widgets/` out of
`wayland-plugin-gui` into the new core crate; move the pure widget tests
(`tests/kit_widgets.rs`, `tests/knob_input.rs`) with them; wayland-plugin-gui
re-exports the moved types so its own API is unchanged. Gate wayland deps and
crate body to Linux. Update `resonance-plugin` (`editor_widgets.rs:45,120`,
`preset_ui.rs:26`) to import from the core crate; add the `RuntimeEditor`
re-export in `editor_host.rs`. *Touches shared Linux code (pure moves +
re-exports, no logic).* Verify: full suite on the Linux box, goldens
untouched; `cargo check --workspace` passes on macOS with the wayland crate
stubbed.

**3b. `cocoa-plugin-gui` runtime v1 — L (the big one).**
New crate per §1/§2/§3/§4: main-thread controller (NSWindow + egui NSView),
NSOpenGL context module, NSEvent→egui input translation (pointer, scroll,
keyboard, modifiers — clipboard/DnD/IME explicitly out, same as the Wayland
v1 scope note at `wayland-plugin-gui/src/lib.rs:32`), CVDisplayLink repaint
with `request_repaint` integration and hidden-pause, `SharedSize` publication
on every applied size, close-button → `on_close` exactly once, watchdog-safe
`destroy()`. Includes `examples/hello.rs`. A reentrancy guard skips repaint
while a modal run loop is active (see 3h). *No Linux behavior change (new
crate).* Verify: `cargo run -p cocoa-plugin-gui --example hello` on the Mac —
widgets draw, close button fires `on_close` once, teardown under 5s; plus
headless unit tests for input mapping and size bookkeeping.

**3c. Per-plugin factory migration — M total (11 × S, mechanical).**
In each factory (`plugins/*/src/editor/factory.rs` or `editor/mod.rs`):
imports switch to `resonance_plugin::editor_host`; the hard-coded strings at
e.g. `resonance-gate/src/editor/factory.rs:44-49` become a shared helper
(`editor_host::native_api() -> &'static str` returning `"wayland"`/`"cocoa"`
per cfg) so `supports`/`preferred` are platform-correct everywhere at once.
`EditorOptions` unchanged (`app_id` is ignored by Cocoa v1; title is used).
Update the two `set_scale`/`set_parent` comments in `clap_bridge/gui.rs`.
*Touches shared code all 11 plugins compile; behavior on Linux identical
(same strings come out of the helper).* Verify: headless factory-negotiation
tests (the pure half of `editor_open.rs`) extended to assert the platform's
api name on both OSes; Linux suite green.

**3d. Our host opens Cocoa editors — S/M.**
`resonance-audio/src/clap_host/gui.rs`: negotiate
`CLAP_WINDOW_API_COCOA`(floating) on macOS instead of the hard-coded
`CLAP_WINDOW_API_WAYLAND` (`gui.rs:6,33,39`) — a cfg-selected constant, same
sequence. Document (and assert in a debug build) the §1 deadlock rule at the
`handle_open_plugin_editor` call site (`engine/plugins.rs:518`). *Shared
file; Linux path compiles to the identical constant.* Verify: on the Mac,
launch the app, insert each bundled plugin, open/close every editor twice;
on Linux, suite green.

**3e. `bundle.sh` macOS layout — S/M.** (after 3c; parallel with 3d)
Per-OS artifact branch at `scripts/bundle.sh:118-122`: Darwin emits
`Foo.clap/Contents/MacOS/Foo` from `lib*.dylib` plus a minimal generated
`Info.plist` with `CFBundleExecutable`/`CFBundleIdentifier` (the step-2
loader reads exactly that key, falling back to the file stem —
`resonance-audio/src/clap_host/bundle.rs:bundle_binary_path`). Fix the bash-4
dependency: `#!/usr/bin/env bash` + an explicit version check, or replace
`mapfile` (`bundle.sh:51-52`) with portable reads. *No Linux behavior change
(Linux branch byte-identical output).* Verify: run on the Mac → 11 bundles;
scanner catalogs and loads all 11; run on Linux → output identical to
pre-change (diff the bundled dir).

**3f. Editor lifecycle tests, macOS analog — M.** (after 3b/3c)
A `#[ignore]`d live round-trip mirroring
`plugins/resonance-gate/tests/editor_open.rs` (create → show → size →
set_size → hide → drop under the same wall-clock watchdog — the failure mode
on macOS is the same teardown wedge, now a main-thread-dispatch deadlock
instead of a thread-join hang), runnable from a logged-in Mac session; and a
`cocoa-plugin-gui/tests/` size test in the spirit of `editor_size.rs`
asserting **requested** sizes only (the Wayland test's warning about
compositor-imposed sizes maps to macOS zoom/full-screen). Document both in
CLAUDE.md's ignored-tests section. *No Linux behavior change.* Verify: both
pass on the Mac with `-- --ignored`; default `run-tests.py` stays headless on
both OSes.

**3g. Embedded mode + third-party editor hosting — L, deferred (v1.5).**
Plugin side: real `set_parent` (attach the egui NSView to the host's NSView,
`is_floating=false` supported in `supports()`); host side: marshal all CLAP
gui-extension calls to the process main thread on macOS (third-party plugins
require the *real* AppKit main thread, unlike ours whose runtime
self-dispatches — this is the one structural host change, and why it is its
own item), and wrap embedded-only third-party editors in a host-created
NSWindow. *Touches `engine/plugins.rs` threading; needs its own design pass
at pickup time.* Verify: our plugins embedded in a third-party macOS CLAP
host (Reaper is the cheapest); a third-party Cocoa plugin's editor open in
our app.

**3h. rfd modal audit — S.** (with or after 3b)
`rfd::FileDialog` (blocking) is called from editor `ui()` in amp
(`editor/header.rs:166`), drums (`editor/kit_browser.rs:75`), ir
(`editor/header.rs:119`). Under §1 these now run on the main thread, where
`NSOpenPanel`'s modal loop is *legal* — but it re-enters the run loop, so 3b's
repaint reentrancy guard is load-bearing; this item verifies each of the
three dialogs on the Mac and adds the guard test. *No Linux behavior change.*

---

## 7. Explicitly out of scope

- **Code signing / notarization / Gatekeeper** — dev builds run locally
  unsigned; distribution packaging is its own future plan.
- **AU / VST3 wrappers** — nothing in the tree needs them; CLAP-only stands.
- **Clipboard, DnD, IME, fractional-scale work** — matching the Wayland
  runtime's v1 scope note (`lib.rs:32`), not exceeding it.
- **Golden-image baselines for macOS** and the aarch64 parity-test hashes —
  tracked in the port assessment, orthogonal to editors.

## 8. Branch plan

Stacked off `osx/step-2`, one branch per item, merged in order:
`osx/step-3a` (core extraction — the only branch needing a Linux full-suite
gate before anything stacks on it), `osx/step-3b` (cocoa runtime),
`osx/step-3c` (factories), `osx/step-3d` (host), then `osx/step-3e`/`3f`/`3h`
in any order. `osx/step-3g` starts from a fresh design pass after v1 ships.
The definition of done for v1: on the Mac, every bundled plugin's editor
opens, resizes, and closes cleanly from the app, twice in a row, with the
suite green on both platforms.
