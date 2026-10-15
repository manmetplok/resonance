# Plugin SDK (`resonance-plugin`)

> Verified against master @ `5d6a2993` on 2026-10-15.
> Spec history: none as a single spec; pieces come from the plugin audit
> (`docs/history/reviews/plugin-audit.md`, `plugin-audit-plan.md`) and the
> code-review campaigns (`docs/history/reviews/code-review-todo.md`).

`resonance-plugin` is the thin framework every first-party CLAP plugin is
built on. A plugin implements one trait, `ResonancePlugin`, declares its
params as code, and calls `export_clap!`; the crate turns that into a CLAP
plugin over `clack-plugin` (it replaced nih-plug; pinned by commit in the
workspace `Cargo.toml`, clack 0.2 at `27ca2839`). It also holds what the
app and the plugins must agree on: the first-party CLAP ids, the private CLAP
extensions, the preset library and the shared editor plumbing.

## Behaviour

- **One trait.** `ResonancePlugin` (`resonance-plugin/src/plugin.rs`)
  carries identity constants (`CLAP_ID`, `NAME`, `VENDOR`, `VERSION`,
  `DESCRIPTION`, `FEATURES`), I/O shape (`INPUT_CHANNELS` — `None` means
  instrument —, `MIDI_INPUT`, `SIDECHAIN_INPUT` mono/stereo only,
  `output_layout()` for multi-out, up to 8 output ports), lifecycle
  (`new`, `initialize`, `reset`, `deactivate`), `process` /
  `process_with_key` (the bridge always calls the latter; the default drops
  the key), state hooks and optional sources (`extra_state_saver`,
  `param_text_source`, `kit_info_source`, `editor_factory`), plus
  `set_render_mode(offline)`, `latency_samples()` and `set_host()`.
- **Feature strings** are `&[&CStr]` from `resonance-plugin/src/features.rs`
  and reach the host verbatim; the bridge checks at compile time that at
  least one main category is declared.
- **Params** are code: `FloatParam`, `IntParam`, `BoolParam` behind the
  `Param` trait (`param.rs`), with `FloatRange`/`IntRange` mappings
  (`range.rs`, every knob moves through `normalize`/`denormalize`),
  formatters (`formatters.rs`) and per-sample `Smoother` (`smoother.rs`).
  CLAP param ids are `stable_hash` (FNV-1a) of the string id.
  Params can be flagged `state_excluded`; the host learns that through the
  `com.resonance.param-flags` extension.
- **State** is JSON: `{"version": 1, "params": {id: value}}` plus any
  top-level fields from `extra_state_saver()` (`state.rs`,
  `STATE_VERSION = 1`). Renamed params are declared in `param_renames()`
  (`ParamRename`); value changes that a rename cannot express go in the
  `STATE_UPGRADE` constant (idempotent, run on every load path, including
  presets).
- **`export_clap!(MyPlugin)`** (`lib.rs`) exports the CLAP entry and a
  second C symbol, `resonance_factory_presets`, that returns the plugin's
  `FACTORY_PRESETS` as a JSON array (or null). The host reads factory preset
  names from it without instantiating the plugin; third-party plugins lack
  it and are treated as having none.
- **CLAP extensions registered by the bridge** (`clap_bridge/mod.rs`):
  audio-ports, params, state, state-context, preset-load, note-ports (when
  `MIDI_INPUT`), latency, render, gui, and three Resonance-private ones:
  `com.resonance.preset-session` (loaded preset identity and modified flag,
  `clap_bridge/preset_session.rs`), `com.resonance.param-flags`
  (`clap_bridge/param_flags.rs`) and `com.resonance.kit-info` (the drum
  kit's pads as JSON, `clap_bridge/kit_info.rs`). Their ABIs live in
  `resonance-common` (`kit_info`, `param_flags`, `preset_session`).
- **Re-entrant host calls are served.** CLAP lets a host call back into the
  plugin from inside a host function the plugin called (`get_value` during
  `params.rescan()`, `latency.get()` during the `latency.changed()` made in
  `activate`, `gui.destroy()` during `gui.closed()`). clack 0.2 hands every
  main-thread call a `&ClapMainThread`, so the bridge keeps the plugin slot,
  the open editor and the preset-report bookkeeping in `Cell`/`RefCell`
  (`clap_bridge/shared.rs`: `plugin_ref`, `plugin_mut`, `with_editor`,
  `destroy_editor`) and holds no borrow across a call that can re-enter. A
  re-entrant reader that finds the plugin slot busy takes the active-plugin
  path (the shared mirror); a writer gets a `PluginError`; a `gui.destroy`
  that lands inside another editor call is carried out when that call
  returns. None of it is on the audio thread: `ClapAudioProcessor` owns its
  own plugin object and `process` touches no cell.
- **Host handle.** `HostHandle` (`host.rs`) is the plugin's only way to call
  the host: report latency (`set_latency_samples`), request a restart,
  process or main-thread callback, ask for a params or param-text rescan,
  and announce param changes the editor made (`announce_param_change` →
  CLAP output events `GESTURE_BEGIN`, `PARAM_VALUE`, `GESTURE_END`,
  `clap_bridge/param_output.rs`).
  `EditAnnouncer` lends it to editor widgets.
- **First-party identity.** `first_party.rs` holds every first-party CLAP id
  (`AMP` … `WAVETABLE`, `ALL` with 13 entries), `SECONDARY_KEY_PLUGINS`
  (reverb, EQ) and the param keys the app reads by name (for example
  `first_party::drums::{KIT_SELECT, KIT_LOAD_PROGRESS, OUTPUT_MODE}`).
  The app depends on no plugin crate, so this module is the contract.
- **Background loaders.** `loader.rs` has `Mailbox` (single-slot handoff
  from a loader thread to the audio thread) and `rescan_directory`, used by
  amp and IR.
- **Editor plumbing** (behind the `editor-widgets` feature):
  `editor_host.rs` picks the platform runtime as `RuntimeEditor`, gives
  `native_api()` (`"wayland"` or `"cocoa"`), wraps a running editor in
  `RuntimeEditorHandle` and lends the announcer to every frame through
  `AnnouncingApp`/`with_announcer`. `PluginEditor::set_size` returns
  `Result<(), SizeRefused>` (`gui.rs`): a refusal — `RuntimeEditorHandle`
  refuses once the window is gone or being torn down — reaches the host as
  `false` from `clap_plugin_gui.set_size`, with the editor's reason in
  `clap_host_log` (clack before 0.2 answered `true` regardless); `editor_widgets.rs` binds params to the
  pure widgets of `plugin-gui-core`; `preset_ui.rs`, `library_ui.rs` and
  `file_picker.rs` are the shared preset bar, library browser skin and
  non-blocking file picker (Linux; Cocoa calls `rfd` on the main thread).
  See [plugin-editor-runtimes.md](plugin-editor-runtimes.md).
- **Library row adapters.** `kit_rows.rs` and `nam_rows.rs` turn the drum-kit
  and NAM libraries into the shared `LibraryRows` browser model, so the
  plugin editors and the app's control methods filter the same way. See
  [plugin-preset-library.md](plugin-preset-library.md).

### The plugin pattern

ARCHITECTURE.md "Plugin Pattern" describes the crate shape (`lib.rs`,
`params.rs`, `dsp.rs` or `dsp/`, optional `presets.rs`, `viz.rs`, and a
feature-gated `editor/` whose `theme.rs` re-exports
`plugin_gui_core::theme::lavender::*`). All 13 plugins follow it; the larger
ones split `dsp` and `params` into directories. Every plugin's
`editor/theme.rs` is the one-line façade.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Trait + events | `resonance-plugin/src/plugin.rs` | `ResonancePlugin`, `NoteEvent`, `PluginEvent`, `OutputPortSpec`, `TempoInfo`, `beats_per_bar` |
| Export macro | `resonance-plugin/src/lib.rs` | `export_clap!`, `stable_hash`, `resonance_factory_presets` |
| CLAP bridge | `resonance-plugin/src/clap_bridge/` | `ClapBridge`, `ClapShared`, `ClapMainThread`, `ClapAudioProcessor`; `process.rs` (activation sync), `state.rs`, `params.rs`, `ports.rs`, `midi.rs`, `gui.rs` |
| Params | `param.rs`, `range.rs`, `smoother.rs`, `formatters.rs` | `FloatParam`, `IntParam`, `BoolParam`, `FloatRange`, `Smoother` |
| State | `state.rs` | `STATE_VERSION`, `ParamRename`, `StateUpgrade`, `wire_clap_id` |
| Identity | `first_party.rs`, `features.rs` | `first_party::ALL`, `first_party::drums` |
| Host handle | `host.rs` | `HostHandle`, `EditAnnouncer` |
| Editor glue | `editor_host.rs`, `editor_widgets.rs`, `gui.rs` | `RuntimeEditor`, `RuntimeEditorHandle`, `native_api`, `EditorFactory`, `PluginEditor` |

Invariants:

- `process()` must not allocate or lock; the bridge holds the plugin object
  inside the audio processor while active, so main-thread work goes through
  the cached sources (`ExtraStateSaver`, `ParamTextSource`,
  `KitInfoSource`), harvested once at creation.
- Port count must not change across activations (the host caches it).
- A first-party CLAP id is spelled only in `first_party.rs`
  (`tools/arch-invariants`, `first_party_clap_ids_are_spelled_once`).
- Plugins reach `resonance-common` only for the allow-listed utilities
  (`PLUGIN_COMMON_ITEMS` in `tools/arch-invariants`); today only amp, drums
  and IR depend on it.

## Surfaces

- **GUI:** none of its own; every plugin editor is built on it.
- **Control API / MCP:** indirect. The app reads params, state and the
  private extensions of first-party plugins through the CLAP host
  (`track_plugin_params`, `track_set_plugin_param`, preset tools).
- **Agent skills:** indirect; skill param keys are checked against real
  param tables (see [dsp-test-support.md](dsp-test-support.md)).

## How it's tested

| What | Where | Run |
|---|---|---|
| Params, ranges, smoothing, formatters | `resonance-plugin/tests/{param,range,smoother,formatters,typed_entry,param_widgets,knob_binding}.rs` | `./scripts/run-tests.py -p resonance-plugin` |
| Bridge through clack-host | `tests/clap_bridge_params_state.rs`, `process_abi.rs`, `state.rs`, `state_race.rs`, `midi_events.rs`, `midi_decode.rs`, `sidechain.rs`, `latency.rs`, `latency_runtime.rs`, `render_mode.rs`, `param_announce.rs`, `param_output_flags.rs`, `params_rescan.rs`, `host_handle_retire.rs`, `gui_closed.rs` (with its `reentrancy` module: a host calling back in from `rescan`, `latency.changed` and `gui.closed`; and `editor_set_size`: a refused size is `false` plus the reason in the host log), `features.rs` | same |
| Fleet rules | `tests/fleet_palette.rs`, `tests/fleet_preset_adoption.rs` (also pins every `CLAP_ID` to `first_party::ALL`) | same |
| Layering, id spelling | `tools/arch-invariants/tests/architecture.rs` | `cargo test -p arch-invariants` |
| Third-party host view | `resonance-audio/tests/clap_host/` loads built plugin cdylibs | `./scripts/run-tests.py -p resonance-audio` |

Not covered: the `fleet_*` tests list 11 plugins and pin that count; they do
not include `resonance-color` or `resonance-stereo`, and a new plugin is not
added to them automatically. `set_parent` (embedded editors) is a stub.

## Decisions and deviations

- Own framework over clack instead of nih-plug: smaller, and the private
  extensions (kit-info, preset-session, param-flags) need direct access.
- Factory presets go through an exported C symbol, not CLAP's
  `preset-discovery` factory: one symbol read from a library the host has
  open is enough for first-party banks.
- The doc comment on `FACTORY_PRESETS` says "four of the eleven plugins ship
  none"; there are 13 plugins and four (amp, drums, IR, mastering) ship none.

## Known gaps

- Embedded editors (`set_parent`) are not supported; see
  [macos-port-open-checks](../../intents/plugins-instruments/macos-port-open-checks.md).
- `clap_bridge/gui.rs` has no window retitle (`set_title` is a no-op).
