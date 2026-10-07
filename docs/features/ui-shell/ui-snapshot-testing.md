# UI snapshot (golden PNG) testing

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: ba doc #285 §4-T7 (bless flag); memory notes "goldens: bless
> locally, this machine is canonical".

UI changes are checked against committed PNGs. A test builds a hermetic app
(`Resonance::new_for_test*`), seeds state, renders `app.view()` through
`iced_test::Simulator` at a fixed size with the bundled fonts, and compares
the pixels with `resonance-app/tests/snapshots/<name>-wgpu.png`. Type
checking can't catch alignment, overlap or spacing problems; these tests
can.

## Behaviour

**The helper** (`resonance-app/tests/common/mod.rs::assert_golden(snapshot,
path)`)

- `path` is crate-relative, for example `tests/snapshots/foo.png`. iced_test
  always writes a backend-suffixed file: `foo-wgpu.png`, or
  `foo-tiny-skia.png` when wgpu can't start.
- **Normal run:** if no golden exists for either backend, the test fails
  ("no golden on disk … Bless it deliberately"). This stops
  `matches_image` from silently creating one and passing for ever.
  Otherwise any pixel difference fails ("snapshot diverged from golden").
- **`RESONANCE_BLESS=1`:** deletes both backend variants and re-renders the
  golden in place.
- **`RESONANCE_SKIP_GOLDENS=1`:** still renders, so the view code runs, but
  skips the comparison and logs a note. It exists for non-conformant
  software renderers.

**Writing one.** Every golden test follows the same pattern:

- Build the app with `new_for_test()` / `new_for_test_on(tab)`.
- Build `iced::Settings` with `ICON_FONT_BYTES` plus every `UI_FONT_FACES`
  face and `default_font: UI_FONT`. Each test file has a small
  `sim_settings()` for this.
- Create `Simulator::with_size(settings, Size, app.view())` and take
  `snapshot(&theme::resonance_theme())`.
- Full-window goldens use 1440×900, the app's minimum size. Component
  goldens use smaller sizes.

There are 75 test modules that call `assert_golden`, and 157 PNGs.

**Canonical machine.** There is no CI. The developer's machine is canonical
for goldens. A divergence there is a real regression or an intended change
to re-bless, never "font noise". After touching the render path, run the
suite with `RESONANCE_RENDER_THREADS=8` as well. Goldens must be identical
at any render-thread count (see CLAUDE.md).

**Silent goldens.** A golden of an empty or silent state matches for ever.
Pin each scenario's inputs so the image shows the thing under test.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Helper | `resonance-app/tests/common/mod.rs` | `assert_golden`, `should_skip_goldens` |
| Goldens | `resonance-app/tests/snapshots/*.png` | `<name>-wgpu.png` |
| Hermetic app | `resonance-app/src/lib.rs`, `src/user_dirs.rs`, `src/test_support/` | `new_for_test`, `new_for_test_on`, `new_for_test_with_capture`, `enter_hermetic` |
| Test layout rule | `tools/arch-invariants/tests/architecture.rs` | `app_test_binaries_are_the_known_groups`, `test_binary_counts_only_grow_deliberately` |

## Surfaces

- **By hand:**
  `RESONANCE_BLESS=1 cargo test -p resonance-app --test <group>` re-blesses
  a group's goldens (groups: compose, control, io, midi, mixer, performance,
  plugins, timeline, vocal). Look at the changed PNGs before committing.
- **Control API / MCP:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| All goldens | `resonance-app/tests/<group>/*.rs` | `./scripts/run-tests.py -p resonance-app` |
| Bless | the same tests | `RESONANCE_BLESS=1 cargo test -p resonance-app --test <group>` |

Not covered: plugin editors (egui) have no golden PNGs in this harness.
Their editor lifecycle is checked by the `#[ignore]`d Wayland/Cocoa tests
listed in CLAUDE.md.

## Decisions and deviations

- New golden tests go in an existing group binary as a module, not as a new
  file under `resonance-app/tests/`. An arch-invariants test enforces this.
- The old live `--demo` screenshot loop is gone. The binary has no `--demo`
  flag, so goldens are the only visual check an agent can run headless.

## Known gaps

- Several surfaces have no golden:
  - the startup screen on its own;
  - the quit and delete-track confirms;
  - the missing-plugins dialog;
  - the keymap conflict banner;
  - Performance count-in;
  - the Media › Presets tab.
