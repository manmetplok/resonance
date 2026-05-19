# Architecture

Patterns this codebase has gotten right. Imitate these when adding new code; resist drifting away from them.

## Crate Layering

The workspace is a deliberate DAG. Every crate has a single responsibility, and the lower layers know nothing about the upper layers.

```
resonance-dsp ──┬─► resonance-metering ──► resonance-mastering plugin
                ├─► resonance-audio ─────► resonance-app
                └─► (every FX plugin)
resonance-music-theory ──► resonance-app  (pure theory, no audio/app deps)
resonance-common ──► resonance-audio, every plugin
resonance-plugin ──► every plugin, resonance-app (UI helpers)
```

Hard rules — these are load-bearing for build times, testability, and cognitive load:

- `resonance-music-theory` **does not depend on audio, app, or plugin code**. It is pure music theory: pitch, scale, chord, progression, voicing, generators. It can be built and tested headless. Keep it that way.
- `resonance-audio` **does not depend on `resonance-music-theory` or `resonance-app`**. The audio engine doesn't know about chords or about Iced messages. If you find yourself wanting to add a music-theory dep to `resonance-audio`, the work belongs in the app layer instead.
- `resonance-dsp`, `resonance-metering`, `resonance-common` are framework-agnostic — no Iced, no CLAP, no plugin trait. They're reusable building blocks.
- `resonance-app` is allowed to depend on everything; it is the integration layer.

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
- The editor is **feature-gated** (`default = ["editor"]`). Headless builds for tests/CI use `--no-default-features` and skip the egui/wayland deps.
- `editor/theme.rs` is plugin-local. Plugins do not share a theme module — each plugin has its own visual identity.

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
├── assistant/        Independent feature in its own subdir
│   ├── analyze.rs
│   ├── decide.rs
│   └── reference.rs
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

The Message enum is partitioned by domain (`TransportMessage`, `TrackMessage`, ...), and each top-level variant carries the right sub-message into the right handler. The `Resonance` impl `update()` method is just a dispatch.

This pattern scales. New domains add a file; new messages within a domain add a match arm. **Keep new handlers in this shape** — do not add giant `impl Resonance` blocks with dozens of methods. (`engine_events.rs` and `project_io.rs` are the historical exceptions; they should be split to match.)

## Audio Engine Public API

`resonance-audio` exposes one surface to the app: `AudioEngine` (commands in via `AudioCommand`, events out via `AudioEvent`). The entire `engine/` module is private. Internal types (`Track`, `Bus`, `MidiClip`, the mixer thread, the CLAP host) are not exposed.

The app reconstructs its own state from `AudioEvent`s. The engine never reaches up to mutate app state. This one-way flow is what lets the engine be tested without spinning up Iced, and what lets the app be reasoned about without thinking about the audio thread.

When adding engine functionality:
1. New `AudioCommand` variant for the input.
2. Handler on the engine thread that mutates engine state and emits...
3. New `AudioEvent` variant carrying the result.
4. App-side handler in `engine_events.rs` (or its successor split) that mirrors the change in app state.

Do not add direct getter methods on `AudioEngine` that read engine state — that creates synchronization headaches and undermines the command/event boundary.

## Test Layout

Tests live in `<crate>/tests/`, not in `#[cfg(test)] mod tests` blocks inside source files. This:
- keeps source files focused on production code
- forces tests to use the public API
- avoids ballooning the largest source files further

When a file would otherwise need a test module, add a sibling `tests/<feature>.rs` integration test instead.

### Binary-crate exception

`resonance-app` is a binary crate (`main.rs`, no `lib.rs`). Integration tests
under `tests/` cannot see any of its types — they are inaccessible from outside
the binary. A handful of inline `#[cfg(test)] mod tests` blocks therefore remain
in `resonance-app/src/`:

- `recent.rs` — exercises private `insert_pure`, `derive_display_name`, and `MAX_RECENT`.
- `undo.rs` — exercises private fields (`UndoHistory::capacity`, `undo`, `redo`) for coalescing/capacity invariants.
- `compose/invariants.rs`, `compose/tests.rs` — section/chord state round-trips that read crate-internal types.
- `update/project_io/replay.rs` — exercises the private `migrate_auto_name` helper.

These are the documented exception, not the rule. Do not add new inline tests
elsewhere in the workspace. If you need to test a private helper outside
`resonance-app`, make the helper `pub(crate)` and write a `tests/<feature>.rs`
integration test in that crate instead. Promoting `resonance-app` to a library
crate purely to migrate these tests is **not** worth the visibility audit and
re-export churn it would entail.

## Anti-Patterns to Avoid

Things that have caused pain and that future code should not repeat:

- **`mod.rs` as a dumping ground.** A `mod.rs` that grows past ~200 lines is a sign the directory wants to be a real module, not a single-file wrapper. `mod.rs` should re-export and dispatch, not house types.
- **Giant `impl Resonance` blocks.** When one file has 855 lines of methods on the central app struct, you have a god object disguised as a module. Per-domain handler files (see *Update-Handler Pattern*) are the answer.
- **One view function per screen.** A 1700-line `view()` is unmaintainable. Split by sub-region (one file per panel/strip/lane type), not by widget primitive.
- **Mixing concerns in I/O code.** File I/O, struct construction, state mutation, and engine command dispatch should be in separate functions and ideally separate files. When they're interleaved, you can't test serialization without a running engine.
- **Pub fields with hidden invariants.** Most state structs are intentionally `pub`-everywhere for Iced ergonomics. That's fine — but if a field has a non-trivial invariant (e.g., "loop_in < loop_out", "sub-track count matches plugin output count"), wrap *that specific field* behind a method. Don't pretend everything else needs encapsulating, and don't pretend the invariant doesn't exist.
- **Inline control-rate logic on the audio thread.** Click envelope synthesis, loop-seam stitching, and master peak metering in one mix function makes audio bugs and metronome bugs entangled. Extract each concern to its own helper.
