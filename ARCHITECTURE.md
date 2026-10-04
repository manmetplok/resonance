# Architecture

Patterns this codebase has gotten right. Imitate these when adding new code; resist drifting away from them.

## Crate Layering

The workspace is a deliberate DAG. Every crate has a single responsibility, and the lower layers know nothing about the upper layers.

```
resonance-dsp ──┬─► resonance-metering ──┬─► resonance-mastering plugin
                │                       └─► resonance-mastering-assist ──┬─► resonance-mastering plugin
                │                                                        └─► resonance-app
                ├─► resonance-audio ─────► resonance-app
                └─► (every FX plugin)
resonance-music-theory ──┬─► resonance-app                   (pure theory, no audio/app deps)
                         ├─► resonance-audio                 (vocal-tuning scale-snap only — doc #160/todo #358)
                         ├─► resonance-svs ──► resonance-app  (vocal synthesis)
                         └─► resonance-granular-delay plugin (scale-quantized grain pitch)
resonance-common ──► resonance-audio, resonance-plugin, amp/drums/ir plugins (utilities only)
resonance-plugin ──┬─► every plugin, resonance-app (UI helpers)
                   └─► wayland-plugin-gui / cocoa-plugin-gui  (cfg-selected runtime; no plugin names one)
plugin-gui-core ──► wayland-plugin-gui, cocoa-plugin-gui, resonance-plugin, every plugin (editor contract + widgets)
```

Hard rules — these are load-bearing for build times, testability, and cognitive load:

- `resonance-music-theory` **does not depend on audio, app, or plugin code**. It is pure music theory: pitch, scale, chord, progression, voicing, generators. It can be built and tested headless. Keep it that way.
- `resonance-audio` **does not depend on `resonance-app`**, and its one dependency the other direction on `resonance-music-theory` is narrow and sanctioned (doc #160, todo #358): the vocal-tuning render/bounce path (`engine/vocal_render.rs`) uses `resonance_music_theory::scale::{Mode, Scale}` for scale-snap, and `types::TuningScale` is the small, wire-stable enum the vocal-tuning data model and UI use, mapped to `Mode` in one place (`TuningScale::to_mode`). Don't widen this into a general audio/theory coupling — a new use needs its own doc/todo justification, not a ride on this one. The audio engine still doesn't know about Iced messages.
- `resonance-dsp`, `resonance-metering`, `resonance-common` are framework-agnostic — no Iced, no CLAP, no plugin trait. They're reusable building blocks.
- `resonance-mastering-assist` is the mastering assistant's analysis and decision engine (genre target bands, reference comparison, suggestions as param writes by key), a library crate: depends on `resonance-metering` and `resonance-dsp` only. The mastering plugin's panel and the app's `master.assist` both run it; the plugin owns the param lookup its keys resolve through, and its `tests/assistant_lockstep.rs` pins every key the engine emits to a real param.
- Plugins reach `resonance-common` **only for utilities** (`scan_directory`, `drum_map`, WAV decode, the content libraries `nam_library`, `drumkit_library` and `library_marks`, the CLAP extension ABIs `kit_info`, `param_flags` and `preset_session`, `factory_presets` via the SDK — the full list is `PLUGIN_COMMON_ITEMS` in `tools/arch-invariants`). DAW model types (takes, automation, MIDI maps, device definitions, freeze, track groups) are not plugin API. Pure-DSP helpers such as `flush_denormals` live in `resonance-dsp`, so most plugins do not depend on `resonance-common` at all; only amp, drums and ir do (ARCH-07).
- `resonance-svs` (singing-voice synthesis) depends only on `resonance-music-theory`. It renders DiffSinger `.ds` segments to audio headless, and ships its own CLI binary so the pipeline can be exercised without booting the app.
- `plugin-gui-core` is the platform-neutral half of the editor stack: the `EditorApp`/`EditorOptions`/`EditorError` contract, the fleet theme, and the pure egui widget set. No windowing code; builds on every OS.
- `wayland-plugin-gui` is the Linux editor runtime — it hosts an egui UI in its own Wayland window/thread, building on `plugin-gui-core` for the shared contract (and re-exporting it, so plugins have one import surface). No plugin depends on it directly: `resonance-plugin`'s `editor-widgets` feature pulls in the runtime for the current target and re-exports it as `editor_host::RuntimeEditor`, so a plugin's `editor` feature names only `plugin-gui-core` and `resonance-plugin/editor-widgets` (ARCH-08; `tools/arch-invariants` fails the build otherwise). It knows nothing about any specific plugin or the app. Its windowing body is Linux-only; other targets get a stub `Editor` so the workspace builds everywhere.
- `cocoa-plugin-gui` is the macOS editor runtime — same public `Editor` surface, inverted mechanics: the window lives on the AppKit main thread (which AppKit requires) and the `Send` handle dispatches onto it; rendering is NSOpenGLView + the same egui_glow painter. Windowing body macOS-only, stub elsewhere. Plugins reach whichever runtime matches the platform through `resonance_plugin::editor_host` (migration tracked in `macos-editor-plan.md` item 3c).
- `resonance-app` is allowed to depend on everything; it is the integration layer. The exception: it depends on no plugin crate. Plugins reach the app as CLAP bundles only; code both need goes into a library crate both depend on (as `resonance-mastering-assist` did for `master.assist`).

These rules are tests, not only prose: `tools/arch-invariants/tests/architecture.rs` reads `cargo metadata` and fails the suite on an internal dependency that is not an edge of the diagram, a plugin manifest or source that names a platform runtime, a plugin naming a `resonance_common` item outside the utility allow-list (or gaining the dependency unlisted), a GUI toolkit or windowing stack outside its crate, `resonance-app` depending on a plugin crate, an inline `#[cfg(test)]` beyond the documented exception, or a new top-level file in `resonance-app/tests/` (ARCH-10). Adding a crate means deciding its layer here and adding its row there.

When extending: add new building blocks to the lowest layer they fit, not the most convenient one. A new filter goes in `resonance-dsp`, not in the plugin that needs it first.

## Plugin Pattern

Every CLAP plugin in `plugins/` follows the same shape. Copy it for new plugins:

```
plugins/<name>/src/
├── lib.rs        ResonancePlugin impl + export_clap! macro invocation
├── params.rs     Params struct (FloatParam/IntParam/BoolParam fields)
├── dsp.rs        Pure DSP — no plugin trait, no UI, no allocs in process()
├── presets.rs    (optional) Built-in preset definitions
├── viz.rs        (optional) Lock-free viz state for the editor
└── editor/
    ├── mod.rs        EditorFactory + EditorApp impl, ui() entry
    ├── theme.rs      Plugin-local colours (do not import app theme)
    ├── controls.rs   Knob/button rows
    └── <feature>.rs  Per-panel views (curve, meters, scope, ...)
```

Discipline:

- `dsp.rs` is the pure-DSP boundary. It must be testable without the plugin framework. Plugins ship integration tests in `tests/` that drive `dsp.rs` directly.
- `params.rs` defines parameters as code, not as a serialized blob. Adding a parameter is a code change, not a config change.
- The editor is **feature-gated** (`default = ["editor"]`). Headless builds for tests/CI use `--no-default-features` and skip the egui and platform-runtime deps.
- `editor/theme.rs` is a one-line façade, not an independent palette: every one of the 13 plugins re-exports the shared design system with `pub use plugin_gui_core::theme::lavender::*` (the canonical tokens — see above), so all editors read as one product (ba todo #1338). A plugin may add a few local constants built *from* those shared tokens (e.g. an oscilloscope trace or a gain-reduction meter colour derived from `ACCENT`/`WARM`), and could in principle replace the façade to diverge — none currently do.

## Bumping the clack git pin (DEP-13)

`clack-plugin`/`clack-extensions`/`clack-host` are pinned to a git `rev`
in the root `Cargo.toml` (`[workspace.dependencies]`) rather than a
crates.io release — clack has none that cover the features this
codebase needs. That means there's no semver signal on a bump, and a
rewritten upstream history or an unreachable GitHub is a build break,
not a version conflict. `clap-sys` (`resonance-audio`'s own dependency
on the raw CLAP C headers) must stay in lockstep with whatever ABI
version clack's pin assumes — check clack's own `clap-sys` requirement
when bumping, not just its crate version.

Procedure for a bump:

1. Update the `rev` on all three `clack-*` entries together (they come
   from the same upstream commit; never let them drift).
2. `cargo update -p clack-plugin -p clack-extensions -p clack-host` to
   pull the new commit into `Cargo.lock`, then `cargo build --workspace`.
3. Run the `clap_host` test group (`./scripts/run-tests.py -p
   resonance-audio` covers it, or directly: the real-ABI tests in
   `resonance-audio/tests/clap_host/` and `resonance-plugin`'s own
   state/preset tests that drive a plugin through clack-host).
4. Run the editor lifecycle guard by hand from a live session (see
   CLAUDE.md's "Tests" section) — `editor_open` / `editor_size` on both
   the Wayland and Cocoa runtimes if you can reach both machines. A
   clack bump is exactly the kind of change that can silently shift the
   create → show → size → set_size → hide → drop sequence those tests
   pin down, and their failure mode is a hang, not a compile error.
5. Re-bundle (`scripts/bundle.sh`) and spot check a plugin's GUI opens
   in a real host, since clack mediates the whole CLAP ABI surface.

## Mastering as the Reference Decomposition

`plugins/resonance-mastering` is the model for how a non-trivial component should be decomposed. Use it as the template when a plugin or module grows past ~1500 lines:

```
src/
├── chain.rs          Top-level signal flow (orchestrator only)
├── stages/           One file per processing stage
│   ├── glue_compressor.rs
│   ├── multiband/    Subdir when a stage has internal structure
│   ├── linear_phase_eq/
│   └── ...
├── params/           One file per stage's parameter struct
│   ├── glue_compressor.rs
│   ├── multiband.rs
│   └── ...
├── assistant/        Independent feature in its own subdir (its engine,
│   ├── capture.rs    analysis and decisions, is the library crate
│   ├── decide.rs     resonance-mastering-assist; this is the plugin's half)
│   ├── reference.rs
│   └── state.rs
└── editor/
    ├── controls/     One file per stage's control panel
    └── <metric>.rs   Per-meter views (lufs_meter, tp_meter, ...)
```

Why this works: every file has one job; every directory has one theme; the depth never exceeds three. When a stage grows complex it gets a subdir of its own (`stages/multiband/`), not a 1000-line `stages.rs`.

## Update-Handler Pattern (resonance-app)

The app crate routes Iced messages through per-domain handlers. Each handler module exports `pub fn handle(r: &mut Resonance, msg: SpecificMessage) -> Task<Message>`:

```
resonance-app/src/update/
├── transport.rs   handle(r, TransportMessage)
├── track.rs       handle(r, TrackMessage)
├── clips.rs       handle(r, ClipMessage)
├── plugin.rs      handle(r, PluginMessage)
├── viewport.rs    handle(r, ViewportMessage)
└── ...
```

The Message enum is partitioned by domain (`TransportMessage`, `TrackMessage`, ...), and each top-level variant carries the right sub-message into the right handler. The `Resonance` impl `update()` method is just a dispatch, plus two non-domain helpers that live alongside the handlers: `update/gates.rs` (the pre-dispatch startup-modal and bounce-in-progress gates run on every message) and `update/tick.rs` (`handle_tick`, the periodic UI tick).

This pattern scales. New domains add a file; new messages within a domain add a match arm. **Keep new handlers in this shape** — do not add giant `impl Resonance` blocks with dozens of methods. (`engine_events.rs` and `project_io.rs` were the historical exceptions; both have since been split to match — engine events route through the free `handle_engine_event` in `engine_events/dispatch.rs`, project I/O through `update/project_io/`.)

## Audio Engine Public API

`resonance-audio` exposes one surface to the app: `AudioEngine` (commands in via `AudioCommand`, events out via `AudioEvent`). The entire `engine/` module is private. Internal types (`Track`, `Bus`, `MidiClip`, the mixer thread, the CLAP host) are not exposed.

The app reconstructs its own state from `AudioEvent`s. The engine never reaches up to mutate app state. This one-way flow is what lets the engine be tested without spinning up Iced, and what lets the app be reasoned about without thinking about the audio thread.

When adding engine functionality:
1. New `AudioCommand` variant for the input.
2. Handler on the engine thread that mutates engine state and emits...
3. New `AudioEvent` variant carrying the result.
4. App-side handler in `engine_events/` (per-domain file, routed via `dispatch.rs`) that mirrors the change in app state.

Do not add direct getter methods on `AudioEngine` that read engine state — that creates synchronization headaches and undermines the command/event boundary.

## Test Layout

Tests live in `<crate>/tests/`, not in `#[cfg(test)] mod tests` blocks inside source files. This:
- keeps source files focused on production code
- forces tests to use the public API
- avoids ballooning the largest source files further

When a file would otherwise need a test module, add a sibling `tests/<feature>.rs` integration test instead.

### `resonance-app` private-helper exception

`resonance-app` is a library crate with a thin binary shim: the modules live
under `lib.rs`, and `main.rs` only parses CLI args and wires up the iced
runtime. That split exists so the integration tests under
`resonance-app/tests/` can drive the real `view()` / `update()` paths via
`iced_test` — new app tests belong there. A handful of inline
`#[cfg(test)] mod tests` blocks nevertheless remain in `resonance-app/src/`,
because their subjects are private helpers that would leak implementation
details if promoted to `pub`:

- `recent.rs` — exercises private `insert_pure`, `derive_display_name`, and `MAX_RECENT`.
- `compose/invariants.rs`, `compose/tests.rs` — section/chord state round-trips that read crate-internal types.

These are the documented exception, not the rule. Do not add new inline tests
elsewhere in the workspace (shrink this list when you can, don't grow it) — for
example, the `update/project_io/replay.rs` and `replay_diff.rs` helpers this
list used to carry an exception for (`migrate_auto_name`,
`sort_plugins_by_saved_order`, `structurally_compatible`, `id_set_eq`,
`midi_notes_equal`) are `pub fn` now, with their inline tests migrated to
`resonance-app/tests/io/replay.rs` and `replay_diff.rs`; the `undo.rs` inline
tests likewise moved out, to `tests/timeline/undo_history.rs`. If you need to
test a private helper in any other crate, make the helper `pub(crate)` and
write a `tests/<feature>.rs` integration test in that crate instead.

## Anti-Patterns to Avoid

Things that have caused pain and that future code should not repeat:

- **`mod.rs` as a dumping ground.** A `mod.rs` that grows past ~200 lines is a sign the directory wants to be a real module, not a single-file wrapper. `mod.rs` should re-export and dispatch, not house types.
- **Giant `impl Resonance` blocks.** When one file has 855 lines of methods on the central app struct, you have a god object disguised as a module. Per-domain handler files (see *Update-Handler Pattern*) are the answer.
- **One view function per screen.** A 1700-line `view()` is unmaintainable. Split by sub-region (one file per panel/strip/lane type), not by widget primitive.
- **Mixing concerns in I/O code.** File I/O, struct construction, state mutation, and engine command dispatch should be in separate functions and ideally separate files. When they're interleaved, you can't test serialization without a running engine.
- **Pub fields with hidden invariants.** Most state structs are intentionally `pub`-everywhere for Iced ergonomics. That's fine — but if a field has a non-trivial invariant (e.g., "loop_in < loop_out", "sub-track count matches plugin output count"), wrap *that specific field* behind a method. Don't pretend everything else needs encapsulating, and don't pretend the invariant doesn't exist.
- **Inline control-rate logic on the audio thread.** Click envelope synthesis, loop-seam stitching, and master peak metering in one mix function makes audio bugs and metronome bugs entangled. Extract each concern to its own helper.
