# Hermetic test app

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: ba doc #285 / todo #1365 (not in the repo);
> `docs/history/reviews/code-review-todo.md` (STATE-14, FU-D5, FU-M12b);
> test layout in `docs/history/refactor/arch-test-grouping.md`.

Integration tests build a full `Resonance` — real `update`, real `view`, real
state — with nothing from the host machine attached: no audio device, no
engine thread, no installed plugins, no reads or writes of the user's config.
A test's result then depends only on the test.

## Behaviour

- **Constructors** (behind the `test-support` cargo feature, which
  `resonance-app`'s own dev-dependency turns on):
  - `Resonance::new_for_test()` — command-capturing engine whose commands
    are drained and dropped by a background thread. Sends are *accepted*, so
    the app takes the same branches as with a live engine.
  - `new_for_test_on(tab)` — the same, starting on `tab`. Use this instead of
    the binary's process-wide `STARTUP_TAB` `OnceLock`, which only the first
    writer in a process can set.
  - `new_for_test_with_capture()` — also returns the
    `crossbeam_channel::Receiver<AudioCommand>`, so a test can assert on the
    exact commands an action sent. `test_capture_engine()` swaps in a fresh
    channel mid-test.
  - `new_for_test_disconnected()` — every send hits the "engine is gone"
    branch.
- **What a test app is not given:** construction sends no engine commands at
  all — no device lists, no `ScanPlugins`, no startup "Track 1", no clip-id
  grant, no crashed-session offer. Recents, settings and user presets start
  empty or default. Compiled-in device definitions are still loaded.
- **The process becomes hermetic.** `assemble` calls
  `user_dirs::enter_hermetic()`, which points `user_dirs::config_dir()` and
  `data_dir()` at `$TMPDIR/resonance-hermetic-<pid>/` for the rest of the
  process. Recents, settings, presets, templates and autosave scratch dirs
  written by a test land there, never in `~/.config` or `~/.local/share`.
- **No engine echoes.** The capturing engine processes nothing, so a test
  that needs the round trip feeds `AudioEvent`s back itself
  (`test_apply_engine_event`, `test_handle_engine_event`).
- **Accessors.** `src/test_support/*.rs` adds about 300 `test_*` methods on
  `Resonance` (read state, set active project, set project path, build the
  project file, snapshot for undo, replay a loaded project, …), split by
  domain. `demo.rs` seeds fixture projects (`seed_demo_content` and friends).
  Shared helpers for test binaries are in `resonance-app/tests/common/mod.rs`
  (`app_bare`, `app_active_project`, `app_with_tempo_120`, `assert_golden`,
  `call` for control requests).
- **Undo in tests** needs an active project and an anchor dir; a bare
  `test_set_active_project(true)` app with no path records no history.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Constructors | `resonance-app/src/lib.rs` | `new_for_test`, `new_for_test_on`, `new_for_test_with_capture`, `new_for_test_disconnected`, `assemble`, `Host::{Machine, None}` |
| Hermetic dirs | `resonance-app/src/user_dirs.rs` | `enter_hermetic`, `hermetic_root`, `config_dir`, `data_dir` |
| Test engines | `resonance-audio/src/engine/audio_engine.rs` | `AudioEngine::for_test_capture`, `for_test_disconnected` |
| Accessors | `resonance-app/src/test_support/{mod,project,tracks,timeline,transport,mixer_plugins,chain_ui,pool_media,shortcuts,vocal_compose}.rs` | `test_*` |
| Fixtures | `resonance-app/src/demo.rs` | `seed_demo_content`, `seed_pool_assets`, … |
| Shared test helpers | `resonance-app/tests/common/mod.rs` | `app_bare`, `app_active_project`, `assert_golden`, `should_skip_goldens` |
| Feature gate | `resonance-app/Cargo.toml` | `test-support = ["resonance-audio/test-internals"]` |

Rules (from `CLAUDE.md`, enforced where noted): build test apps with
`new_for_test*`, never `Resonance::new()`; add tests as modules of an
existing group binary (`arch-invariants::app_test_binaries_are_the_known_groups`);
no inline `#[cfg(test)]` outside the documented exceptions
(`no_inline_test_modules_outside_the_documented_exception`).

## Surfaces

- Tests only. Run with `./scripts/run-tests.py -p resonance-app`; re-bless
  goldens with `RESONANCE_BLESS=1`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Construction emits nothing, reads no config, keeps device defs | `resonance-app/tests/hermetic_construction.rs` | `./scripts/run-tests.py -p resonance-app` |
| Saves/loads/presets write under the temp root | `resonance-app/tests/io/hermetic_user_state.rs` | same |
| Startup track (invoked by hand on a hermetic app) | `resonance-app/tests/io/startup_default_track.rs` | same |

Not covered: nothing checks that no test calls `Resonance::new()`; that rule
is prose in `CLAUDE.md`. One test breaks it on purpose:
`resonance-app/tests/e2e_compose_via_control.rs` boots the real engine to
assert an offline bounce really writes a WAV, so it depends on the machine's
audio device and installed plugins.

## Decisions and deviations

- The hermetic engine accepts commands rather than refusing them, because
  the app has real error paths behind a failed send that tests must not take
  by accident.
- `hermetic_construction.rs` is its own top-level test binary (allow-listed
  with `e2e_compose_via_control.rs` in `arch-invariants`), beside the nine
  group binaries.

## Known gaps

- The module doc of `src/test_support/mod.rs` still says it is "not behind
  any feature gate"; `lib.rs` gates it on `feature = "test-support"`.
