# Plugin parameters, state and presets (engine side)

> Verified against master @ `ef215b08` on 2026-10-15.
> Spec history: `docs/history/specs/plugin-preset-library.md` (§6–§8,
> slices P5, P7, P8), `docs/history/reviews/plugin-audit.md` (findings X8,
> F1, HOST-01), `docs/history/specs/nam-model-library.md` §9.2.

How the engine reads and writes a plugin's parameters, saves and restores
its state, and loads presets through the plugin. The preset *library*
(browsing, marks, search, the app's preset files) is in
[plugin-preset-library.md](../plugins-instruments/plugin-preset-library.md) and [preset-browser.md](../ui-shell/preset-browser.md); parameter automation lanes are in
[automation-lanes.md](../timeline/automation-lanes.md) and [channel-strips-and-inspector.md](../mixer/channel-strips-and-inspector.md).

## Behaviour

### Describing parameters

- On `AddPlugin` the engine queries every parameter into `ParamInfo`: id,
  name, range, default, current value, the plugin's own text for it, `unit`,
  `stepped`, `choices`, `module`, `hidden`, `automatable`.
- CLAP has no unit or enum list, so the host derives them from the plugin's
  `value_to_text`: `unit_from_text` takes `"dB"` off `"-6.0 dB"`;
  `choice_labels` walks a stepped parameter's steps (at most
  `MAX_CHOICE_STEPS` = 64) and keeps labels only when they say more than the
  number.

### Setting a parameter

- `SetPluginParam { instance_id, param_id, value }`: `try_lock` the instance
  (re-enqueue on a miss), queue the change, then call
  `clap_plugin_params.flush` at once so it lands even with the transport
  stopped (the stopped mixer runs no `process()`). If the plugin has no
  `flush`, the next `process()` applies it.
- The engine answers with `PluginParamText { value, text }`: how the plugin
  now formats the value. Automation moves parameters without this echo.
- `ResolvePluginParamText { token, … }` asks the plugin which value a text
  names (`text_to_value`, `ClapInstance::param_from_text`) and answers
  `PluginParamTextResolved { token, value }`. The control API uses it for
  labels like `"Low-pass"` or `"-6 dB"`.

### Parameters the plugin changes itself

- Output param events from `process()` or a flush are collected per
  gesture; a gesture is reported once, when it ends, as
  `PluginParamEdited { edit }`. The app records it as one undoable edit.
- `clap_host_params.rescan` with `VALUES` / `TEXT` → `PluginParamValuesChanged`
  (only the params that moved); a full rescan or a preset-state load →
  `PluginParamsRefreshed` (every param).
- `request_flush` is serviced from the engine loop's host-request poll.

### State

- `SavePluginState` → `PluginStateSaved { data }` (`clap.state.save`).
- `LoadPluginState` reloads with a deactivate / activate cycle around
  `load` so a plugin re-runs its own initialisation
  (`ClapInstance::reload_with_state`); a failure is an `Error` of kind
  `Plugin`.
- `SaveAllPluginStates` (project save): flush each instance first, report
  the values that moved since the host last read them, then save every
  state, so the saved file holds what the plugin plays. Instances whose
  lock is busy are retried.
- The app's mirror takes those values — and every other
  `PluginParamValuesChanged` / `PluginParamsRefreshed` — except for a param
  an enabled automation lane drives (`AutomationTarget::PluginParam`):
  a re-read finds it at its automated value under the playhead, which is
  not the param's own setting, so the mirror (and the file) keep the base
  value (`engine_events/plugins.rs` `automated_params`, BUG-38). With the
  lane's Read off, the value is taken as usual.
- The host's stream callbacks survive misbehaving plugins (null buffers,
  absurd sizes) and report `-1` per `stream.h`.

### Presets through the plugin

- `SavePluginPresetState` → `PluginPresetStateSaved { data, preset_form,
  first_party }`. With `clap.state-context` the plugin writes its preset
  form (`CLAP_STATE_CONTEXT_FOR_PRESET`); without it the full state is the
  preset (third-party tier T0).
- `LoadPluginPresetState { capture }` lays a preset over the current state.
  With a `capture` token the engine sends `PluginStateCaptured` twice
  (before and after), which is what undo and redo restore.
- `LoadPluginPresetFromLocation { location, load_key }` uses
  `clap.preset-load` (`from_location`, file or plugin-internal). The plugin
  confirms with `loaded()` → `PluginPresetLoaded`.
- `SetPluginPresetIgnoredParams` tells a first-party plugin which params the
  host automates, so they do not mark its preset modified
  (`com.resonance.preset-session`). First-party plugins also report
  `PluginPresetIdentity` (loaded preset, modified flag) and, for the drums,
  `PluginKitInfo`.

### Preset discovery

After each scan, one worker thread runs every bundle's
`clap.preset-discovery-factory`: providers are created, their declared
locations and file types copied and sealed, `PLUGIN` locations and the files
of `FILE` locations (depth-capped walk) indexed, and the providers
destroyed. No plugin instance is touched. Results are cached per bundle in
`<cache>/preset-discovery/<bundle>.json`, keyed on binary and file size +
mtime; a rescan re-indexes everything. Each plugin's list arrives as
`PluginPresetsDiscovered { plugin_id, presets }`. Shutdown waits up to 2 s
for the worker.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Param model | `resonance-audio/src/types/tempo/mod.rs` | `ParamInfo` (lives in the tempo module for historical reasons) |
| Meaning from text | `resonance-audio/src/clap_host/param_meta.rs` | `unit_from_text`, `choice_labels`, `label_round_trips`, `MAX_CHOICE_STEPS` |
| Flush, edits, rescans | `resonance-audio/src/clap_host/params.rs`, `clap_host/mod.rs` | `flush_pending_params`, `service_flush_request`, `sync_plugin_values`, `take_param_edits`, `host_params_rescan`, `host_params_request_flush` |
| State | `resonance-audio/src/clap_host/state.rs` | `save_state`, `load_state`, `reload_with_state` |
| Preset state + load | `resonance-audio/src/clap_host/preset_state.rs` | `save_preset_state`, `load_preset_state`, `load_preset_from_location`, `set_preset_ignored_params` |
| Discovery | `resonance-audio/src/clap_host/discovery.rs`, `engine/scan.rs` | `spawn_discovery_jobs`, `shutdown_discovery`, `DiscoveryFactory` |
| Handlers | `resonance-audio/src/engine/plugins.rs` | `handle_set_plugin_param`, `handle_resolve_param_text`, `handle_save_plugin_state`, `handle_load_plugin_state`, `handle_save_plugin_preset_state`, `handle_load_plugin_preset_state`, `handle_load_plugin_preset_from_location`, `handle_save_all_plugin_states`, `reload_plugin_state` |
| App mirror | `resonance-app/src/engine_events/plugins.rs`, `presets.rs` | — |

Invariant: every call into a plugin's params, state or preset extensions
happens with the instance mutex held, which is what keeps it from running
concurrently with `process()`.

## Surfaces

- **GUI:** the generic parameter panel and plugin editors; preset browser
  ([preset-browser.md](../ui-shell/preset-browser.md)).
- **Control API / MCP:** `track|bus|master.plugin_params` →
  `mcp__resonance__track_plugin_params` (and `bus_`, `master_`);
  `.set_plugin_param` → `mcp__resonance__track_set_plugin_param` (and `bus_`,
  `master_`); `.plugin_presets`, `.load_plugin_preset`,
  `.save_plugin_preset` → `mcp__resonance__track_plugin_presets`,
  `…_load_plugin_preset`, `…_save_plugin_preset`; `presets.*` tools for the
  library.
- **Agent skills:** `mixing`, `mastering`, `spatial`, `drumming`,
  `arranging` read and set plugin params; `mixing` and `spatial` load
  presets.

## How it's tested

| What | Where | Run |
|---|---|---|
| Flush, flags, meta, text | `resonance-audio/tests/clap_host/clap_param_flush.rs`, `clap_param_flags.rs`, `clap_param_meta.rs`, `param_from_text.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Live values at save | `tests/clap_host/plugin_live_values.rs` | same |
| State / presets / factory / discovery | `tests/clap_host/plugin_preset_state.rs`, `clap_factory_presets.rs`, `preset_discovery.rs`, `clap_ffi_hardening.rs` | same |
| Kit info | `tests/clap_host/drums_kit_info.rs` | same |
| Automated params keep their base through a save's rescan (BUG-38) | `resonance-app/tests/plugins/plugin_live_values.rs` | `./scripts/run-tests.py -p resonance-app` |
| App + control | `resonance-app/tests/plugins/plugin_edited_by_plugin.rs`, `plugin_live_values.rs`, `plugin_param_undo.rs`, `plugin_state_excluded_params.rs`; `tests/control/control_plugin_param_meta.rs`, `control_plugin_param_bounds.rs`, `control_plugin_params_persist.rs`, `control_plugin_presets.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered: third-party plugins' state-context and preset-load behaviour
(fakes and first-party plugins only).

## Decisions and deviations

- The host flushes on every `SetPluginParam`, not only when stopped; the
  flush drains the queue so a change is never applied twice.
- Parameter text is echoed per write instead of re-querying the whole
  parameter list (finding F1/X8).

## Known gaps

None known in the engine half.
