# Third-party plugins (the user's view)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/intents/prototypes/third-party-plugin-hosting/`
> (epic #45 prototype). The CLAP host internals (bundle loading, instance
> lifecycle, latency, state) are documented under
> [clap-hosting.md](../audio-engine/clap-hosting.md), [clap-params-and-presets.md](../audio-engine/clap-params-and-presets.md) and [latency-compensation.md](../audio-engine/latency-compensation.md).

Resonance hosts any CLAP plugin it finds on disk, next to its own. Users see
them in the add-instrument and add-effect pickers, get a generic parameter
panel for every plugin plus the plugin's own editor window when it offers
one, and keep a project's plugin settings even when a plugin is not
installed. Agents see the same catalog and can rescan.

## Behaviour

- **Where plugins come from** (`resonance-audio/src/engine/scan.rs`):
  Linux `~/.clap` and `/usr/lib/clap`; macOS `~/Library/Audio/Plug-Ins/CLAP`
  and `/Library/Audio/Plug-Ins/CLAP`; then every `$CLAP_PATH` entry; then
  `target/bundled/` next to the executable and the working directory (the
  first-party bundles of a checkout). Subfolders are searched up to depth 4,
  symlinks are resolved and each bundle loads once. Duplicate plugin ids are
  reported as warnings.
- **No catalog cache.** Every start loads every bundle in-process. Only CLAP
  preset discovery is cached (see
  [plugin-preset-library.md](plugin-preset-library.md)).
- **Rescan is additive.** "Rescan Plugins" (Settings) and `plugins.rescan`
  open only bundles not yet loaded; loaded ones, their instances and
  editors are untouched. An uninstalled plugin stays in the catalog until
  the next start, because unloading a library a running instance came from
  would crash the audio thread. Earlier scan failures are cleared when a
  rescan starts.
- **Settings → Plugins** shows a one-line summary (plugin, instrument and
  effect counts), the Rescan button ("Scanning..." while running) and one
  red line per failed bundle with the loader's reason. There is no folder
  list, no add-folder setting, no catalog table and no cancel.
- **Adding a plugin** uses the inspector's "+ Add instrument" / "+ Add to
  chain" pick lists, "▸ with preset…", or the command palette `+ plugin`
  mode. Bundled and third-party plugins are listed together with no source
  or vendor facets.
- **Plugin window.** Every hosted plugin gets an in-app window with a preset
  bar and a generic parameter list. "Open Editor" opens the plugin's own GUI
  as a floating window; on Linux only the Wayland window API is offered, so
  an X11-only plugin falls back to the generic panel. Each refusal is
  reported and the generic panel stays usable.
- **Missing plugins.** A slot whose plugin cannot be loaded stays in the
  chain as `PluginAvailability::Missing { reason }`. Its state blob and
  param values are kept and written back on save, autosave and templates,
  byte for byte. The user sees a load-time dialog listing every missing
  plugin and where it sits ("Continue anyway", Rescan; reopenable from
  Settings), a pill on the mixer strip, a red recovery box under the chain
  row ("Replace with…", Rescan, Remove) and a recovery body in the plugin
  window. Replacing keeps the slot's instance id and position. When the
  plugin reappears, its settings come back.
- **Latency** of hosted plugins is compensated, but not shown to the user.
- **Crashes.** Plugins run in the app's process with no sandbox, scan
  timeout or blocklist. FFI inputs are hardened, state calls catch Rust
  panics, and libraries are never unloaded; a segfault or hang inside a
  plugin still takes the app down.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Scan + rescan | `resonance-audio/src/engine/scan.rs` | `scan_plugins`, `rescan_plugins`, `ScannedPlugin` |
| Editor negotiation | `resonance-audio/src/clap_host/gui.rs` | `CLAP_WINDOW_API_NATIVE` |
| Settings view | `resonance-app/src/view/settings.rs` | Plugins section |
| Missing slots | `resonance-app/src/state/tracks.rs` (`PluginAvailability`), `view/missing_plugins_dialog.rs`, `view/mixer/inspector/chain.rs` (`missing_recovery`), `view/mixer/strip_parts.rs`, `update/plugin_replace.rs`, `update/project_io/serialize.rs` | |
| Plugin window | `resonance-app/src/view/plugin_window.rs` | generic params, Open/Close Editor |
| Control handlers | `resonance-control/src/methods/plugins.rs`; app `update/control/` | `CATALOG`, `RESCAN` |

## Surfaces

- **GUI:** Settings → Plugins; mixer inspector add pickers; command palette
  `+ plugin`; plugin window; missing-plugin dialog, strip pill and recovery
  row.
- **Control API / MCP:** `plugins.catalog` →
  `mcp__resonance__plugins_catalog` (entries id, name, kind, plus
  `scan_failures`; no vendor or path); `plugins.rescan` →
  `mcp__resonance__plugins_rescan`; adding through `track_add_instrument`,
  `track_add_effect`, `track_replace_effect`, `bus_add_effect`,
  `master_add_effect`. Missing slots are reported in plugin-target replies.
- **Agent skills:** `resonance-agent-plugin/skills/arranging` allows
  `plugins_catalog`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Rescan over the control API | `resonance-app/tests/control/control_plugins_rescan.rs` | `./scripts/run-tests.py -p resonance-app` |
| Settings button | `resonance-app/tests/plugins/settings_plugin_rescan_button.rs` | same |
| Missing slots and state preservation | `resonance-app/tests/plugins/{missing_plugin_slot,missing_plugin_state_preserved}.rs` | same |
| Generic panel, chain rows | `resonance-app/tests/mixer/{mixer_generic_param_panel,inspector_chain_row}.rs` | same |
| Engine scan, load failures, editor state, FFI | `resonance-audio/tests/clap_host/{plugin_rescan,plugin_load_failure,plugin_editor_state,plugin_id_duplicate_rejected,clap_ffi_hardening}.rs` | `./scripts/run-tests.py -p resonance-audio` |

Not covered: real third-party plugins (tests use first-party cdylibs and
fixtures); crash or hang isolation, because there is none.

## Decisions and deviations

Against the prototype: the generic parameter panel, the native editor
toggle, missing-plugin placeholders with preserved state and the control
surface are built; recovery actions live on the slot, not in the dialog,
on purpose. The scan-folder list and add-folder setting, the catalog table
with search and filters, scan progress and cancel, a picker modal with
source/vendor facets, the "3P" badge and the latency readout are not built.

## Known gaps

- [third-party-hosting-gaps](../../intents/plugins-instruments/third-party-hosting-gaps.md):
  catalog UI, user scan folders, scan progress/cancel, latency readout,
  X11-only editors, crash isolation.
