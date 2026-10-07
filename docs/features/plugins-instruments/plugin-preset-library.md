# Plugin preset library

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/plugin-preset-library.md`. Its status
> line ("round 1 built on `feat/plugin-presets`") is stale: round 2 (P2–P8,
> its review round and verification pass) is on master. P9 is partly built,
> P10 is not.

One preset system for every plugin, first-party and third-party. Each preset
is a JSON file with metadata (category, instrument, genres, character,
tags). Users browse, audition, favourite, tag, save, rename, duplicate,
delete, import and export presets from a preset bar in every first-party
editor and from the app (plugin panel bar, preset browser overlay, media
browser Presets tab, "with preset…" add pickers, drag to a track). Agents
search, load, save and mark presets over the control API. Drum kits and amp
models share the same browser model and marks.

## Behaviour

- **File format** (`resonance-plugin/src/presets/format.rs`):
  `"format": "resonance.preset"`, `format_version: 1`, a `meta` block (name,
  author, description, category, instrument[], genres[], character[],
  tags[], created, modified, derived_from) and a `state` with encoding
  `resonance-json` (first-party params doc) or `clap-state` (an opaque CLAP
  state blob, with its `form`, for third-party plugins). Older files are
  converted once by `presets/migrate.rs`; originals are kept with a
  `.legacy` suffix.
- **User presets** live in `$XDG_DATA_HOME/resonance/plugin-presets/<clap id>/<name>-<id8>.json`
  (override `RESONANCE_PLUGIN_PRESET_DIR`, or the app setting
  `presets.plugin_preset_root`). Deleting moves a file to
  `<root>/.trash/<clap id>/` for 30 days. Identity is `(source, id)`; the
  name is only a display hint.
- **Factory presets** are format-1 files under `plugins/<p>/presets/`,
  `include_str!`'d into the plugin and exposed through `FACTORY_PRESETS` and
  the exported `resonance_factory_presets` symbol (see
  [plugin-sdk.md](plugin-sdk.md)). 105 in total: wavetable 28, reverb 20,
  compressor 11, EQ 11, gate 9, granular delay 8, stereo 7, delay 6,
  color 5. Amp, drums, IR and mastering ship none. Factory ids are never
  reused or silently dropped.
- **Whole-sound presets.** Plugins with state outside params (amp model,
  drums kit and mics, IR file, wavetable user tables) put it in the preset
  through `ExtraStateSaver::preset_keys`; params that should not travel
  (amp `file_select`, drums `output_mode` and per-pad output) are
  `excluded_from_presets`. A preset is laid over the current state
  (`overlay_preset`).
- **Loaded identity and "modified".** `PresetSession` persists the loaded
  preset with the project and compares against a baseline (params plus an
  extra-state hash) at most every 100 ms on the main thread; automated
  params are left out of the compare. It reports `{source, id, name,
  modified}` to the host through `com.resonance.preset-session`.
- **Third-party presets.** User presets of third-party plugins are stored as
  `clap-state` blobs (saved via `clap.state-context` `FOR_PRESET`). Presets
  a third-party plugin publishes through CLAP `preset-discovery` are
  indexed on a worker and cached per bundle in
  `<cache>/preset-discovery/<bundle>.json`, and loaded with
  `clap.preset-load`.
- **Audition and undo.** Clicking a row auditions without recording; Load
  or keep records one undo step. Loads carry a capture token so the engine
  saves the full state before and after: Esc/Revert restores the whole
  sound, including extra state, and undo/redo of a preset load are exact.
  ◀/▶ runs coalesce into one undo entry.
- **Search** is one engine for every surface (`library_view::parse_search`):
  substring tokens ANDed, plus `is:fav|recent|user|factory`, `tag:`, `by:`,
  `genre:`, `cat:`, `for:`, `char:`. Scoped tokens match exactly or by slug
  prefix. Facets (category, instrument, genre, character) and sorts.
- **Vocabulary** is seeded, not closed
  (`resonance-common/src/library_marks/vocab.rs`): instrument and effect
  category lists, instrument, genre and character values; other values are
  slugged and kept.
- **Marks** (favourite, personal tags, last used, use count) live in the
  shared marks file under `plugin-preset:<clap id>:<preset id>`, next to
  `amp-model` and `drumkit` marks. There is no "hidden" mark; `rating` is
  reserved and unused.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Format, files, migration | `resonance-plugin/src/presets/{format,files,migrate}.rs` | `PresetFile`, `PresetMeta`, `FORMAT_VERSION` |
| Library (only writer) | `resonance-plugin/src/presets/library.rs` | `PresetLibrary`, `SaveRequest`, `FactoryEntry` |
| One plugin's view | `presets/{bank,editor,session}.rs` | `PresetBank`, `PresetEditor`, `PresetSession`, `SoundSnapshot` |
| Query + rows + marks | `presets/{query,rows,marks}.rs` | `Query`, `PresetRows`, `MarksSource` |
| Browser model (GUI-agnostic) | `resonance-plugin/src/library_view.rs` | `LibraryRows`, `BrowserModel`, `parse_search`, `AuditionBracket` |
| Plugin-editor UI | `presets/browser.rs`, `resonance-plugin/src/preset_ui.rs`, `library_ui.rs` | `PresetBrowser`, `preset_bar`, `library_list` |
| CLAP plumbing | `resonance-plugin/src/clap_bridge/{preset,preset_session}.rs`; ABIs `resonance-common/src/{preset_session,factory_presets}.rs` | `com.resonance.preset-session/1` |
| Marks + vocab | `resonance-common/src/library_marks.rs`, `library_marks/vocab.rs` | `Marks`, `kind::PLUGIN_PRESET` |
| Host: library | `resonance-app/src/plugin_preset_library.rs`, `state/presets.rs` | one `PresetLibrary` per app |
| Host: UI | `resonance-app/src/update/plugin_preset_ui.rs`, `view/preset_browser.rs`, `view/browser/mod.rs` (Presets tab), `view/mixer/inspector/{chain,bus,master}.rs` | `Overlay::PresetBrowser`, `PresetDrag` |
| Host: engine | `resonance-audio/src/clap_host/{preset_state,discovery}.rs`, `engine/scan.rs` | `PluginPresetsDiscovered` |
| Control handlers | `resonance-app/src/update/control/{plugin_presets,presets}.rs` | |

## Surfaces

- **GUI:** preset bar in every first-party editor (◀ ☆ picker ▶ Browse,
  Save, Save as…) with a browser overlay and metadata form; in the app, a
  bar in the plugin panel header (◀ name ▶ ★ Presets…), the preset browser
  overlay, the media browser Presets tab, "▸ with preset…" in the add
  pickers, drag a preset onto a track header; palette commands
  `PreviousPluginPreset`, `NextPluginPreset`, `BrowsePluginPresets`.
- **Control API / MCP:**
  - `presets.search` → `mcp__resonance__presets_search`,
    `presets.rename` → `presets_rename`, `presets.delete` → `presets_delete`,
    `presets.set_marks` → `presets_set_marks`, `presets.update_meta` →
    `presets_update_meta`, `presets.vocabulary` → `presets_vocabulary`.
    These are library edits: no undo entry, no revision bump.
  - Per slot: `track.plugin_presets` / `track.load_plugin_preset` /
    `track.save_plugin_preset` (and the `bus.*` and `master.*` twins) →
    `track_plugin_presets`, `track_load_plugin_preset`,
    `track_save_plugin_preset`, `bus_…`, `master_…`. Lists report
    `current`, `modified`, `modified_known`, `total`, `facets`.
  - A `preset` argument (id or name) on `track_add_effect`,
    `track_add_instrument`, `bus_add_effect`, `master_add_effect`.
  - Track presets (`track_presets`, `track_save_preset`,
    `track_apply_preset`) are a separate system (whole track chains), not
    this library.
- **Agent skills:** `mixing` and `spatial` in
  `resonance-agent-plugin/skills/` use `presets_search` and friends; skill
  keys blocks may name factory presets, checked by `skill_keys` tests.

## How it's tested

| What | Where | Run |
|---|---|---|
| Format, bank, library, session | `resonance-plugin/tests/{presets,preset_library}.rs` | `./scripts/run-tests.py -p resonance-plugin` |
| Browser model + egui smoke renders | `resonance-plugin/tests/{library_view,library_ui_render,preset_bar_render,preset_bar_rescan,kit_rows}.rs` | same |
| Fleet rules (every factory file complete, ids pinned, session adopted) | `resonance-plugin/tests/fleet_preset_adoption.rs` | same |
| Per-plugin factory banks | e.g. `plugins/resonance-wavetable/tests/presets.rs`, `plugins/*/tests/skill_keys.rs` | per plugin |
| Host engine | `resonance-audio/tests/clap_host/{preset_discovery,plugin_preset_state,clap_factory_presets}.rs` | `./scripts/run-tests.py -p resonance-audio` |
| App + control | `resonance-app/tests/control/{control_plugin_presets,control_presets_library}.rs`, `resonance-app/tests/mixer/mixer_preset_bar.rs`, `resonance-app/tests/io/preset_tab_widgets.rs` | `./scripts/run-tests.py -p resonance-app` |
| MCP lockstep | `resonance-mcp/tests/agent_plugin_lockstep.rs` | `./scripts/run-tests.py -p resonance-mcp` |

Not covered: no golden images for the preset bar or browser (the render
tests are headless egui smoke tests). `fleet_preset_adoption.rs` lists 11
plugins and does not check color or stereo for session adoption.

## Decisions and deviations

- D1–D11 were never answered; the build follows every recommended option.
- Factory presets reach the host through a custom exported symbol, not by
  publishing CLAP `preset-discovery` (P10 not built).
- First-party vs third-party is decided by provenance (the instance serves
  `com.resonance.preset-session`), not by the content.
- The spec's §18 module list names `vocab` and `fs`; the code has
  `presets/files.rs` and `presets/rows.rs`, and the vocabulary lives in
  `resonance-common`.

## Known gaps

- `.rpreset` multi-preset export bundles, discovered-preset soundpacks and
  user-content flags, IR and track-preset kinds in the shared library, and
  publishing our presets to other hosts:
  [preset-library-remaining](../../intents/plugins-instruments/preset-library-remaining.md).
