# Plugin bundling and adding a plugin crate

> Verified against master @ `ef215b08` on 2026-10-15.
> Spec history: ba todos #1250 and #1073 (in the script's own comments);
> macOS bundle layout from `docs/history/macos/macos-editor-plan.md`
> item 3e.

`scripts/bundle.sh` builds every first-party plugin in release mode and
publishes it as a `.clap` in `target/bundled/`, which the app scans from a
checkout. This doc also lists what a new plugin crate has to touch, because
some of it is derived and some is not.

## Behaviour

- **Derived plugin list.** The script takes every directory under
  `plugins/` that has a `Cargo.toml` and cross-checks it both ways against
  the workspace members under `plugins/` (read from
  `cargo metadata --no-deps --offline`). It fails if a crate is not a
  member, if a member is nested deeper than `plugins/<name>/`, if cargo
  metadata cannot be read, or if a crate is not a `cdylib`.
- **All or nothing.** One `cargo build --release -p …` for all plugins,
  then staging into `target/bundled.new` and a swap at the end. A failed
  build or copy leaves the previous bundle intact, and a renamed or deleted
  plugin's old `.clap` disappears.
- **Platform shape.** Linux: `lib<name>.so` copied to `<name>.clap`. macOS
  (`uname -s` = Darwin): a bundle directory
  `<name>.clap/Contents/MacOS/<name>` plus a minimal `Info.plist`
  (`CFBundleExecutable`, `CFBundleIdentifier com.resonance.<name>`,
  `CFBundlePackageType BNDL`). The script runs on macOS's stock bash 3.2 (no
  `mapfile`).
- **Tests do not need it.** `scripts/run-tests.py` builds the plugin
  cdylibs that `resonance-audio`'s `clap_host` tests load (it finds them by
  `plugin_binary("<crate>")` calls), so the suite does not depend on a
  bundle run.

## Adding a plugin crate

Registration for bundling is one step: create `plugins/<name>/` (crate
type `["cdylib", "lib"]`) and add it to the workspace `members`. Several
checks then require more, and fail until done:

1. Add its CLAP id to `resonance-plugin/src/first_party.rs` (a constant and
   an entry in `ALL`) and declare `const CLAP_ID = first_party::<NAME>` in
   the plugin. `tools/arch-invariants` rejects the id spelled as a literal
   anywhere else.
2. If the crate has a `tests/` folder, add a row to `TEST_BINARY_CAPS` in
   `tools/arch-invariants/tests/architecture.rs` (a crate with test files
   and no row fails).
3. Depend only on allowed crates (the plugin row of the layering rules in
   ARCHITECTURE.md; `resonance-common` items only from
   `PLUGIN_COMMON_ITEMS`).
4. Add it to `ALL_PLUGINS` and `FLEET` in
   `resonance-plugin/tests/fleet_preset_adoption.rs` (instruments also to
   `INSTRUMENT_PLUGINS`) and to `FLEET` in `tests/fleet_palette.rs`, with
   the knob family its editor draws. Both `FLEET`s pin a count of 13; the
   palette one is also compared with every `plugins/*` crate that has an
   `src/editor/theme.rs`, and the preset one with `ALL_PLUGINS`, so a new
   editor crate fails until it is listed.
5. If a skill names its params, add a `skill_keys` test
   (`resonance_dsp_test_support::skill_keys_test!`).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Bundling | `scripts/bundle.sh` | `members_under_plugins`, `dirs_under_plugins`, `write_info_plist` |
| Same rule as a test | `tools/arch-invariants/tests/architecture.rs` | the "plugins/<name>/ ⇔ workspace member, and every one a cdylib" check |
| Test-time plugin builds | `scripts/run-tests.py` | `plugin_cdylibs`, `build_plugin_cdylibs` |
| Where the app looks | `resonance-audio/src/engine/scan.rs`; macOS bundle lookup in `resonance-audio/src/clap_host/bundle.rs` (`bundle_binary_path`) | |

## Surfaces

- **GUI:** none; after bundling, Settings → Plugins → Rescan picks up new
  bundles without a restart.
- **Control API / MCP:** `plugins.rescan` → `mcp__resonance__plugins_rescan`.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Directory ⇔ member ⇔ cdylib rule | `tools/arch-invariants/tests/architecture.rs` | `cargo test -p arch-invariants` |
| Id spelling, test-binary caps | same file | same |

Not covered: the script itself has no test; macOS output was checked by hand
when item 3e landed.

## Decisions and deviations

- The literal plugin list and the per-plugin `bundler.toml` files are gone:
  a plugin that was a member and had a `bundler.toml` was still left out
  because only the script's literal list was wrong.
- `bundle.sh` produces release builds only, and does not install into
  `~/.clap` or `~/Library/Audio/Plug-Ins/CLAP`.

## Known gaps

- The fleet test lists in `resonance-plugin/tests/fleet_*.rs` are
  hand-maintained and miss color and stereo.
